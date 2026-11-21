package com.pcremote.prs

import android.graphics.SurfaceTexture
import android.media.Image
import android.opengl.GLES11Ext
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLSurface
import android.opengl.GLES30
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Full colour (4:4:4) for a stream whose frames carry it packed into twice the picture's height
 * (the hosts' convert_ps / Y444 layout): luma rows [0,h) are Y, rows [h,2h) are Cb at full
 * resolution, and the chroma plane holds Cr for pixel 2x in U and 2x+1 in V. The decoder hands each
 * frame to [frame] from its own buffers; frames never go through an ImageReader, whose tiled
 * formats the CPU cannot read on some decoders (Qualcomm), and reading one aborts the app.
 *
 * Where the GPU reads raw YUV (GL_EXT_YUV_target: Adreno, Mali), the decoder renders straight into
 * [input], a texture: no copy on the CPU and no upload at all. Elsewhere frames come through [frame]
 * from the decoder's own buffers, with interleaved chroma uploaded as one two-channel texture.
 * Either way a first pass turns each picture pixel into RGB once, and a second pass scales that to
 * the view with the GPU's own filtering.
 */
class PackedRenderer(private val output: Surface, private val w: Int, private val h: Int, private val onError: (String) -> Unit) {
    private val thread = HandlerThread("pcr-full-colour", android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY).apply { start() }
    private val handler = Handler(thread.looper)

    private var display: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var context: EGLContext = EGL14.EGL_NO_CONTEXT
    private var surface: EGLSurface = EGL14.EGL_NO_SURFACE
    private var unpack = 0
    private var show = 0
    private var unpackMode = -1
    /** luma (R8, w x 2h), uv (RG8, w/2 x h), cb and cr (R8, w/2 x h, planar decoders), rgb (w x h). */
    private val textures = IntArray(5)
    private val frameBuffer = IntArray(1)
    private val luma = ByteBuffer.allocateDirect(w * h * 2)
    private val uv = ByteBuffer.allocateDirect(w * h)
    private val cb = ByteBuffer.allocateDirect((w / 2) * h)
    private val cr = ByteBuffer.allocateDirect((w / 2) * h)
    private var lastByte = ByteArray(1)
    /** How the chroma plane is laid out: [NV12], [NV21] or [PLANAR]. Decided from the first frames. */
    @Volatile private var mode = NV12
    private var modeKnown = false
    /** Guards the plane buffers between the decoder's thread and this one. */
    private val planes = Any()
    private val drawPosted = java.util.concurrent.atomic.AtomicBoolean(false)
    @Volatile private var hasFrame = false
    @Volatile private var failed = false
    /** Frames drawn, for tests. */
    @Volatile var drawn = 0; private set
    /** The decoder renders here directly when this GPU reads raw YUV; null means use [frame]. */
    @Volatile var input: Surface? = null; private set
    private var direct: SurfaceTexture? = null
    private var directProgram = 0
    private var directTexture = 0

    init {
        // Ready before the decoder is configured: it needs to know where to render.
        val ready = CountDownLatch(1)
        handler.post {
            runCatching { setUp() }.onFailure { fail("Full colour could not start here: ${it.message}") }
            if (!failed) runCatching { setUpDirect() }.onFailure { Log.i(TAG, "frames come through the CPU: ${it.message}") }
            ready.countDown()
        }
        ready.await(2, TimeUnit.SECONDS)
    }

    /** A texture the decoder renders into, read raw by the YUV sampler: no copies at all. */
    private fun setUpDirect() {
        val extensions = GLES30.glGetString(GLES30.GL_EXTENSIONS) ?: ""
        check("GL_EXT_YUV_target" in extensions) { "no GL_EXT_YUV_target" }
        directProgram = link(VERTEX, DIRECT)
        val t = IntArray(1)
        GLES30.glGenTextures(1, t, 0)
        directTexture = t[0]
        GLES30.glActiveTexture(GLES30.GL_TEXTURE5)
        GLES30.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, directTexture)
        for (p in intArrayOf(GLES30.GL_TEXTURE_MIN_FILTER, GLES30.GL_TEXTURE_MAG_FILTER)) GLES30.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, p, GLES30.GL_NEAREST)
        GLES30.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES30.GL_TEXTURE_WRAP_S, GLES30.GL_CLAMP_TO_EDGE)
        GLES30.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES30.GL_TEXTURE_WRAP_T, GLES30.GL_CLAMP_TO_EDGE)
        GLES30.glUseProgram(directProgram)
        GLES30.glUniform1i(GLES30.glGetUniformLocation(directProgram, "frame"), 5)
        GLES30.glUniform1i(GLES30.glGetUniformLocation(directProgram, "h"), h)
        val st = SurfaceTexture(directTexture)
        st.setDefaultBufferSize(w, h * 2)
        st.setOnFrameAvailableListener({ drawDirect() }, handler)
        direct = st
        input = Surface(st)
    }

    private fun drawDirect() {
        if (failed || display == EGL14.EGL_NO_DISPLAY) return
        runCatching {
            direct?.updateTexImage()
            GLES30.glUseProgram(directProgram)
            GLES30.glActiveTexture(GLES30.GL_TEXTURE5)
            GLES30.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, directTexture)
            unpackOnce(directProgram)
        }.onFailure { fail("Full colour stopped: ${it.message}") }
    }

    /**
     * Takes one decoded frame (an image from MediaCodec.getOutputImage). The planes are copied before
     * this returns, so the caller may release the decoder's buffer straight away.
     */
    fun frame(image: Image) {
        if (failed || image.height < h * 2 || image.width < w) return
        runCatching {
            val p = image.planes
            synchronized(planes) {
                rows(p[0].buffer, p[0].rowStride, w, h * 2, luma)
                if (p[1].pixelStride == 2 && p[2].pixelStride == 2) {
                    if (!modeKnown) decideOrder(p[1].buffer, p[2].buffer)
                    // The plane that starts first holds the pairs in memory order.
                    val (first, second) = if (mode == NV21) p[2] to p[1] else p[1] to p[2]
                    rows(first.buffer, first.rowStride, w, h, uv, second.buffer)
                } else {
                    mode = PLANAR
                    rows(p[1].buffer, p[1].rowStride, w / 2, h, cb)
                    rows(p[2].buffer, p[2].rowStride, w / 2, h, cr)
                }
                hasFrame = true
            }
        }.onFailure { fail("Full colour stopped: ${it.message}"); return }
        if (drawPosted.compareAndSet(false, true)) handler.post {
            drawPosted.set(false)
            if (!failed) runCatching { draw() }.onFailure { fail("Full colour stopped: ${it.message}") }
        }
    }

    /**
     * Semi-planar chroma is one block of pairs; Image gives it as two views one byte apart. In NV12 the
     * V view reads the U view shifted by one, in NV21 the other way round. A row where both hold (one
     * flat colour) decides nothing; the next frame is checked instead.
     */
    private fun decideOrder(u: ByteBuffer, v: ByteBuffer) {
        val n = minOf(w - 2, u.remaining() - 1, v.remaining() - 1)
        var nv12 = true
        var nv21 = true
        for (i in 0 until n) {
            if (v.get(v.position() + i) != u.get(u.position() + i + 1)) nv12 = false
            if (u.get(u.position() + i) != v.get(v.position() + i + 1)) nv21 = false
            if (!nv12 && !nv21) break
        }
        if (nv12 != nv21) { mode = if (nv12) NV12 else NV21; modeKnown = true }
    }

    /**
     * Copies [height] rows of [width] bytes from a plane into [out], packed tight. An interleaved
     * plane's view ends one byte short of its last pair; that byte comes from [tail], the other view.
     */
    private fun rows(src: ByteBuffer, stride: Int, width: Int, height: Int, out: ByteBuffer, tail: ByteBuffer? = null) {
        val b = src.duplicate()
        val base = b.position()
        out.clear()
        if (stride == width && b.remaining() >= width * height) {
            b.limit(base + width * height)
            out.put(b)
        } else for (y in 0 until height) {
            val start = base + y * stride
            val end = minOf(start + width, b.capacity())
            b.limit(end).position(start)
            out.put(b)
            b.limit(b.capacity())
        }
        if (out.position() < width * height && tail != null) {
            // The missing byte is the second of the last pair: the other view's last byte.
            val t = tail.duplicate()
            lastByte[0] = t.get(t.limit() - 1)
            out.put(lastByte, 0, 1)
        }
        out.flip()
    }

    private fun fail(why: String) {
        if (failed) return
        failed = true
        Log.w(TAG, why)
        onError(why)
    }

    private fun setUp() {
        display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
        val version = IntArray(2)
        check(EGL14.eglInitialize(display, version, 0, version, 1)) { "no EGL" }
        val attributes = intArrayOf(EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8,
            EGL14.EGL_RENDERABLE_TYPE, 0x40 /* EGL_OPENGL_ES3_BIT */, EGL14.EGL_SURFACE_TYPE, EGL14.EGL_WINDOW_BIT, EGL14.EGL_NONE)
        val configs = arrayOfNulls<EGLConfig>(1)
        val count = IntArray(1)
        check(EGL14.eglChooseConfig(display, attributes, 0, configs, 0, 1, count, 0) && count[0] > 0) { "no OpenGL ES 3 configuration" }
        context = EGL14.eglCreateContext(display, configs[0], EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 3, EGL14.EGL_NONE), 0)
        check(context != EGL14.EGL_NO_CONTEXT) { "no OpenGL ES 3 context" }
        surface = EGL14.eglCreateWindowSurface(display, configs[0], output, intArrayOf(EGL14.EGL_NONE), 0)
        check(surface != EGL14.EGL_NO_SURFACE) { "the view is busy" }
        check(EGL14.eglMakeCurrent(display, surface, surface, context)) { "could not use the view" }
        // The decoder already paces frames to the display; waiting for vsync here again would add a frame of delay.
        EGL14.eglSwapInterval(display, 0)
        unpack = link(VERTEX, UNPACK)
        show = link(VERTEX, SHOW)
        // The triangle comes from gl_VertexID alone, but some drivers still need a vertex array bound.
        val vao = IntArray(1)
        GLES30.glGenVertexArrays(1, vao, 0)
        GLES30.glBindVertexArray(vao[0])
        GLES30.glGenTextures(5, textures, 0)
        val formats = arrayOf(
            Triple(GLES30.GL_R8, w, h * 2), Triple(GLES30.GL_RG8, w / 2, h),
            Triple(GLES30.GL_R8, w / 2, h), Triple(GLES30.GL_R8, w / 2, h), Triple(GLES30.GL_RGBA8, w, h))
        for (i in 0..4) {
            GLES30.glActiveTexture(GLES30.GL_TEXTURE0 + i)
            GLES30.glBindTexture(GLES30.GL_TEXTURE_2D, textures[i])
            val filter = if (i == 4) GLES30.GL_LINEAR else GLES30.GL_NEAREST
            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_MIN_FILTER, filter)
            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_MAG_FILTER, filter)
            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_WRAP_S, GLES30.GL_CLAMP_TO_EDGE)
            GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, GLES30.GL_TEXTURE_WRAP_T, GLES30.GL_CLAMP_TO_EDGE)
            GLES30.glTexStorage2D(GLES30.GL_TEXTURE_2D, 1, formats[i].first, formats[i].second, formats[i].third)
        }
        GLES30.glGenFramebuffers(1, frameBuffer, 0)
        GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, frameBuffer[0])
        GLES30.glFramebufferTexture2D(GLES30.GL_FRAMEBUFFER, GLES30.GL_COLOR_ATTACHMENT0, GLES30.GL_TEXTURE_2D, textures[4], 0)
        check(GLES30.glCheckFramebufferStatus(GLES30.GL_FRAMEBUFFER) == GLES30.GL_FRAMEBUFFER_COMPLETE) { "no picture buffer" }
        GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, 0)
        GLES30.glUseProgram(unpack)
        for ((i, name) in listOf("luma", "uv", "cb", "cr").withIndex()) GLES30.glUniform1i(GLES30.glGetUniformLocation(unpack, name), i)
        GLES30.glUniform1i(GLES30.glGetUniformLocation(unpack, "h"), h)
        unpackMode = GLES30.glGetUniformLocation(unpack, "mode")
        GLES30.glUseProgram(show)
        GLES30.glUniform1i(GLES30.glGetUniformLocation(show, "rgb"), 4)
        GLES30.glPixelStorei(GLES30.GL_UNPACK_ALIGNMENT, 1)
    }

    private fun upload(unit: Int, width: Int, height: Int, format: Int, data: ByteBuffer) {
        data.position(0)
        GLES30.glActiveTexture(GLES30.GL_TEXTURE0 + unit)
        GLES30.glBindTexture(GLES30.GL_TEXTURE_2D, textures[unit])
        GLES30.glTexSubImage2D(GLES30.GL_TEXTURE_2D, 0, 0, 0, width, height, format, GLES30.GL_UNSIGNED_BYTE, data)
    }

    private fun draw() {
        if (display == EGL14.EGL_NO_DISPLAY || !hasFrame) return
        val m: Int
        synchronized(planes) {
            m = mode
            upload(0, w, h * 2, GLES30.GL_RED, luma)
            if (m == PLANAR) {
                upload(2, w / 2, h, GLES30.GL_RED, cb)
                upload(3, w / 2, h, GLES30.GL_RED, cr)
            } else upload(1, w / 2, h, GLES30.GL_RG, uv)
        }
        // Pass 1: each picture pixel to RGB, once.
        GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, frameBuffer[0])
        GLES30.glViewport(0, 0, w, h)
        GLES30.glUseProgram(unpack)
        GLES30.glUniform1i(unpackMode, m)
        unpackOnce(unpack)
    }

    /**
     * Runs the bound unpacking program. When the view is exactly the picture's size (1080p on a
     * 1080p TV) it writes straight to the view: one full-screen pass instead of two, which a
     * Chromecast's GPU needs to keep up at 60 frames a second. Otherwise it fills the picture
     * texture and [present] scales that to the view.
     */
    private fun unpackOnce(program: Int) {
        val size = IntArray(2)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_WIDTH, size, 0)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_HEIGHT, size, 1)
        val exact = size[0] == w && size[1] == h
        GLES30.glUniform1i(GLES30.glGetUniformLocation(program, "flip"), if (exact) 1 else 0)
        if (exact) {
            GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, 0)
            GLES30.glViewport(0, 0, w, h)
            GLES30.glDrawArrays(GLES30.GL_TRIANGLES, 0, 3)
            EGL14.eglSwapBuffers(display, surface)
            drawn++
        } else {
            GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, frameBuffer[0])
            GLES30.glViewport(0, 0, w, h)
            GLES30.glDrawArrays(GLES30.GL_TRIANGLES, 0, 3)
            present()
        }
    }

    /** Pass 2: the picture scaled to the view by the GPU's filtering. */
    private fun present() {
        GLES30.glBindFramebuffer(GLES30.GL_FRAMEBUFFER, 0)
        val size = IntArray(2)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_WIDTH, size, 0)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_HEIGHT, size, 1)
        GLES30.glViewport(0, 0, size[0], size[1])
        GLES30.glUseProgram(show)
        GLES30.glDrawArrays(GLES30.GL_TRIANGLES, 0, 3)
        EGL14.eglSwapBuffers(display, surface)
        drawn++
    }

    /** Frees the view for the next renderer or decoder; waits briefly so the view is free on return. */
    fun release() {
        val done = CountDownLatch(1)
        failed = true
        handler.post {
            direct?.release(); direct = null
            input?.release(); input = null
            if (display != EGL14.EGL_NO_DISPLAY) {
                EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
                if (surface != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, surface)
                if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display, context)
                EGL14.eglTerminate(display)
                display = EGL14.EGL_NO_DISPLAY
            }
            done.countDown()
        }
        done.await(500, TimeUnit.MILLISECONDS)
        thread.quitSafely()
    }

    private fun link(vertex: String, fragment: String): Int {
        fun compile(type: Int, source: String): Int {
            val s = GLES30.glCreateShader(type)
            GLES30.glShaderSource(s, source)
            GLES30.glCompileShader(s)
            val ok = IntArray(1)
            GLES30.glGetShaderiv(s, GLES30.GL_COMPILE_STATUS, ok, 0)
            check(ok[0] != 0) { GLES30.glGetShaderInfoLog(s) }
            return s
        }
        val p = GLES30.glCreateProgram()
        GLES30.glAttachShader(p, compile(GLES30.GL_VERTEX_SHADER, vertex))
        GLES30.glAttachShader(p, compile(GLES30.GL_FRAGMENT_SHADER, fragment))
        GLES30.glLinkProgram(p)
        val ok = IntArray(1)
        GLES30.glGetProgramiv(p, GLES30.GL_LINK_STATUS, ok, 0)
        check(ok[0] != 0) { GLES30.glGetProgramInfoLog(p) }
        return p
    }

    private companion object {
        const val TAG = "PcrFullColour"
        const val NV12 = 0
        const val NV21 = 1
        const val PLANAR = 2
        const val VERTEX = """#version 300 es
out vec2 coordinate;
void main() {
    vec2 corner = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);
    coordinate = vec2((corner.x + 1.0) * 0.5, 1.0 - (corner.y + 1.0) * 0.5);
    gl_Position = vec4(corner, 0.0, 1.0);
}"""
        /** One picture pixel: row y of the RGB texture is picture row y. Limited-range BT.709, as the hosts encode. */
        const val UNPACK = """#version 300 es
precision highp float;
precision highp int;
uniform sampler2D luma;
uniform sampler2D uv;
uniform sampler2D cb;
uniform sampler2D cr;
uniform int h;
uniform int mode;
uniform int flip;
out vec4 colour;
void main() {
    ivec2 p = ivec2(gl_FragCoord.xy);
    if (flip == 1) p.y = h - 1 - p.y;
    float y = texelFetch(luma, p, 0).r;
    float b = texelFetch(luma, ivec2(p.x, p.y + h), 0).r;
    ivec2 c = ivec2(p.x / 2, p.y);
    float r;
    if (mode == 2) {
        r = (p.x % 2 == 0) ? texelFetch(cb, c, 0).r : texelFetch(cr, c, 0).r;
    } else {
        vec2 pair = texelFetch(uv, c, 0).rg;
        if (mode == 1) pair = pair.yx;
        r = (p.x % 2 == 0) ? pair.x : pair.y;
    }
    float l = 1.164383 * (y - 0.0627451);
    vec2 d = vec2(b, r) - vec2(0.5019608);
    colour = vec4(l + 1.792741 * d.y, l - 0.213249 * d.x - 0.532909 * d.y, l + 2.112402 * d.x, 1.0);
}"""
        /**
         * The same unpacking from the decoder's own frame, read raw by the YUV sampler. Chroma row y
         * sits with luma row 2y; at luma column 2k the sampler gives that row's chroma pair k.
         */
        const val DIRECT = """#version 300 es
#extension GL_EXT_YUV_target : require
precision highp float;
precision highp int;
uniform __samplerExternal2DY2YEXT frame;
uniform int h;
uniform int flip;
out vec4 colour;
void main() {
    ivec2 p = ivec2(gl_FragCoord.xy);
    if (flip == 1) p.y = h - 1 - p.y;
    float y = texelFetch(frame, p, 0).r;
    float b = texelFetch(frame, ivec2(p.x, p.y + h), 0).r;
    vec3 c = texelFetch(frame, ivec2(p.x - p.x % 2, 2 * p.y), 0).rgb;
    float r = (p.x % 2 == 0) ? c.g : c.b;
    float l = 1.164383 * (y - 0.0627451);
    vec2 d = vec2(b, r) - vec2(0.5019608);
    colour = vec4(l + 1.792741 * d.y, l - 0.213249 * d.x - 0.532909 * d.y, l + 2.112402 * d.x, 1.0);
}"""
        const val SHOW = """#version 300 es
precision mediump float;
uniform sampler2D rgb;
in vec2 coordinate;
out vec4 colour;
void main() {
    colour = texture(rgb, coordinate);
}"""
    }
}

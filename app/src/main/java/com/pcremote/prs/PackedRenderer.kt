package com.pcremote.prs

import android.graphics.ImageFormat
import android.media.Image
import android.media.ImageReader
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
 * resolution, and the two chroma planes hold Cr for pixels 2x and 2x+1. The decoder writes into
 * [input]; each frame is unpacked by a shader and drawn onto [output], the stream's view.
 */
class PackedRenderer(private val output: Surface, private val w: Int, private val h: Int, private val onError: (String) -> Unit) {
    private val thread = HandlerThread("pcr-full-colour", android.os.Process.THREAD_PRIORITY_URGENT_DISPLAY).apply { start() }
    private val handler = Handler(thread.looper)
    private val reader = ImageReader.newInstance(w, h * 2, ImageFormat.YUV_420_888, 3)
    /** Where the decoder renders. */
    val input: Surface get() = reader.surface

    private var display: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var context: EGLContext = EGL14.EGL_NO_CONTEXT
    private var surface: EGLSurface = EGL14.EGL_NO_SURFACE
    private var program = 0
    private val textures = IntArray(3)
    private val luma = ByteBuffer.allocateDirect(w * h * 2)
    private val cb = ByteBuffer.allocateDirect((w / 2) * h)
    private val cr = ByteBuffer.allocateDirect((w / 2) * h)
    @Volatile private var failed = false
    /** Frames drawn, for tests. */
    @Volatile var drawn = 0; private set

    init {
        handler.post { runCatching { setUp() }.onFailure { fail("Full colour could not start here: ${it.message}") } }
        reader.setOnImageAvailableListener({ r -> r.acquireLatestImage()?.use { if (!failed) runCatching { draw(it) }.onFailure { e -> fail("Full colour stopped: ${e.message}") } } }, handler)
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
        program = link(VERTEX, FRAGMENT)
        GLES30.glUseProgram(program)
        // The triangle comes from gl_VertexID alone, but some drivers still need a vertex array bound.
        val vao = IntArray(1)
        GLES30.glGenVertexArrays(1, vao, 0)
        GLES30.glBindVertexArray(vao[0])
        GLES30.glGenTextures(3, textures, 0)
        val sizes = arrayOf(w to h * 2, w / 2 to h, w / 2 to h)
        for (i in 0..2) {
            GLES30.glActiveTexture(GLES30.GL_TEXTURE0 + i)
            GLES30.glBindTexture(GLES30.GL_TEXTURE_2D, textures[i])
            for (p in intArrayOf(GLES30.GL_TEXTURE_MIN_FILTER, GLES30.GL_TEXTURE_MAG_FILTER)) GLES30.glTexParameteri(GLES30.GL_TEXTURE_2D, p, GLES30.GL_NEAREST)
            GLES30.glTexStorage2D(GLES30.GL_TEXTURE_2D, 1, GLES30.GL_R8, sizes[i].first, sizes[i].second)
            GLES30.glUniform1i(GLES30.glGetUniformLocation(program, arrayOf("luma", "cb", "cr")[i]), i)
        }
        GLES30.glUniform2i(GLES30.glGetUniformLocation(program, "shown"), w, h)
        GLES30.glPixelStorei(GLES30.GL_UNPACK_ALIGNMENT, 1)
    }

    /** Uploads a plane. Rows with padding are packed tight first: some drivers mishandle GL_UNPACK_ROW_LENGTH. */
    private fun upload(unit: Int, width: Int, height: Int, plane: Image.Plane, tight: ByteBuffer) {
        val src = plane.buffer
        val data = if (plane.pixelStride == 1 && plane.rowStride == width) src else {
            tight.clear()
            for (y in 0 until height) {
                val row = y * plane.rowStride
                if (plane.pixelStride == 1) {
                    val view = src.duplicate()
                    view.position(row).limit(row + width)
                    tight.put(view)
                } else for (x in 0 until width) tight.put(src.get(row + x * plane.pixelStride))
            }
            tight.flip()
            tight
        }
        GLES30.glActiveTexture(GLES30.GL_TEXTURE0 + unit)
        GLES30.glBindTexture(GLES30.GL_TEXTURE_2D, textures[unit])
        GLES30.glTexSubImage2D(GLES30.GL_TEXTURE_2D, 0, 0, 0, width, height, GLES30.GL_RED, GLES30.GL_UNSIGNED_BYTE, data)
    }

    private fun draw(image: Image) {
        if (display == EGL14.EGL_NO_DISPLAY || image.height < h * 2) return
        val planes = image.planes
        upload(0, w, h * 2, planes[0], luma)
        upload(1, w / 2, h, planes[1], cb)
        upload(2, w / 2, h, planes[2], cr)
        val size = IntArray(2)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_WIDTH, size, 0)
        EGL14.eglQuerySurface(display, surface, EGL14.EGL_HEIGHT, size, 1)
        GLES30.glViewport(0, 0, size[0], size[1])
        GLES30.glDrawArrays(GLES30.GL_TRIANGLES, 0, 3)
        EGL14.eglSwapBuffers(display, surface)
        drawn++
    }

    /** Frees the view for the next renderer or decoder; waits briefly so the view is free on return. */
    fun release() {
        val done = CountDownLatch(1)
        handler.post {
            reader.setOnImageAvailableListener(null, null)
            if (display != EGL14.EGL_NO_DISPLAY) {
                EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
                if (surface != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, surface)
                if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display, context)
                EGL14.eglTerminate(display)
                display = EGL14.EGL_NO_DISPLAY
            }
            reader.close()
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
        const val VERTEX = """#version 300 es
out vec2 coordinate;
void main() {
    vec2 corner = vec2(gl_VertexID == 1 ? 3.0 : -1.0, gl_VertexID == 2 ? 3.0 : -1.0);
    coordinate = vec2((corner.x + 1.0) * 0.5, 1.0 - (corner.y + 1.0) * 0.5);
    gl_Position = vec4(corner, 0.0, 1.0);
}"""
        // Limited-range BT.709, as the hosts encode. Bilinear by hand: the planes are not filterable as one image.
        const val FRAGMENT = """#version 300 es
precision highp float;
precision highp int;
uniform sampler2D luma;
uniform sampler2D cb;
uniform sampler2D cr;
uniform ivec2 shown;
in vec2 coordinate;
out vec4 colour;
vec3 full(ivec2 p) {
    p = clamp(p, ivec2(0), shown - ivec2(1));
    ivec2 c = ivec2(p.x / 2, p.y);
    float v = (p.x % 2 == 0) ? texelFetch(cb, c, 0).r : texelFetch(cr, c, 0).r;
    return vec3(texelFetch(luma, p, 0).r, texelFetch(luma, ivec2(p.x, p.y + shown.y), 0).r, v);
}
void main() {
    vec2 f = coordinate * vec2(shown) - 0.5;
    ivec2 i = ivec2(floor(f));
    vec2 t = f - floor(f);
    vec3 s = mix(mix(full(i), full(i + ivec2(1, 0)), t.x), mix(full(i + ivec2(0, 1)), full(i + ivec2(1, 1)), t.x), t.y);
    float y = 1.164383 * (s.x - 0.0627451);
    vec2 c = s.yz - vec2(0.5019608);
    colour = vec4(y + 1.792741 * c.y, y - 0.213249 * c.x - 0.532909 * c.y, y + 2.112402 * c.x, 1.0);
}"""
    }
}

//! Draws NV12 pictures with a small shader inside eframe's own OpenGL context, so
//! colour conversion and scaling run on the graphics card. No extra dependency.
use crate::picture::Picture;
use eframe::{
    egui,
    egui_glow::{self, ShaderVersion},
    glow::{self, HasContext},
};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub type Handle = Arc<Mutex<Shared>>;

/// The shader works in this window.
static SHADER: AtomicBool = AtomicBool::new(false);
/// Drawing decoded frames straight from Direct3D textures: 0 not tried yet, 1 works, 2 does not.
static INTEROP: AtomicU8 = AtomicU8::new(0);

/// Whether the decoder may leave pictures on the graphics card (video/dxgl.rs).
pub fn interop_usable() -> bool {
    SHADER.load(Ordering::Relaxed) && INTEROP.load(Ordering::Relaxed) != 2
}

#[derive(Default)]
pub struct Shared {
    pending: Option<Picture>,
    gl: Option<Resources>,
    failed: bool,
}

struct Resources {
    program: glow::Program,
    packed: Option<glow::UniformLocation>,
    shown: Option<glow::UniformLocation>,
    full_colour: bool,
    vao: glow::VertexArray,
    luma: glow::Texture,
    chroma: glow::Texture,
    size: [usize; 2],
    /// The frame on screen when it came straight from the decoder; held so it is not overwritten.
    #[cfg(windows)]
    shown_gpu: Option<crate::video::dxgl::GpuFrame>,
    #[cfg(windows)]
    interop: Option<interop::Interop>,
}

/// None when the context is too old for the shader; the caller then converts on the CPU.
pub fn new(gl: &Arc<glow::Context>) -> Option<Handle> {
    let ok = ShaderVersion::get(gl).is_new_shader_interface();
    SHADER.store(ok, Ordering::Relaxed);
    ok.then(Handle::default)
}

pub fn submit(handle: &Handle, picture: Picture) {
    handle.lock().unwrap().pending = Some(picture);
}

pub fn failed(handle: &Handle) -> bool {
    handle.lock().unwrap().failed
}

pub fn callback(handle: Handle, rect: egui::Rect) -> egui::PaintCallback {
    egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
            let mut shared = handle.lock().unwrap();
            if shared.failed {
                return;
            }
            if let Err(error) = draw(&mut shared, painter.gl()) {
                // Logged once: the picture is then converted on the CPU, which costs far more.
                eprintln!("secpar: video shader failed, using the CPU instead: {error}");
                shared.failed = true;
                SHADER.store(false, Ordering::Relaxed);
            }

        })),
    }
}

const VERTEX: &str = "
const vec2 corners[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
out vec2 coordinate;
void main() {
    vec2 corner = corners[gl_VertexID];
    coordinate = vec2((corner.x + 1.0) * 0.5, 1.0 - (corner.y + 1.0) * 0.5);
    gl_Position = vec4(corner, 0.0, 1.0);
}";

// Limited-range BT.709, the matrix the Windows host encodes with.
const FRAGMENT: &str = "
#ifdef GL_ES
precision mediump float;
#endif
uniform sampler2D luma;
uniform sampler2D chroma;
uniform int full_colour;
uniform ivec2 shown;
in vec2 coordinate;
out vec4 colour;
// One pixel of a packed 4:4:4 frame (see picture.rs): Y, Cb, Cr.
vec3 full(ivec2 p) {
    p = clamp(p, ivec2(0), shown - ivec2(1));
    vec2 cr = texelFetch(chroma, ivec2(p.x / 2, p.y), 0).rg;
    return vec3(texelFetch(luma, p, 0).r, texelFetch(luma, ivec2(p.x, p.y + shown.y), 0).r, (p.x % 2 == 0) ? cr.r : cr.g);
}
void main() {
    vec3 s;
    if (full_colour == 1) {
        // Bilinear by hand: the planes cannot be filtered as textures.
        vec2 f = coordinate * vec2(shown) - 0.5;
        ivec2 i = ivec2(floor(f));
        vec2 t = f - floor(f);
        s = mix(mix(full(i), full(i + ivec2(1, 0)), t.x), mix(full(i + ivec2(0, 1)), full(i + ivec2(1, 1)), t.x), t.y);
    } else {
        s = vec3(texture(luma, coordinate).r, texture(chroma, coordinate).rg);
    }
    float y = 1.164383 * (s.x - 0.0627451);
    vec2 c = s.yz - vec2(0.5019608);
    colour = vec4(
        y + 1.792741 * c.y,
        y - 0.213249 * c.x - 0.532909 * c.y,
        y + 2.112402 * c.x,
        1.0);
}";

fn draw(shared: &mut Shared, gl: &Arc<glow::Context>) -> Result<(), String> {
    let version = ShaderVersion::get(gl);
    // SAFETY: plain OpenGL calls on the context eframe made current for this callback.
    unsafe {
        if shared.gl.is_none() {
            shared.gl = Some(create(gl, version)?);
        }
        let resources = shared.gl.as_mut().unwrap();
        if let Some(picture) = shared.pending.take() {
            let began=secpar_stream::timing::begin();
            #[cfg(windows)]
            if let Some(frame) = picture.gpu.clone() {
                // Registered once per ring; a failure means the decoder copies frames back from now on.
                if interop::prepare(gl, resources, &frame).is_err() {
                    INTEROP.store(2, Ordering::Relaxed);
                    eprintln!("secpar: OpenGL cannot read Direct3D textures here; frames are copied back");
                    return Ok(());
                }
                INTEROP.store(1, Ordering::Relaxed);
                resources.shown_gpu = Some(frame);
                resources.size = picture.size;
                resources.full_colour = picture.packed;
            } else {
                resources.shown_gpu = None;
                upload(gl, resources, &picture);
            }
            #[cfg(not(windows))]
            upload(gl, resources, &picture);
            secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Upload,began);
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Draw);
        }
        if resources.size == [0, 0] {
            return Ok(());
        }
        gl.disable(glow::BLEND);
        gl.disable(glow::DEPTH_TEST);
        gl.use_program(Some(resources.program));
        gl.uniform_1_i32(resources.packed.as_ref(), resources.full_colour as i32);
        let [w, h] = resources.size;
        gl.uniform_2_i32(resources.shown.as_ref(), w as i32, if resources.full_colour { h as i32 / 2 } else { h as i32 });
        #[cfg(windows)]
        let locked = match (&resources.shown_gpu, &resources.interop) {
            (Some(frame), Some(interop)) => Some(interop.lock(frame.index).ok_or("could not lock the decoded frame")?),
            _ => None,
        };
        #[cfg(not(windows))]
        let locked: Option<(glow::Texture, glow::Texture)> = None;
        let (luma, chroma) = match &locked {
            #[cfg(windows)]
            Some(l) => (l.luma, l.chroma),
            #[cfg(not(windows))]
            Some(l) => *l,
            None => (resources.luma, resources.chroma),
        };
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(luma));
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(chroma));
        gl.bind_vertex_array(Some(resources.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);
        drop(locked);
    }
    Ok(())
}

unsafe fn create(gl: &glow::Context, version: ShaderVersion) -> Result<Resources, String> {
    let declaration = version.version_declaration();
    let compile = |kind, body: &str| -> Result<glow::Shader, String> {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, &format!("{declaration}\n{body}"));
        gl.compile_shader(shader);
        if gl.get_shader_compile_status(shader) {
            Ok(shader)
        } else {
            Err(gl.get_shader_info_log(shader))
        }
    };
    let vertex = compile(glow::VERTEX_SHADER, VERTEX)?;
    let fragment = compile(glow::FRAGMENT_SHADER, FRAGMENT)?;
    let program = gl.create_program()?;
    gl.attach_shader(program, vertex);
    gl.attach_shader(program, fragment);
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(gl.get_program_info_log(program));
    }
    gl.delete_shader(vertex);
    gl.delete_shader(fragment);
    gl.use_program(Some(program));
    gl.uniform_1_i32(gl.get_uniform_location(program, "luma").as_ref(), 0);
    gl.uniform_1_i32(gl.get_uniform_location(program, "chroma").as_ref(), 1);
    let texture = || filtered_texture(gl);
    Ok(Resources {
        packed: gl.get_uniform_location(program, "full_colour"),
        shown: gl.get_uniform_location(program, "shown"),
        full_colour: false,
        program,
        vao: gl.create_vertex_array()?,
        luma: texture()?,
        chroma: texture()?,
        size: [0, 0],
        #[cfg(windows)]
        shown_gpu: None,
        #[cfg(windows)]
        interop: None,
    })
}

unsafe fn filtered_texture(gl: &glow::Context) -> Result<glow::Texture, String> {
    let texture = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
    for (name, value) in [
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
    ] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
    }
    Ok(texture)
}

/// OpenGL reading the decoder's Direct3D 11 textures through WGL_NV_DX_interop2, which Intel,
/// NVIDIA and AMD drivers provide on Windows.
#[cfg(windows)]
mod interop {
    use super::Resources;
    use crate::video::dxgl::{GpuFrame, Ring};
    use eframe::glow::{self, HasContext};
    use std::ffi::c_void;
    use std::sync::Arc;
    use windows::core::{Interface, PCSTR};
    use windows::Win32::Graphics::OpenGL::wglGetProcAddress;

    type Open = unsafe extern "system" fn(*mut c_void) -> *mut c_void;
    type Close = unsafe extern "system" fn(*mut c_void) -> i32;
    type Register = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32) -> *mut c_void;
    type Unregister = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
    type Lock = unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void) -> i32;

    const READ_ONLY: u32 = 0;

    struct Calls {
        open: Open,
        close: Close,
        register: Register,
        unregister: Unregister,
        lock: Lock,
        unlock: Lock,
    }

    unsafe fn load() -> Option<Calls> {
        unsafe fn get<T>(name: &str) -> Option<T> {
            let name = std::ffi::CString::new(name).ok()?;
            let f = wglGetProcAddress(PCSTR(name.as_ptr().cast()))?;
            Some(std::mem::transmute_copy(&f))
        }
        Some(Calls {
            open: get("wglDXOpenDeviceNV")?,
            close: get("wglDXCloseDeviceNV")?,
            register: get("wglDXRegisterObjectNV")?,
            unregister: get("wglDXUnregisterObjectNV")?,
            lock: get("wglDXLockObjectsNV")?,
            unlock: get("wglDXUnlockObjectsNV")?,
        })
    }

    pub struct Interop {
        calls: Calls,
        device: *mut c_void,
        ring: Arc<Ring>,
        gl: Arc<glow::Context>,
        /// Per slot: the OpenGL textures and their interop handles.
        slots: Vec<(glow::Texture, glow::Texture, [*mut c_void; 2])>,
    }

    /// Both textures of one slot, readable by OpenGL until this is dropped.
    pub struct Locked<'a> {
        interop: &'a Interop,
        handles: [*mut c_void; 2],
        pub luma: glow::Texture,
        pub chroma: glow::Texture,
    }
    impl Drop for Locked<'_> {
        fn drop(&mut self) {
            let mut handles = self.handles;
            // SAFETY: the handles were locked by `Interop::lock` on this context.
            unsafe { (self.interop.calls.unlock)(self.interop.device, 2, handles.as_mut_ptr()) };
        }
    }

    // SAFETY: used only inside eframe's paint callback, on the thread that owns the OpenGL context.
    unsafe impl Send for Interop {}
    unsafe impl Sync for Interop {}

    impl Interop {
        pub fn lock(&self, index: usize) -> Option<Locked<'_>> {
            let (luma, chroma, mut handles) = *self.slots.get(index)?;
            // SAFETY: registered handles on this device.
            let ok = unsafe { (self.calls.lock)(self.device, 2, handles.as_mut_ptr()) } != 0;
            ok.then_some(Locked { interop: self, handles, luma, chroma })
        }
    }

    impl Drop for Interop {
        fn drop(&mut self) {
            // SAFETY: everything here was registered or opened by this value on the same context.
            unsafe {
                for (luma, chroma, handles) in self.slots.drain(..) {
                    for h in handles {
                        (self.calls.unregister)(self.device, h);
                    }
                    self.gl.delete_texture(luma);
                    self.gl.delete_texture(chroma);
                }
                (self.calls.close)(self.device);
            }
        }
    }

    /// Makes sure the ring `frame` belongs to is registered with OpenGL.
    pub unsafe fn prepare(gl: &Arc<glow::Context>, resources: &mut Resources, frame: &GpuFrame) -> Result<(), ()> {
        if resources.interop.as_ref().is_some_and(|i| i.ring.id == frame.ring.id) {
            return Ok(());
        }
        // The old ring's textures are let go before the new ones are registered.
        resources.shown_gpu = None;
        resources.interop = None;
        let calls = load().ok_or(())?;
        let device = (calls.open)(frame.ring.device.as_raw());
        if device.is_null() {
            return Err(());
        }
        let mut interop = Interop { calls, device, ring: frame.ring.clone(), gl: gl.clone(), slots: Vec::new() };
        for slot in &frame.ring.slots {
            let luma = super::filtered_texture(gl).map_err(|_| ())?;
            let chroma = super::filtered_texture(gl).map_err(|_| ())?;
            let l = (interop.calls.register)(device, slot.luma.as_raw(), luma.0.get(), glow::TEXTURE_2D, READ_ONLY);
            let c = (interop.calls.register)(device, slot.chroma.as_raw(), chroma.0.get(), glow::TEXTURE_2D, READ_ONLY);
            interop.slots.push((luma, chroma, [l, c]));
            if l.is_null() || c.is_null() {
                return Err(());
            }
        }
        // The filters set on the textures before registering are kept; set them again to be sure.
        for (luma, chroma, _) in &interop.slots {
            for t in [*luma, *chroma] {
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            }
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        resources.interop = Some(interop);
        Ok(())
    }
}

unsafe fn upload(gl: &glow::Context, resources: &mut Resources, picture: &Picture) {
    let [w, h] = picture.size;
    let [cw, ch] = picture.chroma_size();
    let resized = resources.size != picture.size;
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
    for (texture, unit, format, internal, width, height, data) in [
        (resources.luma, glow::TEXTURE0, glow::RED, glow::R8, w, h, &picture.y),
        (resources.chroma, glow::TEXTURE1, glow::RG, glow::RG8, cw, ch, &picture.uv),
    ] {
        gl.active_texture(unit);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        let pixels = glow::PixelUnpackData::Slice(Some(data));
        if resized {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                width as i32,
                height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                pixels,
            );
        } else {
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                width as i32,
                height as i32,
                format,
                glow::UNSIGNED_BYTE,
                pixels,
            );
        }
    }
    resources.size = picture.size;
    resources.full_colour = picture.packed;
}

//! Draws NV12 pictures with a small shader inside eframe's own OpenGL context, so
//! colour conversion and scaling run on the graphics card. No extra dependency.
use crate::picture::Picture;
use eframe::{
    egui,
    egui_glow::{self, ShaderVersion},
    glow::{self, HasContext},
};
use std::sync::{Arc, Mutex};

pub type Handle = Arc<Mutex<Shared>>;

#[derive(Default)]
pub struct Shared {
    pending: Option<Picture>,
    gl: Option<Resources>,
    failed: bool,
}

struct Resources {
    program: glow::Program,
    vao: glow::VertexArray,
    luma: glow::Texture,
    chroma: glow::Texture,
    size: [usize; 2],
}

/// None when the context is too old for the shader; the caller then converts on the CPU.
pub fn new(gl: &Arc<glow::Context>) -> Option<Handle> {
    ShaderVersion::get(gl)
        .is_new_shader_interface()
        .then(Handle::default)
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
            if draw(&mut shared, painter.gl()).is_err() {
                shared.failed = true;
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
in vec2 coordinate;
out vec4 colour;
void main() {
    float y = 1.164383 * (texture(luma, coordinate).r - 0.0627451);
    vec2 c = texture(chroma, coordinate).rg - vec2(0.5019608);
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
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(resources.luma));
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(resources.chroma));
        gl.bind_vertex_array(Some(resources.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.active_texture(glow::TEXTURE0);
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
    let texture = || -> Result<glow::Texture, String> {
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
    };
    Ok(Resources {
        program,
        vao: gl.create_vertex_array()?,
        luma: texture()?,
        chroma: texture()?,
        size: [0, 0],
    })
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
}

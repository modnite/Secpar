//! GPU NV12 conversion. The normal frame path never reads video pixels back.
use eframe::{glow::{self,HasContext},egui_glow::ShaderVersion};
use gstreamer_video::{VideoInfo,VideoColorMatrix,VideoColorRange,VideoChromaSite};

#[derive(Clone,Copy)]
pub struct Color {matrix:[f32;9],offset:[f32;3]}
impl Color {
    pub fn from_info(info:&VideoInfo)->Self {
        let colors=info.colorimetry();
        let(kr,kb)=match colors.matrix(){VideoColorMatrix::Bt601=>(0.299,0.114),VideoColorMatrix::Bt2020=>(0.2627,0.0593),VideoColorMatrix::Bt709=>(0.2126,0.0722),_=>if info.height()>=720{(0.2126,0.0722)}else{(0.299,0.114)}};
        Self::new(kr,kb,colors.range()==VideoColorRange::Range0_255)
    }
    fn new(kr:f32,kb:f32,full:bool)->Self {
        let kg=1.-kr-kb;let y=if full{1.}else{255./219.};let c=if full{1.}else{255./224.};
        Self{matrix:[y,y,y,0.,-2.*kb*(1.-kb)/kg*c,2.*(1.-kb)*c,2.*(1.-kr)*c,-2.*kr*(1.-kr)/kg*c,0.],
            offset:[if full{0.}else{-16./255.},-128./255.,-128./255.]}
    }
}
#[derive(Clone,Copy)]
pub struct Geometry {y_scale:[f32;2],uv_scale:[f32;2],y_bounds:[f32;4],uv_bounds:[f32;4],shift:[f32;2]}
impl Geometry {
    pub fn new(visible:(u32,u32),storage:(u32,u32),site:VideoChromaSite)->Result<Self,String> {
        if visible.0==0||visible.1==0||visible.0>storage.0||visible.1>storage.1||storage.0>8192||storage.1>8192{return Err("Invalid native video geometry".into());}
        let scale=|v:(u32,u32),s:(u32,u32)|[v.0 as f32/s.0 as f32,v.1 as f32/s.1 as f32];
        let bounds=|v:(u32,u32),s:(u32,u32)|[0.5/s.0 as f32,0.5/s.1 as f32,(v.0 as f32-0.5)/s.0 as f32,(v.1 as f32-0.5)/s.1 as f32];
        let uv=(visible.0.div_ceil(2),visible.1.div_ceil(2));let uv_store=(storage.0.div_ceil(2),storage.1.div_ceil(2));
        Ok(Self{y_scale:scale(visible,storage),uv_scale:scale(uv,uv_store),y_bounds:bounds(visible,storage),uv_bounds:bounds(uv,uv_store),
            shift:[if site.contains(VideoChromaSite::H_COSITED){0.5/storage.0 as f32}else{0.},if site.contains(VideoChromaSite::V_COSITED){0.5/storage.1 as f32}else{0.}]})
    }
}
pub struct Shader {program:glow::NativeProgram,vao:glow::NativeVertexArray,uniforms:Vec<glow::NativeUniformLocation>}
impl Shader {
    pub fn new(gl:&glow::Context)->Result<Self,String> {
        let version=ShaderVersion::get(gl);
        if !matches!(version,ShaderVersion::Gl140|ShaderVersion::Es300){return Err("Native video needs OpenGL3 or GLES3".into());}
        let prefix=format!("{}{}",version.version_declaration(),if version.is_embedded(){"precision highp float;\n"}else{""});
        let vertex=format!("{prefix}out vec2 video_uv;void main(){{vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2));video_uv=vec2(p.x,1.0-p.y);gl_Position=vec4(p*2.0-1.0,0.0,1.0);}}");
        let fragment=format!("{prefix}in vec2 video_uv;out vec4 color;uniform sampler2D plane_y;uniform sampler2D plane_uv;uniform mat3 conversion;uniform vec3 offsets;uniform vec2 y_scale;uniform vec2 uv_scale;uniform vec4 y_bounds;uniform vec4 uv_bounds;uniform vec2 chroma_shift;void main(){{vec2 y=clamp(video_uv*y_scale,y_bounds.xy,y_bounds.zw);vec2 uv=clamp(video_uv*uv_scale+chroma_shift,uv_bounds.xy,uv_bounds.zw);vec3 sample_yuv=vec3(texture(plane_y,y).r,texture(plane_uv,uv).rg)+offsets;color=vec4(conversion*sample_yuv,1.0);}}");
        unsafe {
            let vs=compile(gl,glow::VERTEX_SHADER,&vertex)?;
            let fs=match compile(gl,glow::FRAGMENT_SHADER,&fragment){Ok(s)=>s,Err(e)=>{gl.delete_shader(vs);return Err(e)}};
            let program=match gl.create_program(){Ok(p)=>p,Err(e)=>{gl.delete_shader(vs);gl.delete_shader(fs);return Err(e)}};
            gl.attach_shader(program,vs);gl.attach_shader(program,fs);gl.link_program(program);
            gl.detach_shader(program,vs);gl.detach_shader(program,fs);gl.delete_shader(vs);gl.delete_shader(fs);
            if !gl.get_program_link_status(program){let error=gl.get_program_info_log(program);gl.delete_program(program);return Err(error);}
            let vao=match gl.create_vertex_array(){Ok(v)=>v,Err(e)=>{gl.delete_program(program);return Err(e)}};
            let names=["plane_y","plane_uv","conversion","offsets","y_scale","uv_scale","y_bounds","uv_bounds","chroma_shift"];
            let uniforms:Option<Vec<_>>=names.iter().map(|n|gl.get_uniform_location(program,n)).collect();
            let Some(uniforms)=uniforms else{gl.delete_vertex_array(vao);gl.delete_program(program);return Err("Video shader uniforms are unavailable".into())};
            Ok(Self{program,vao,uniforms})
        }
    }
    /// Called only with its creating OpenGL context current. Egui restores its
    /// own state after this callback. NV12 texture data stays on the GPU.
    pub fn draw(&self,gl:&glow::Context,textures:[glow::NativeTexture;2],color:Color,geometry:Geometry) {
        unsafe {
            gl.disable(glow::BLEND);gl.use_program(Some(self.program));gl.bind_vertex_array(Some(self.vao));
            for(i,texture)in textures.into_iter().enumerate(){gl.active_texture(glow::TEXTURE0+i as u32);gl.bind_texture(glow::TEXTURE_2D,Some(texture));gl.uniform_1_i32(Some(&self.uniforms[i]),i as i32);}
            gl.uniform_matrix_3_f32_slice(Some(&self.uniforms[2]),false,&color.matrix);
            gl.uniform_3_f32_slice(Some(&self.uniforms[3]),&color.offset);
            gl.uniform_2_f32_slice(Some(&self.uniforms[4]),&geometry.y_scale);gl.uniform_2_f32_slice(Some(&self.uniforms[5]),&geometry.uv_scale);
            gl.uniform_4_f32_slice(Some(&self.uniforms[6]),&geometry.y_bounds);gl.uniform_4_f32_slice(Some(&self.uniforms[7]),&geometry.uv_bounds);
            gl.uniform_2_f32_slice(Some(&self.uniforms[8]),&geometry.shift);
            gl.draw_arrays(glow::TRIANGLES,0,3);gl.active_texture(glow::TEXTURE0);
        }
    }
    pub fn destroy(self,gl:&glow::Context){unsafe{gl.delete_vertex_array(self.vao);gl.delete_program(self.program);}}
}
unsafe fn compile(gl:&glow::Context,kind:u32,source:&str)->Result<glow::NativeShader,String> {
    let shader=gl.create_shader(kind)?;gl.shader_source(shader,source);gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader){let error=gl.get_shader_info_log(shader);gl.delete_shader(shader);Err(error)}else{Ok(shader)}
}

#[cfg(test)]
#[path="egl_test.rs"]
mod egl_test;

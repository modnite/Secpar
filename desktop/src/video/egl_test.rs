//! Software EGL fixture: never binds a user display or a DRM/render device.
use super::*;
use std::{ffi::{c_void,c_char,CString},ptr};
type Display=unsafe extern "C" fn(u32,*mut c_void,*const i32)->*mut c_void;
type Initialize=unsafe extern "C" fn(*mut c_void,*mut i32,*mut i32)->u32;
type Bind=unsafe extern "C" fn(u32)->u32;
type Choose=unsafe extern "C" fn(*mut c_void,*const i32,*mut *mut c_void,i32,*mut i32)->u32;
type CreateContext=unsafe extern "C" fn(*mut c_void,*mut c_void,*mut c_void,*const i32)->*mut c_void;
type CreateSurface=unsafe extern "C" fn(*mut c_void,*mut c_void,*const i32)->*mut c_void;
type Current=unsafe extern "C" fn(*mut c_void,*mut c_void,*mut c_void,*mut c_void)->u32;
type Destroy=unsafe extern "C" fn(*mut c_void,*mut c_void)->u32;
type Terminate=unsafe extern "C" fn(*mut c_void)->u32;
type Proc=unsafe extern "C" fn(*const c_char)->*const c_void;
struct Egl {
    _library:libloading::Library,display:*mut c_void,context:*mut c_void,surface:*mut c_void,
    current:Current,destroy_context:Destroy,destroy_surface:Destroy,terminate:Terminate,
    gl:glow::Context,
}
impl Egl {
    fn open(es:bool)->Self {
        assert_eq!(std::env::var("SECPAR_GPU_REVIEW").as_deref(),Ok("1"));
        unsafe {
            let lib=libloading::Library::new("libEGL.so.1").unwrap();
            let proc=*lib.get::<Proc>(b"eglGetProcAddress\0").unwrap();
            let raw=proc(c"eglGetPlatformDisplayEXT".as_ptr());assert!(!raw.is_null());
            let get_display:Display=std::mem::transmute(raw);
            let initialize=*lib.get::<Initialize>(b"eglInitialize\0").unwrap();
            let bind=*lib.get::<Bind>(b"eglBindAPI\0").unwrap();let choose=*lib.get::<Choose>(b"eglChooseConfig\0").unwrap();
            let create_context=*lib.get::<CreateContext>(b"eglCreateContext\0").unwrap();let create_surface=*lib.get::<CreateSurface>(b"eglCreatePbufferSurface\0").unwrap();
            let current=*lib.get::<Current>(b"eglMakeCurrent\0").unwrap();let destroy_context=*lib.get::<Destroy>(b"eglDestroyContext\0").unwrap();
            let destroy_surface=*lib.get::<Destroy>(b"eglDestroySurface\0").unwrap();let terminate=*lib.get::<Terminate>(b"eglTerminate\0").unwrap();
            let display=get_display(0x31dd,ptr::null_mut(),ptr::null());assert!(!display.is_null());
            assert_ne!(initialize(display,ptr::null_mut(),ptr::null_mut()),0);
            assert_ne!(bind(if es{0x30a0}else{0x30a2}),0);
            let attrs=[0x3033,1,0x3040,if es{0x40}else{8},0x3024,8,0x3023,8,0x3022,8,0x3021,8,0x3038];
            let mut config=ptr::null_mut();let mut count=0;
            assert_ne!(choose(display,attrs.as_ptr(),&mut config,1,&mut count),0);assert_eq!(count,1);
            let surface=create_surface(display,config,[0x3057,4,0x3056,4,0x3038].as_ptr());assert!(!surface.is_null());
            let context_attrs=if es{vec![0x3098,3,0x3038]}else{vec![0x3098,3,0x30fb,3,0x30fd,1,0x3038]};
            let context=create_context(display,config,ptr::null_mut(),context_attrs.as_ptr());assert!(!context.is_null());
            assert_ne!(current(display,surface,surface,context),0);
            let gl=glow::Context::from_loader_function(|name|proc(CString::new(name).unwrap().as_ptr()));
            Self{_library:lib,display,context,surface,current,destroy_context,destroy_surface,terminate,gl}
        }
    }
}
impl Drop for Egl {fn drop(&mut self){unsafe{
    (self.current)(self.display,ptr::null_mut(),ptr::null_mut(),ptr::null_mut());
    (self.destroy_context)(self.display,self.context);(self.destroy_surface)(self.display,self.surface);(self.terminate)(self.display);
}}}
unsafe fn texture(gl:&glow::Context,size:i32,uv:bool,data:&[u8])->glow::NativeTexture {
    let t=gl.create_texture().unwrap();gl.bind_texture(glow::TEXTURE_2D,Some(t));
    for p in [glow::TEXTURE_MIN_FILTER,glow::TEXTURE_MAG_FILTER]{gl.tex_parameter_i32(glow::TEXTURE_2D,p,glow::LINEAR as i32);}
    for p in [glow::TEXTURE_WRAP_S,glow::TEXTURE_WRAP_T]{gl.tex_parameter_i32(glow::TEXTURE_2D,p,glow::CLAMP_TO_EDGE as i32);}
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT,1);
    gl.tex_image_2d(glow::TEXTURE_2D,0,if uv{glow::RG8}else{glow::R8} as i32,size,size,0,if uv{glow::RG}else{glow::RED},glow::UNSIGNED_BYTE,glow::PixelUnpackData::Slice(Some(data)));t
}
fn check(es:bool,padded:bool,bt709:bool,full:bool) {
    let egl=Egl::open(es);let gl=&egl.gl;let shader=Shader::new(gl).unwrap();
    let store=if padded{8}else{4};let mut y=vec![16u8;store*store];let mut uv=vec![128u8;store*store/2];
    // Independently specified8-bit white/red/blue/green reference patches.
    let(levels,chroma)=match(bt709,full){
        (false,false)=>([[235,81],[41,145]],[[[128,128],[90,240]],[[240,110],[54,34]]]),
        (true,false)=>([[235,63],[32,173]],[[[128,128],[102,240]],[[240,118],[42,26]]]),
        (false,true)=>([[255,76],[29,150]],[[[128,128],[85,255]],[[255,107],[44,21]]]),
        (true,true)=>([[255,54],[18,182]],[[[128,128],[99,255]],[[255,116],[30,12]]]),
    };
    // A visible4x4 frame, optionally in8x8 storage. Padding must not bleed in.
    for row in 0..4{for col in 0..4{y[row*store+col]=levels[row/2][col/2];}}
    for row in 0..2{for col in 0..2{let pair=chroma[row][col];let i=(row*(store/2)+col)*2;uv[i..i+2].copy_from_slice(&pair);}}
    unsafe {
        let textures=[texture(gl,store as i32,false,&y),texture(gl,store as i32/2,true,&uv)];
        gl.viewport(0,0,4,4);gl.disable(glow::SCISSOR_TEST);gl.clear_color(1.,0.,1.,1.);gl.clear(glow::COLOR_BUFFER_BIT);
        shader.draw(gl,textures,Color::new(if bt709{0.2126}else{0.299},if bt709{0.0722}else{0.114},full),Geometry::new((4,4),(store as u32,store as u32),VideoChromaSite::NONE).unwrap());
        let mut pixels=[0u8;64];gl.read_pixels(0,0,4,4,glow::RGBA,glow::UNSIGNED_BYTE,glow::PixelPackData::Slice(Some(&mut pixels)));
        for(x,row,expected)in[(0,3,[255,255,255]),(3,3,[254,0,0]),(0,0,[0,0,255]),(3,0,[0,255,1])] {
            let offset=(row*4+x)*4;let actual=&pixels[offset..offset+3];
            assert!(actual.iter().zip(expected).all(|(a,b)|(*a as i16-b as i16).abs()<=3),"es={es} padded={padded} bt709={bt709} full={full} corner{x},{row} got{actual:?} expected{expected:?}");
        }
        assert_eq!(gl.get_error(),glow::NO_ERROR);
        for t in textures{gl.delete_texture(t);}shader.destroy(gl);
    }
}
#[test]
#[ignore="isolated software EGL only; SECPAR_GPU_REVIEW=1"]
fn gpu_nv12_colors_orientation_and_padding_on_gl_and_gles() {
    for es in [false,true]{for padded in [false,true]{for bt709 in [false,true]{for full in [false,true]{check(es,padded,bt709,full);}}}}
}

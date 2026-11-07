//! Import validated VA planes into OpenGL without CPU video mapping.
//! Khronos EGL_EXT_image_dma_buf_import[_modifiers], GL_OES_EGL_image.
use super::dmabuf::ExportedFrame;
use eframe::glow::{self,HasContext};
use std::{ffi::{c_void,c_char,CStr},os::fd::AsRawFd,sync::Arc,thread::{self,ThreadId},ptr};
type Ptr=*mut c_void;
type Current=unsafe extern "C" fn()->Ptr;
type Query=unsafe extern "C" fn(Ptr,i32)->*const c_char;
type Proc=unsafe extern "C" fn(*const c_char)->*const c_void;
type Image=unsafe extern "C" fn(Ptr,Ptr,u32,Ptr,*const i32)->Ptr;
type Destroy=unsafe extern "C" fn(Ptr,Ptr)->u32;
type Modifiers=unsafe extern "C" fn(Ptr,i32,i32,*mut u64,*mut u32,*mut i32)->u32;
type Target=unsafe extern "C" fn(u32,Ptr);
const R8:u32=u32::from_le_bytes(*b"R8  ");
const GR88:u32=u32::from_le_bytes(*b"GR88");
pub struct ImportContext {
    _library:libloading::Library,display:usize,context:usize,thread:ThreadId,
    current_display:Current,current_context:Current,image:Image,destroy:Destroy,target:Target,
    modifiers:Vec<u64>,
}
#[must_use="Retain imported frames until their GL fence completes"]
pub struct Imported {
    frame:Arc<ExportedFrame>,textures:[glow::NativeTexture;2],images:[usize;2],
    fence:Option<usize>,sync_failed:bool,
}
impl ImportContext {
    pub fn current()->Result<Self,String> {
        unsafe {
            let library=libloading::Library::new("libEGL.so.1").map_err(|_|"EGL library is unavailable")?;
            let current_display=*library.get::<Current>(b"eglGetCurrentDisplay\0").map_err(|_|"EGL display lookup is unavailable")?;
            let current_context=*library.get::<Current>(b"eglGetCurrentContext\0").map_err(|_|"EGL context lookup is unavailable")?;
            let query=*library.get::<Query>(b"eglQueryString\0").map_err(|_|"EGL capabilities are unavailable")?;
            let proc=*library.get::<Proc>(b"eglGetProcAddress\0").map_err(|_|"EGL entry points are unavailable")?;
            let display=current_display();let context=current_context();
            if display.is_null()||context.is_null(){return Err("Video window has no current EGL context".into());}
            let extensions=query(display,0x3055);if extensions.is_null(){return Err("EGL extensions are unavailable".into());}
            let extensions=CStr::from_ptr(extensions).to_string_lossy();
            for needed in ["EGL_EXT_image_dma_buf_import","EGL_EXT_image_dma_buf_import_modifiers"]{
                if !extensions.split_whitespace().any(|s|s==needed){return Err(format!("Native video requires {needed}"));}
            }
            let image=proc(c"eglCreateImageKHR".as_ptr());let destroy=proc(c"eglDestroyImageKHR".as_ptr());
            let target=proc(c"glEGLImageTargetTexture2DOES".as_ptr());let mods=proc(c"eglQueryDmaBufModifiersEXT".as_ptr());
            if [image,destroy,target,mods].iter().any(|p|p.is_null()){return Err("Required EGL image functions are unavailable".into());}
            let mods:Modifiers=std::mem::transmute(mods);
            let y=query_modifiers(mods,display,R8)?;let uv=query_modifiers(mods,display,GR88)?;
            let modifiers:Vec<_>=y.into_iter().filter(|m|uv.contains(m)).collect();
            if modifiers.is_empty(){return Err("No shared NV12 plane modifiers support2D textures".into());}
            Ok(Self{_library:library,display:display as usize,context:context as usize,thread:thread::current().id(),
                current_display,current_context,image:std::mem::transmute(image),destroy:std::mem::transmute(destroy),target:std::mem::transmute(target),modifiers})
        }
    }
    pub fn modifiers(&self)->&[u64]{&self.modifiers}
    pub fn is_current(&self)->bool {self.thread==thread::current().id() && unsafe{(self.current_display)() as usize==self.display && (self.current_context)() as usize==self.context}}
    pub fn import(&self,gl:&glow::Context,frame:Arc<ExportedFrame>)->Result<Imported,String> {
        if !self.is_current(){return Err("Video EGL context changed".into());}
        let version=gl.version();
        if !supports_fences(version.major,version.minor,version.is_embedded,
            gl.supported_extensions().contains("GL_ARB_sync")) {
            return Err("Native video requires GPU fence synchronization".into());
        }
        let mut textures=Vec::new();let mut images=Vec::new();
        let result=(||->Result<(),String>{
            for i in 0..2 {
                let plane=frame.plane(i).ok_or("Missing native video plane")?;
                if !self.modifiers.contains(&plane.modifier){return Err("Native video modifier is unsupported".into());}
                let offset=i32::try_from(plane.offset).map_err(|_|"Native video offset is too large")?;
                let pitch=i32::try_from(plane.pitch).map_err(|_|"Native video pitch is too large")?;
                let attrs=[0x3057,plane.width as i32,0x3056,plane.height as i32,0x3271,plane.fourcc as i32,0x3272,plane.fd.as_raw_fd(),0x3273,offset,0x3274,pitch,0x3443,plane.modifier as u32 as i32,0x3444,(plane.modifier>>32)as u32 as i32,0x3038];
                unsafe {
                    let image=(self.image)(self.display as Ptr,ptr::null_mut(),0x3270,ptr::null_mut(),attrs.as_ptr());
                    if image.is_null(){return Err("EGL could not import native video storage".into());}
                    images.push(image as usize);
                    let texture=gl.create_texture()?;textures.push(texture);
                    gl.active_texture(glow::TEXTURE0);gl.bind_texture(glow::TEXTURE_2D,Some(texture));
                    (self.target)(glow::TEXTURE_2D,image);
                    if gl.get_error()!=glow::NO_ERROR{return Err("OpenGL could not bind the imported video plane".into());}
                    for p in [glow::TEXTURE_MIN_FILTER,glow::TEXTURE_MAG_FILTER]{gl.tex_parameter_i32(glow::TEXTURE_2D,p,glow::LINEAR as i32);}
                    for p in [glow::TEXTURE_WRAP_S,glow::TEXTURE_WRAP_T]{gl.tex_parameter_i32(glow::TEXTURE_2D,p,glow::CLAMP_TO_EDGE as i32);}
                }
            }
            Ok(())
        })();
        if let Err(error)=result {
            unsafe{for t in textures{gl.delete_texture(t);}for image in images{(self.destroy)(self.display as Ptr,image as Ptr);}}
            return Err(error);
        }
        Ok(Imported{frame,textures:textures.try_into().unwrap(),images:images.try_into().unwrap(),fence:None,sync_failed:false})
    }
    /// Returns the frame unchanged if its last draw has not finished. Callers
    /// retain that result in a bounded retirement queue rather than recycling VA.
    pub fn try_release(&self,gl:&glow::Context,frame:Imported)->Result<(),Imported> {
        if !self.is_current()||!frame.ready(gl){return Err(frame);}
        self.release_completed(gl,frame);Ok(())
    }
    fn release_completed(&self,gl:&glow::Context,frame:Imported){unsafe{
        if let Some(fence)=frame.fence{gl.delete_sync(glow::NativeFence(fence as *mut _));}
        for t in frame.textures{gl.delete_texture(t);}for image in frame.images{(self.destroy)(self.display as Ptr,image as Ptr);}
        // The retained sample/fds drop only after completion and image teardown.
    }}
    /// Shutdown only, with the owning context current. Normal playback never waits.
    pub fn finish_and_release(&self,gl:&glow::Context,frame:Imported)->Result<(),Imported>{
        if !self.is_current(){return Err(frame);}
        unsafe{gl.finish();}self.release_completed(gl,frame);Ok(())
    }
}
impl Imported {
    pub fn frame(&self)->&ExportedFrame{&self.frame}
    pub fn textures(&self)->[glow::NativeTexture;2]{self.textures}
    pub fn mark_drawn(&mut self,gl:&glow::Context)->Result<(),String>{unsafe{
        match gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE,0) {
            Ok(fence)=>{
                if fence.0.is_null(){self.sync_failed=true;return Err("GPU synchronization did not start".into());}
                if let Some(previous)=self.fence.replace(fence.0 as usize){gl.delete_sync(glow::NativeFence(previous as *mut _));}
                Ok(())
            },Err(error)=>{self.sync_failed=true;Err(error)},
        }
    }}
    fn ready(&self,gl:&glow::Context)->bool {
        if self.sync_failed{return false;}
        self.fence.is_none_or(|fence|unsafe{matches!(gl.client_wait_sync(glow::NativeFence(fence as *mut _),0,0),glow::ALREADY_SIGNALED|glow::CONDITION_SATISFIED)})
    }
}
// A GL3.1 UI context can render the shader but lacks core sync functions.
// Refuse native storage before import rather than calling an unloaded entry point.
fn supports_fences(major:u32,minor:u32,embedded:bool,arb_sync:bool)->bool {
    if embedded {major>=3} else {(major,minor)>=(3,2)||arb_sync}
}
unsafe fn query_modifiers(query:Modifiers,display:Ptr,format:u32)->Result<Vec<u64>,String>{
    let mut count=0;
    if query(display,format as i32,0,ptr::null_mut(),ptr::null_mut(),&mut count)==0||!(1..=64).contains(&count){return Err("Unsupported native plane format or modifier count".into());}
    let mut values=vec![0u64;count as usize];let mut external=vec![0u32;count as usize];let mut written=0;
    if query(display,format as i32,count,values.as_mut_ptr(),external.as_mut_ptr(),&mut written)==0||written<0||written>count{return Err("EGL modifier lookup failed".into());}
    Ok(values.into_iter().zip(external).take(written as usize).filter_map(|(modifier,only)|if only==0{Some(modifier)}else{None}).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fence_support_rejects_gl31_without_sync_but_accepts_core_and_extension() {
        assert!(!supports_fences(3,1,false,false));
        assert!(supports_fences(3,1,false,true));
        assert!(supports_fences(3,2,false,false));
        assert!(supports_fences(4,6,false,false));
        assert!(!supports_fences(2,0,true,false));
        assert!(supports_fences(3,0,true,false));
    }
    #[test]
    fn no_current_egl_context_is_a_clean_unavailable_result(){assert!(ImportContext::current().is_err());}
    #[test]
    fn import_ownership_can_cross_decoder_and_ui_threads(){
        fn send_sync<T:Send+Sync>(){}send_sync::<ImportContext>();send_sync::<Imported>();
    }
    unsafe extern "C" fn modifiers(_:Ptr,_:i32,max:i32,values:*mut u64,external:*mut u32,count:*mut i32)->u32 {
        if count.is_null(){return 0;}*count=3;
        if max==0{return 1;}
        if max!=3||values.is_null()||external.is_null(){return 0;}
        for(i,(value,only))in[(7,0),(8,1),(9,0)].into_iter().enumerate(){*values.add(i)=value;*external.add(i)=only;}
        1
    }
    #[test]
    fn modifier_queries_use_driver_count_and_exclude_external_only_formats(){
        assert_eq!(unsafe{query_modifiers(modifiers,ptr::null_mut(),R8)}.unwrap(),vec![7,9]);
    }
    unsafe extern "C" fn excessive(_:Ptr,_:i32,max:i32,_:*mut u64,_:*mut u32,count:*mut i32)->u32 {
        if max!=0{return 0;}*count=10000;1
    }
    #[test]
    fn excessive_driver_modifier_counts_are_rejected(){assert!(unsafe{query_modifiers(excessive,ptr::null_mut(),R8)}.is_err());}
}

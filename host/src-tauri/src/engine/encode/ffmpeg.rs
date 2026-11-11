//! FFmpeg's hardware encoders; frames stay in D3D11 memory, including QSV mapping.
use super::{Codec, Encoded};
use std::{cell::Cell, ffi::{c_char,c_void,CStr,CString}, ptr::NonNull};
use windows::{core::{Error,HRESULT,Interface},Win32::Graphics::Direct3D11::{ID3D11Device,ID3D11Texture2D}};
unsafe extern "C" {
    fn secpar_avcodec_open(dev:*mut c_void,backend:*const c_char,codec:u32,w:u32,h:u32,fps:u32,kbps:u32,quality:u32,status:*mut i32)->*mut c_void;
    fn secpar_avcodec_send(enc:*mut c_void,tex:*mut c_void,stamp:u64,key:bool)->i32;
    fn secpar_avcodec_receive(enc:*mut c_void,data:*mut *const u8,size:*mut u32,stamp:*mut u64,key:*mut bool)->i32;
    fn secpar_avcodec_bitrate(enc:*mut c_void,kbps:u32)->i32;
    fn secpar_avcodec_error(status:i32,text:*mut c_char,size:u32);
    fn secpar_avcodec_close(enc:*mut c_void);
}
fn checked(status:i32)->windows::core::Result<()> {
    if status>=0 {return Ok(())}
    let mut text=[0 as c_char;256];
    // SAFETY: writable, bounded buffer; av_strerror terminates it.
    unsafe {secpar_avcodec_error(status,text.as_mut_ptr(),text.len() as u32)};
    let detail=unsafe {CStr::from_ptr(text.as_ptr())}.to_string_lossy();
    Err(Error::new(HRESULT(0x80004005u32 as i32),format!("FFmpeg: {detail} ({status})")))
}
pub(super) struct Encoder { raw:NonNull<c_void>, kbps:Cell<u32> }
impl Encoder {
    pub fn new(dev:&ID3D11Device,codec:Codec,(w,h):(u32,u32),fps:u32,kbps:u32,backend:&str,quality:u32)->windows::core::Result<Self> {
        let backend=CString::new(backend).map_err(|_|Error::from_hresult(HRESULT(0x80070057u32 as i32)))?;
        let format=match codec {Codec::H264=>0,Codec::Hevc=>1,Codec::Av1=>2};
        let mut status=0;
        // SAFETY: live borrowed COM device, retained by FFmpeg; string is live for this call.
        let raw=unsafe {secpar_avcodec_open(dev.as_raw(),backend.as_ptr(),format,w,h,fps,kbps,quality,&mut status)};
        checked(status)?;
        Ok(Self{raw:NonNull::new(raw).ok_or_else(||Error::from_hresult(HRESULT(0x80004005u32 as i32)))?,kbps:Cell::new(kbps)})
    }
    pub fn set_bitrate(&self,kbps:u32)->windows::core::Result<()> {
        // SAFETY: encode and configuration execute serially on this session's video thread.
        checked(unsafe {secpar_avcodec_bitrate(self.raw.as_ptr(),kbps)})?;
        self.kbps.set(kbps);Ok(())
    }
    pub fn bitrate(&self)->u32 {self.kbps.get()}
    pub fn encode(&mut self,texture:&ID3D11Texture2D,stamp:u64,key:bool)->windows::core::Result<Vec<Encoded>> {
        // SAFETY: GPU resource remains live until the bridge enqueues its copy.
        checked(unsafe {secpar_avcodec_send(self.raw.as_ptr(),texture.as_raw(),stamp,key)})?;
        let mut frames=Vec::new();
        loop {
            let (mut data,mut size,mut stamp,mut key)=(std::ptr::null(),0,0,false);
            // SAFETY: valid out-parameters; packet bytes remain live until next receive/drop.
            let status=unsafe {secpar_avcodec_receive(self.raw.as_ptr(),&mut data,&mut size,&mut stamp,&mut key)};
            checked(status)?;
            if status==0 {break}
            if data.is_null() || size==0 {return Err(Error::from_hresult(HRESULT(0x80004005u32 as i32)))}
            let data=unsafe {std::slice::from_raw_parts(data,size as usize)}.to_vec();
            frames.push(Encoded{data,key,capture_us:stamp});
        }
        Ok(frames)
    }
}
impl Drop for Encoder {fn drop(&mut self) {
    // SAFETY: this wrapper uniquely owns the context; FFmpeg drains/releases retained frames.
    unsafe {secpar_avcodec_close(self.raw.as_ptr())};
}}

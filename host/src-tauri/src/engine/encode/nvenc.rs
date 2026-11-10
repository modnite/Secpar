//! Runtime-loaded native NVENC. The bridge retains its input resource until output completes.
use super::{Codec, Encoded};
use std::{cell::Cell, ffi::c_void, ptr::NonNull};
use windows::{core::{Interface, Error, HRESULT}, Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D}};
unsafe extern "C" {
    fn secpar_nvenc_open(dev:*mut c_void,codec:u32,w:u32,h:u32,fps:u32,kbps:u32,error:*mut i32)->*mut c_void;
    fn secpar_nvenc_encode(enc:*mut c_void,tex:*mut c_void,time:u64,idr:bool,data:*mut *const u8,size:*mut u32,key:*mut bool)->i32;
    fn secpar_nvenc_bitrate(enc:*mut c_void,kbps:u32)->i32;
    fn secpar_nvenc_close(enc:*mut c_void);
}
fn checked(status:i32)->windows::core::Result<()> {
    if status==0 {Ok(())} else {Err(Error::new(HRESULT(0x80004005u32 as i32),format!("NVENC driver status {status}")))}
}
pub(super) struct Encoder {raw:NonNull<c_void>,kbps:Cell<u32>}
impl Encoder {
    pub fn new(dev:&ID3D11Device,codec:Codec,(w,h):(u32,u32),fps:u32,kbps:u32)->windows::core::Result<Self> {
        if w==0||h==0||fps==0||kbps==0||kbps>u32::MAX/1000 {return Err(Error::from_hresult(HRESULT(0x80070057u32 as i32)));}
        let mut error=0;
        // SAFETY: borrowed live COM device; bridge retains its own reference.
        let ptr=unsafe {secpar_nvenc_open(dev.as_raw(),codec as u32,w,h,fps,kbps,&mut error)};
        checked(error)?;
        Ok(Self {raw:NonNull::new(ptr).ok_or_else(||Error::from_hresult(HRESULT(0x80004005u32 as i32)))?,kbps:Cell::new(kbps)})
    }
    pub fn set_bitrate(&self,kbps:u32)->windows::core::Result<()> {
        if kbps==0||kbps>u32::MAX/1000 {return Err(Error::from_hresult(HRESULT(0x80070057u32 as i32)));}
        // SAFETY: session is live and called only by its owning encode thread.
        checked(unsafe {secpar_nvenc_bitrate(self.raw.as_ptr(),kbps)})?;
        self.kbps.set(kbps);Ok(())
    }
    pub fn bitrate(&self)->u32 {self.kbps.get()}
    pub fn encode(&mut self,tex:&ID3D11Texture2D,capture_us:u64,idr:bool)->windows::core::Result<Vec<Encoded>> {
        let (mut data,mut len,mut key)=(std::ptr::null(),0,false);
        // SAFETY: texture and bridge are live; returned bytes remain valid until next encode/drop.
        checked(unsafe {secpar_nvenc_encode(self.raw.as_ptr(),tex.as_raw(),capture_us,idr,&mut data,&mut len,&mut key)})?;
        if data.is_null()||len==0 {return Err(Error::from_hresult(HRESULT(0x80004005u32 as i32)));}
        let data=unsafe {std::slice::from_raw_parts(data,len as usize)}.to_vec();
        Ok(vec![Encoded {data,key,capture_us}])
    }
}
impl Drop for Encoder {fn drop(&mut self) {
    // SAFETY: unique ownership, destroyed once after the last encode completes.
    unsafe {secpar_nvenc_close(self.raw.as_ptr())};
}}

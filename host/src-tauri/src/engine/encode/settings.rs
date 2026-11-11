use super::{Codec,Encoder};
use serde::{Deserialize,Serialize};
use windows::{core::Interface,Win32::Graphics::{Direct3D11::ID3D11Device,Dxgi::IDXGIDevice}};

#[derive(Debug,Clone,PartialEq,Serialize,Deserialize)]
#[serde(default)]
pub struct Settings {pub backend:String,pub preset:String}
impl Default for Settings {fn default()->Self {Self{backend:"auto".into(),preset:"fast".into()}}}
impl Settings {
    pub fn validate(&self)->Result<(),String> {
        if !["auto","nvenc","ffmpeg-nvenc","ffmpeg-amf","ffmpeg-qsv","media-foundation"].contains(&self.backend.as_str()) {
            return Err("Unknown video encoder".into())
        }
        if !["fast","balanced","quality"].contains(&self.preset.as_str()) {return Err("Unknown encoder preset".into())}
        Ok(())
    }
    pub fn quality(&self)->u32 {match self.preset.as_str(){"balanced"=>1,"quality"=>2,_=>0}}
}
pub fn automatic(dev:&ID3D11Device)->&'static str {
    // SAFETY: the live D3D device retains its DXGI adapter; no raw pointer ownership changes.
    let vendor=unsafe {dev.cast::<IDXGIDevice>().and_then(|d|d.GetAdapter()).and_then(|a|a.GetDesc()).map(|d|d.VendorId)}.unwrap_or(0);
    match vendor {0x10de=>"nvenc",0x1002=>"ffmpeg-amf",0x8086=>"ffmpeg-qsv",_=>"unsupported"}
}
#[derive(Clone,Serialize)]
pub struct Choice {pub id:&'static str,pub name:&'static str,pub available:bool,pub reason:String}
/// Probe once per adapter, rather than opening hardware encoder sessions on every UI refresh.
/// A successful probe is not a promise that every resolution or concurrent-session count works.
pub fn choices(display:&str)->Vec<Choice> {
    use std::{collections::HashMap,sync::{Mutex,OnceLock}};
    static CACHE:OnceLock<Mutex<HashMap<i64,Vec<Choice>>>>=OnceLock::new();
    let Some((dev,_,_,luid))=super::super::capture::device(display) else {return vec![]};
    let mut cache=CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(found)=cache.get(&luid) {return found.clone()}
    let native=automatic(&dev);
    let mut options=vec![];
    for (id,name,matching) in [
        ("nvenc","NVIDIA NVENC",native=="nvenc"),
        ("ffmpeg-nvenc","NVIDIA NVENC (FFmpeg)",native=="nvenc"&&cfg!(feature="ffmpeg")),
        ("ffmpeg-amf","AMD AMF",native=="ffmpeg-amf"&&cfg!(feature="ffmpeg")),
        ("ffmpeg-qsv","Intel Quick Sync",native=="ffmpeg-qsv"&&cfg!(feature="ffmpeg")),
    ] {
        let result=if matching {Encoder::selected(&dev,luid,Codec::H264,(640,360),60,4000,&Settings{backend:id.into(),..Default::default()}).map(|_|())}
            else {Err(windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),"Not available on this GPU or build"))};
        options.push(Choice{id,name,available:result.is_ok(),reason:result.err().map(|e|e.message()).unwrap_or_default()});
    }
    #[cfg(not(feature="ffmpeg"))]
    options.insert(0,Choice{id:"media-foundation",name:"Media Foundation (legacy)",available:true,reason:String::new()});
    cache.insert(luid,options.clone());options
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn legacy_configs_get_safe_defaults_and_round_trip_explicit_choices() {
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(),Settings::default());
        let s=Settings{backend:"ffmpeg-nvenc".into(),preset:"balanced".into()};
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&s).unwrap()).unwrap(),s);
        assert!(s.validate().is_ok());
        assert!(Settings{backend:"mystery".into(),..Default::default()}.validate().is_err());
        assert!(Settings{preset:"mystery".into(),..Default::default()}.validate().is_err());
    }
}

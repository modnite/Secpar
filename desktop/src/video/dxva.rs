//! Hardware video decoding through Media Foundation's DXVA decoders (H.264, HEVC, AV1).
//!
//! The decoder MFT runs with a Direct3D 11 device manager, so the GPU's fixed-function
//! decoder produces NV12 surfaces. We copy the visible part to a staging texture and hand
//! it to the NV12 shader as a `Picture`. Samples that come back in system memory mean the
//! MFT fell back to software, which is no better than OpenH264, so that counts as failure.
use super::DecodeError;
use crate::picture::Picture;
use secpar_stream::frame::Codec;
use std::mem::ManuallyDrop;
use windows::core::{Interface, GUID};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;

pub fn subtype(codec: Codec) -> GUID {
    match codec {
        Codec::H264 => MFVideoFormat_H264,
        Codec::Hevc => MFVideoFormat_HEVC,
        Codec::Av1 => MFVideoFormat_AV1,
    }
}

/// Media Foundation, started for as long as this lives.
struct Mf;
impl Mf {
    fn start() -> windows::core::Result<Mf> {
        // SAFETY: COM may already be initialised on this thread with another model; that is fine.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_LITE)?;
        }
        Ok(Mf)
    }
}
impl Drop for Mf {
    fn drop(&mut self) {
        // SAFETY: paired with MFStartup.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// The discrete adapter if there is one, else the first hardware adapter.
fn pick_adapter() -> Option<(IDXGIAdapter1, String)> {
    // SAFETY: plain DXGI enumeration.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut best: Option<(IDXGIAdapter1, String, usize)> = None;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string();
            if best.as_ref().is_none_or(|b| desc.DedicatedVideoMemory > b.2) {
                best = Some((adapter, name, desc.DedicatedVideoMemory));
            }
        }
        best.map(|(a, n, _)| (a, n))
    }
}

/// Names of the hardware adapters, for diagnostics.
#[cfg(test)]
pub fn adapter_names() -> Vec<String> {
    // SAFETY: plain DXGI enumeration.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return Vec::new() };
        let mut out = Vec::new();
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            if let Ok(desc) = adapter.GetDesc1() {
                out.push(String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string());
            }
        }
        out
    }
}

struct Staging {
    texture: ID3D11Texture2D,
    size: (u32, u32),
}

struct Mft {
    codec: Codec,
    mft: IMFTransform,
    /// Visible rectangle (x, y, width, height) inside the coded surface, from the output type.
    visible: (u32, u32, u32, u32),
    time: i64,
}

pub struct Hardware {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    manager: IMFDXGIDeviceManager,
    staging: Option<Staging>,
    mft: Option<Mft>,
    pub adapter: String,
    /// Tests: make the next N frames fail as if the data were undecodable.
    #[cfg(test)]
    pub inject_invalid: u32,
    // Declared last: MFShutdown must come after every Media Foundation object is released.
    _mf: Mf,
}

fn pack(w: u32, h: u32) -> u64 {
    ((w as u64) << 32) | h as u64
}

impl Hardware {
    /// Opens the GPU and Media Foundation. The decoder MFT itself is created per codec on
    /// the first frame of that codec.
    pub fn open() -> Result<Self, ()> {
        let mf = Mf::start().map_err(|_| ())?;
        let (adapter, name) = pick_adapter().ok_or(())?;
        // SAFETY: COM/D3D calls with valid out-params.
        unsafe {
            let (mut dev, mut ctx) = (None, None);
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            )
            .map_err(|_| ())?;
            let (dev, ctx) = (dev.ok_or(())?, ctx.ok_or(())?);
            // Media Foundation drives the device from its own worker threads.
            if let Ok(mt) = dev.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            let (mut token, mut manager) = (0, None);
            MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(|_| ())?;
            let manager = manager.ok_or(())?;
            manager.ResetDevice(&dev, token).map_err(|_| ())?;
            Ok(Hardware { dev, ctx, manager, staging: None, mft: None, adapter: name, #[cfg(test)] inject_invalid: 0, _mf: mf })
        }
    }

    /// Creates the decoder for `codec` ahead of the first frame, so the first keyframe is not
    /// held up (and the queue behind it dropped) while Media Foundation sets up.
    pub fn prepare(&mut self, codec: Codec) -> Result<(), ()> {
        if self.mft.as_ref().is_none_or(|m| m.codec != codec) {
            self.mft = None;
            self.mft = Some(self.create(codec).map_err(|_| ())?);
        }
        Ok(())
    }

    /// Decodes one access unit. `Ok(None)` while the decoder is still waiting for input.
    pub fn decode(&mut self, codec: Codec, bytes: &[u8]) -> Result<Option<Picture>, DecodeError> {
        if self.mft.as_ref().is_none_or(|m| m.codec != codec) {
            self.mft = None;
            self.mft = Some(self.create(codec).map_err(|_| DecodeError::Device)?);
        }
        #[allow(unused_mut)]
        let mut result = self.run(bytes);
        #[cfg(test)]
        if self.inject_invalid > 0 {
            self.inject_invalid -= 1;
            result = Err(DecodeError::Invalid);
        }
        if result.is_err() {
            // SAFETY: valid device; a removed or reset device cannot be recovered here.
            if unsafe { self.dev.GetDeviceRemovedReason() }.is_err() {
                return Err(DecodeError::Device);
            }
            // Bad data: drop everything inside the decoder and wait for the next keyframe.
            if let Some(m) = &self.mft {
                // SAFETY: valid MFT.
                unsafe {
                    let _ = m.mft.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
                }
            }
        }
        result
    }

    fn create(&mut self, codec: Codec) -> windows::core::Result<Mft> {
        // SAFETY: MFT enumeration; the returned CoTaskMem array is freed after taking each item.
        unsafe {
            let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype(codec) };
            let (mut list, mut n) = (std::ptr::null_mut(), 0u32);
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
                Some(&input),
                None,
                &mut list,
                &mut n,
            )?;
            if list.is_null() {
                return Err(MF_E_TOPO_CODEC_NOT_FOUND.into());
            }
            let activates = std::slice::from_raw_parts_mut(list, n as usize);
            let mut found = None;
            for a in activates.iter_mut() {
                if found.is_some() {
                    break;
                }
                let Some(a) = a.take() else { continue };
                if let Ok(mft) = a.ActivateObject::<IMFTransform>() {
                    if self.configure(&mft, codec).is_ok() {
                        found = Some(mft);
                    }
                }
            }
            activates.iter_mut().for_each(|a| drop(a.take()));
            CoTaskMemFree(Some(list as _));
            let mft = found.ok_or_else(|| windows::core::Error::from(MF_E_TOPO_CODEC_NOT_FOUND))?;
            let visible = Self::visible_area(&mft).unwrap_or((0, 0, 0, 0));
            Ok(Mft { codec, mft, visible, time: 0 })
        }
    }

    /// Everything that must hold for the MFT to decode on the GPU with low latency.
    fn configure(&self, mft: &IMFTransform, codec: Codec) -> windows::core::Result<()> {
        // SAFETY: MF calls on a live MFT and device manager.
        unsafe {
            let attrs = mft.GetAttributes()?;
            if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0 {
                return Err(MF_E_UNSUPPORTED_REPRESENTATION.into());
            }
            if attrs.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) == 0 {
                return Err(MF_E_UNSUPPORTED_D3D_TYPE.into());
            }
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            let _ = attrs.SetUINT32(&MF_SA_D3D11_BINDFLAGS, D3D11_BIND_SHADER_RESOURCE.0 as u32);
            mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, self.manager.as_raw() as usize)?;
            if let Ok(api) = mft.cast::<ICodecAPI>() {
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &subtype(codec))?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, pack(1920, 1080))?;
            input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            mft.SetInputType(0, &input, 0)?;
            Self::choose_nv12(mft)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        Ok(())
    }

    fn choose_nv12(mft: &IMFTransform) -> windows::core::Result<()> {
        // SAFETY: MF calls on a live MFT.
        unsafe {
            let mut i = 0;
            loop {
                let t = mft.GetOutputAvailableType(0, i)?;
                i += 1;
                if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                    mft.SetOutputType(0, &t, 0)?;
                    return Ok(());
                }
            }
        }
    }

    /// The part of the coded surface to show. Coded sizes are multiples of 16 (1088 for
    /// 1080p), and the real picture is described by the display aperture.
    fn visible_area(mft: &IMFTransform) -> Option<(u32, u32, u32, u32)> {
        // SAFETY: MF calls on a live MFT; the blob is read into a buffer of its own size.
        unsafe {
            let t = mft.GetOutputCurrentType(0).ok()?;
            for key in [&MF_MT_MINIMUM_DISPLAY_APERTURE, &MF_MT_GEOMETRIC_APERTURE, &MF_MT_PAN_SCAN_APERTURE] {
                let mut blob = [0u8; 16];
                if t.GetBlob(key, &mut blob, None).is_ok() {
                    if let Some(area) = parse_aperture(&blob) {
                        return Some(area);
                    }
                }
            }
            let size = t.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
            Some((0, 0, (size >> 32) as u32, size as u32))
        }
    }

    fn run(&mut self, bytes: &[u8]) -> Result<Option<Picture>, DecodeError> {
        let m = self.mft.as_mut().ok_or(DecodeError::Device)?;
        // SAFETY: sample and buffer are created, filled and handed to the MFT within this call.
        let sample = unsafe {
            (|| -> windows::core::Result<IMFSample> {
                let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
                let (mut p, mut max) = (std::ptr::null_mut(), 0u32);
                buffer.Lock(&mut p, Some(&mut max), None)?;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len().min(max as usize));
                buffer.Unlock()?;
                buffer.SetCurrentLength(bytes.len() as u32)?;
                let sample = MFCreateSample()?;
                sample.AddBuffer(&buffer)?;
                sample.SetSampleTime(m.time)?;
                sample.SetSampleDuration(166_666)?;
                m.time += 166_666;
                Ok(sample)
            })()
            .map_err(|_| DecodeError::Invalid)?
        };
        let mut latest = None;
        // SAFETY: valid MFT.
        let mut fed = unsafe { m.mft.ProcessInput(0, &sample, 0) };
        // An unlikely "not accepting" means output is waiting: drain it, then feed again.
        if matches!(&fed, Err(e) if e.code() == MF_E_NOTACCEPTING) {
            self.drain(&mut latest)?;
            let m = self.mft.as_mut().ok_or(DecodeError::Device)?;
            // SAFETY: valid MFT.
            fed = unsafe { m.mft.ProcessInput(0, &sample, 0) };
        }
        fed.map_err(|_| DecodeError::Invalid)?;
        self.drain(&mut latest)?;
        Ok(latest)
    }

    /// Takes every ready output, keeping only the newest picture.
    fn drain(&mut self, latest: &mut Option<Picture>) -> Result<(), DecodeError> {
        for _ in 0..8 {
            let m = self.mft.as_mut().ok_or(DecodeError::Device)?;
            // SAFETY: the output sample taken back out of ManuallyDrop is released exactly once.
            let (result, sample) = unsafe {
                let info = m.mft.GetOutputStreamInfo(0).map_err(|_| DecodeError::Invalid)?;
                let provides = info.dwFlags
                    & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32
                    != 0;
                // Without sample provision the MFT writes system memory, i.e. no GPU decode.
                if !provides {
                    return Err(DecodeError::Device);
                }
                let mut db = [MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    pSample: ManuallyDrop::new(None),
                    dwStatus: 0,
                    pEvents: ManuallyDrop::new(None),
                }];
                let mut status = 0;
                let r = m.mft.ProcessOutput(0, &mut db, &mut status);
                let sample = ManuallyDrop::take(&mut db[0].pSample);
                drop(ManuallyDrop::take(&mut db[0].pEvents));
                (r, sample)
            };
            match result {
                Ok(()) => {
                    if let Some(sample) = sample {
                        *latest = Some(self.picture(&sample)?);
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // A new SPS (for example a resolution change): take the new NV12 type.
                    Self::choose_nv12(&m.mft).map_err(|_| DecodeError::Invalid)?;
                    m.visible = Self::visible_area(&m.mft).unwrap_or((0, 0, 0, 0));
                }
                Err(_) => return Err(DecodeError::Invalid),
            }
        }
        Ok(())
    }

    /// Copies the visible part of a decoded surface into a `Picture`.
    fn picture(&mut self, sample: &IMFSample) -> Result<Picture, DecodeError> {
        let visible = self.mft.as_ref().map_or((0, 0, 0, 0), |m| m.visible);
        // SAFETY: the surface belongs to our device; the staging map is released before return.
        unsafe {
            let buffer = sample.GetBufferByIndex(0).map_err(|_| DecodeError::Invalid)?;
            let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() else {
                return Err(DecodeError::Device);
            };
            let mut raw = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw).map_err(|_| DecodeError::Invalid)?;
            if raw.is_null() {
                return Err(DecodeError::Invalid);
            }
            let texture = ID3D11Texture2D::from_raw(raw);
            let index = dxgi.GetSubresourceIndex().map_err(|_| DecodeError::Invalid)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let (x, y, w, h) = fit_area(visible, (desc.Width, desc.Height));
            if self.staging.as_ref().is_none_or(|s| s.size != (desc.Width, desc.Height)) {
                let d = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                self.dev.CreateTexture2D(&d, None, Some(&mut staging)).map_err(|_| DecodeError::Device)?;
                self.staging = Some(Staging { texture: staging.ok_or(DecodeError::Device)?, size: (desc.Width, desc.Height) });
            }
            let staging = &self.staging.as_ref().unwrap().texture;
            self.ctx.CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, index, None);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(|_| DecodeError::Device)?;
            let pitch = map.RowPitch as usize;
            // NV12 staging layout: luma rows, then interleaved chroma rows at the same pitch.
            let total = pitch * desc.Height as usize * 3 / 2;
            let data = std::slice::from_raw_parts(map.pData as *const u8, total);
            let luma = &data[y as usize * pitch + x as usize..pitch * desc.Height as usize];
            let chroma = &data[pitch * desc.Height as usize + (y as usize / 2) * pitch + x as usize..];
            let picture = Picture::from_nv12([w as usize, h as usize], (luma, pitch), (chroma, pitch));
            self.ctx.Unmap(staging, 0);
            picture.ok_or(DecodeError::Dimensions)
        }
    }
}

/// MFVideoArea: two MFOffset (16-bit fraction, 16-bit value) and a width and height.
fn parse_aperture(blob: &[u8; 16]) -> Option<(u32, u32, u32, u32)> {
    let int = |at: usize| i16::from_le_bytes([blob[at], blob[at + 1]]);
    let (x, y) = (int(2), int(6));
    let (w, h) = (i32::from_le_bytes(blob[8..12].try_into().ok()?), i32::from_le_bytes(blob[12..16].try_into().ok()?));
    (x >= 0 && y >= 0 && w > 0 && h > 0).then_some((x as u32, y as u32, w as u32, h as u32))
}

/// The visible rectangle clamped into the surface; odd offsets are dropped so chroma stays aligned.
fn fit_area(area: (u32, u32, u32, u32), surface: (u32, u32)) -> (u32, u32, u32, u32) {
    let (x, y, w, h) = area;
    if w == 0 || h == 0 || x % 2 != 0 || y % 2 != 0 || x + w > surface.0 || y + h > surface.1 {
        return (0, 0, w.min(surface.0).max(if w == 0 { surface.0 } else { 0 }), h.min(surface.1).max(if h == 0 { surface.1 } else { 0 }));
    }
    (x, y, w, h)
}

impl Drop for Hardware {
    fn drop(&mut self) {
        if let Some(m) = self.mft.take() {
            // SAFETY: valid MFT; releases the D3D manager reference before the device goes.
            unsafe {
                let _ = m.mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
                let _ = m.mft.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
                let _ = m.mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0);
            }
        }
    }
}

/// True when this PC has a working GPU decoder for `codec`: the MFT must produce a real
/// frame through a Direct3D surface from `sample`, an access unit that starts a stream.
pub fn probe(codec: Codec, sample: &[u8]) -> bool {
    let Ok(mut hw) = Hardware::open() else { return false };
    for _ in 0..4 {
        match hw.decode(codec, sample) {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blob(x: i16, y: i16, w: i32, h: i32) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[2..4].copy_from_slice(&x.to_le_bytes());
        b[6..8].copy_from_slice(&y.to_le_bytes());
        b[8..12].copy_from_slice(&w.to_le_bytes());
        b[12..16].copy_from_slice(&h.to_le_bytes());
        b
    }
    #[test]
    fn aperture_blob_is_read_in_pixels() {
        assert_eq!(parse_aperture(&blob(0, 0, 1920, 1080)), Some((0, 0, 1920, 1080)));
        assert_eq!(parse_aperture(&blob(2, 4, 854, 480)), Some((2, 4, 854, 480)));
        assert_eq!(parse_aperture(&blob(0, 0, 0, 1080)), None);
        assert_eq!(parse_aperture(&blob(-2, 0, 100, 100)), None);
    }
    #[test]
    fn visible_area_never_leaves_the_surface() {
        assert_eq!(fit_area((0, 0, 1920, 1080), (1920, 1088)), (0, 0, 1920, 1080));
        assert_eq!(fit_area((0, 0, 854, 480), (864, 480)), (0, 0, 854, 480));
        // Larger than the surface or an odd offset: show the surface, never read past it.
        assert_eq!(fit_area((0, 0, 4000, 4000), (1920, 1088)), (0, 0, 1920, 1088));
        assert_eq!(fit_area((1, 0, 100, 100), (1920, 1088)), (0, 0, 100, 100));
        assert_eq!(fit_area((0, 0, 0, 0), (640, 360)), (0, 0, 640, 360));
    }
}

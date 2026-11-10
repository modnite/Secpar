//! Desktop capture of the primary display via DXGI Desktop Duplication, re-opened after
//! access-lost (mode change, rotation, UAC/lock screen, full-screen game switching modes).

use super::qpc_to_us;
use std::time::Duration;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use std::sync::Arc;
mod shared;
pub use shared::SharedCapture;

/// Lost the duplication: re-open it.
#[derive(Debug)]
pub struct Lost;

pub trait Duplicator {
    type Frame;
    /// `Ok(None)` on timeout (nothing changed).
    fn acquire(&mut self, timeout_ms: u32) -> Result<Option<Self::Frame>, Lost>;
    fn size(&self) -> (u32, u32);
}

/// A duplicator that re-opens itself. `reinit` is set whenever a new duplication was opened
/// (the first one too): the caller rebuilds size-dependent state and sends a keyframe.
pub struct Capture<D> {
    open: Box<dyn FnMut() -> Option<D>>,
    dup: Option<D>,
    pub reinit: bool,
}

impl<D: Duplicator> Capture<D> {
    /// `first`: an already opened duplication, if any.
    pub fn new(first: Option<D>, open: impl FnMut() -> Option<D> + 'static) -> Self {
        Capture { open: Box::new(open), reinit: first.is_some(), dup: first }
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.dup.as_ref().map(|d| d.size())
    }

    /// The next changed frame, or None (timeout, or no duplication available right now).
    pub fn next(&mut self, timeout: Duration) -> Option<D::Frame> {
        for _ in 0..2 {
            if self.dup.is_none() {
                self.dup = (self.open)();
                if self.dup.is_none() {
                    // secure desktop or no display: try again later, never in a tight loop
                    std::thread::sleep(timeout.max(Duration::from_millis(200)));
                    return None;
                }
                self.reinit = true;
            }
            match self.dup.as_mut()?.acquire(timeout.as_millis() as u32) {
                Ok(f) => return f,
                Err(Lost) => self.dup = None,
            }
        }
        None
    }
}

#[derive(Clone)]
pub struct Pointer {
    pub x: i32,
    pub y: i32,
    pub visible: bool,
}

#[derive(Clone)]
pub struct Shape {
    pub typ: u32,
    pub w: u32,
    pub h: u32,
    pub pitch: u32,
    pub hx: i32,
    pub hy: i32,
    pub data: Vec<u8>,
}

pub struct GpuFrame {
    /// A leased image. The shared producer cannot overwrite it while a viewer
    /// still holds this Arc. None when only the pointer changed.
    pub tex: Option<Arc<FrameTexture>>,
    pub capture_us: u64,
    pub pointer: Option<Pointer>,
    pub shape: Option<Shape>,
}
pub struct FrameTexture(pub ID3D11Texture2D, Option<SharedHandle>);
struct SharedHandle(windows::Win32::Foundation::HANDLE);
impl Drop for SharedHandle {
    fn drop(&mut self) {
        // SAFETY: the NT handle is owned once; imported resources hold their own references.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}
pub(crate) fn isolated_gpu() -> bool {
    std::env::var("SECPAR_GPU_ISOLATION").as_deref() == Ok("1")
}
impl FrameTexture {
    fn new(texture: ID3D11Texture2D) -> windows::core::Result<Self> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: live texture, writable descriptor.
        unsafe { texture.GetDesc(&mut desc) };
        let handle = if desc.MiscFlags & D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 as u32 != 0 {
            let resource: IDXGIResource1 = texture.cast()?;
            // SAFETY: unnamed handle, local process only; closed by SharedHandle.
            Some(SharedHandle(unsafe { resource.CreateSharedHandle(None,
                DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0, windows::core::PCWSTR::null())? }))
        } else { None };
        Ok(Self(texture, handle))
    }
    pub(crate) fn shared_handle(&self) -> Option<windows::Win32::Foundation::HANDLE> {
        self.1.as_ref().map(|handle| handle.0)
    }
    fn access(&self) -> windows::core::Result<Option<TextureAccess>> {
        if self.1.is_some() { TextureAccess::acquire(&self.0, secpar_stream::timing::Stage::CaptureTextureWait).map(Some) } else { Ok(None) }
    }
}
/// Sunshine's shared capture/encoder texture mutex uses key zero on both devices.
pub(crate) struct TextureAccess(IDXGIKeyedMutex);
impl TextureAccess {
    pub(crate) fn lock(texture: &ID3D11Texture2D) -> windows::core::Result<Self> {
        Self::acquire(texture, secpar_stream::timing::Stage::EncoderTextureWait)
    }
    fn acquire(texture: &ID3D11Texture2D, stage: secpar_stream::timing::Stage) -> windows::core::Result<Self> {
        let mutex: IDXGIKeyedMutex = texture.cast()?;
        // SAFETY: live keyed mutex. Inspect S_OK explicitly: WAIT_TIMEOUT is positive
        // and must not be mistaken for ownership by HRESULT::ok().
        let timing = secpar_stream::timing::begin();
        let status = unsafe { (Interface::vtable(&mutex).AcquireSync)(mutex.as_raw(), 0, 500) };
        secpar_stream::timing::elapsed(stage, timing);
        if status.0 != 0 {
            return Err(windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),
                format!("Shared capture texture wait failed ({status:?})")));
        }
        Ok(Self(mutex))
    }
}
impl Drop for TextureAccess {
    fn drop(&mut self) {
        // SAFETY: constructed only after S_OK; balances acquisition on this device.
        let _ = unsafe { self.0.ReleaseSync(0) };
    }
}
impl std::ops::Deref for FrameTexture {
    type Target = ID3D11Texture2D;
    fn deref(&self) -> &Self::Target { &self.0 }
}
// SAFETY: D3D11 resources are free-threaded. Leased textures are immutable;
// all GPU commands use their device's multithread-protected immediate context.
unsafe impl Send for FrameTexture {}
unsafe impl Sync for FrameTexture {}

pub use secpar_host_core::Display;

fn outputs() -> Vec<(IDXGIAdapter1, IDXGIOutput)> {
    // SAFETY: COM enumeration with valid out-params.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return vec![] };
        (0..)
            .map_while(|i| factory.EnumAdapters1(i).ok())
            .flat_map(|a| (0..).map_while(move |j| a.EnumOutputs(j).ok().map(|o| (a.clone(), o))))
            .collect()
    }
}

pub fn describe(o: &IDXGIOutput) -> Option<Display> {
    // SAFETY: GetDesc fills a plain struct.
    let d = unsafe { o.GetDesc().ok()? };
    let n = d.DeviceName.iter().position(|c| *c == 0).unwrap_or(d.DeviceName.len());
    let id = String::from_utf16_lossy(&d.DeviceName[..n]);
    let r = d.DesktopCoordinates;
    let label = id.trim_start_matches(r"\\.\").replace("DISPLAY", "Display ");
    Some(Display { name: format!("{label} ({}×{})", r.right - r.left, r.bottom - r.top), primary: r.left == 0 && r.top == 0, rect: [r.left, r.top, r.right, r.bottom], id })
}

/// Every display attached to the desktop.
pub fn displays() -> Vec<Display> {
    outputs().iter().filter_map(|(_, o)| describe(o)).collect()
}

/// The adapter that drives `display` (a device name; empty or unknown = the primary display),
/// and a D3D11 device on it for capture, conversion and encoding.
pub fn device(display: &str) -> Option<(ID3D11Device, ID3D11DeviceContext, IDXGIOutput1, i64)> {
    let all = outputs();
    let pick = |want: &dyn Fn(&Display) -> bool| all.iter().find(|(_, o)| describe(o).is_some_and(|d| want(&d))).cloned();
    let (adapter, output) = pick(&|d| !display.is_empty() && d.id == display).or_else(|| pick(&|d| d.primary))?;
    // SAFETY: COM/D3D calls with valid out-params.
    unsafe {
        let (mut dev, mut ctx) = (None, None);
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            Some(&mut ctx),
        )
        .ok()?;
        let (dev, ctx): (ID3D11Device, ID3D11DeviceContext) = (dev?, ctx?);
        // Media Foundation uses the device from its own threads.
        if let Ok(mt) = dev.cast::<ID3D11Multithread>() {
            let _ = mt.SetMultithreadProtected(true);
        }
        let luid = adapter.GetDesc1().ok()?.AdapterLuid;
        let luid = ((luid.HighPart as i64) << 32) | luid.LowPart as i64;
        Some((dev, ctx, output.cast().ok()?, luid))
    }
}

pub struct Dxgi {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    w: u32,
    h: u32,
    copy: Option<Arc<FrameTexture>>,
}

impl Dxgi {
    pub fn open(dev: &ID3D11Device, ctx: &ID3D11DeviceContext, output: &IDXGIOutput1) -> Option<Dxgi> {
        // the lock screen and UAC prompts are another desktop: duplicate the one being shown
        crate::win::desk::follow();
        // SAFETY: D3D calls on live objects.
        unsafe {
            let dup = output.DuplicateOutput(dev).ok()?;
            let d = dup.GetDesc();
            Some(Dxgi { dev: dev.clone(), ctx: ctx.clone(), dup, w: d.ModeDesc.Width, h: d.ModeDesc.Height, copy: None })
        }
    }
}

impl Duplicator for Dxgi {
    type Frame = GpuFrame;

    fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    fn acquire(&mut self, timeout_ms: u32) -> Result<Option<GpuFrame>, Lost> {
        // SAFETY: AcquireNextFrame/ReleaseFrame are paired; the shape buffer is sized by DXGI.
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut res = None;
            if let Err(e) = self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut res) {
                return if e.code() == DXGI_ERROR_WAIT_TIMEOUT { Ok(None) } else { Err(Lost) };
            }
            let mut f = GpuFrame { tex: None, capture_us: super::now_us(), pointer: None, shape: None };
            if info.LastPresentTime != 0 {
                let src: Option<ID3D11Texture2D> = res.and_then(|r| r.cast().ok());
                if let Some(src) = src {
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    src.GetDesc(&mut desc);
                    if (desc.Width, desc.Height) != (self.w, self.h) {
                        let _ = self.dup.ReleaseFrame();
                        return Err(Lost); // mode change
                    }
                    if self.copy.is_none() {
                        desc.Usage = D3D11_USAGE_DEFAULT;
                        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32 | D3D11_BIND_RENDER_TARGET.0 as u32;
                        desc.CPUAccessFlags = 0;
                        desc.MiscFlags = if isolated_gpu() {
                            (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32
                        } else { 0 };
                        let mut t = None;
                        if self.dev.CreateTexture2D(&desc, None, Some(&mut t)).is_err() {
                            let _ = self.dup.ReleaseFrame();
                            return Err(Lost);
                        }
                        self.copy = t.and_then(|texture| FrameTexture::new(texture).ok()).map(Arc::new);
                        if self.copy.is_none() {
                            let _ = self.dup.ReleaseFrame();
                            return Err(Lost);
                        }
                    }
                    let copy = self.copy.clone().expect("created above");
                    {
                        let _access = match copy.access() {
                            Ok(access) => access,
                            Err(_) => { let _ = self.dup.ReleaseFrame(); return Err(Lost); }
                        };
                        self.ctx.CopyResource(&copy.0, &src);
                    }
                    f.tex = Some(copy);
                    f.capture_us = qpc_to_us(info.LastPresentTime);
                }
            }
            if info.LastMouseUpdateTime != 0 {
                let p = info.PointerPosition;
                f.pointer = Some(Pointer { x: p.Position.x, y: p.Position.y, visible: p.Visible.as_bool() });
            }
            if info.PointerShapeBufferSize > 0 {
                let mut data = vec![0u8; info.PointerShapeBufferSize as usize];
                let (mut need, mut si) = (0, DXGI_OUTDUPL_POINTER_SHAPE_INFO::default());
                if self.dup.GetFramePointerShape(data.len() as u32, data.as_mut_ptr().cast(), &mut need, &mut si).is_ok() {
                    f.shape = Some(Shape {
                        typ: si.Type,
                        w: si.Width,
                        h: si.Height,
                        pitch: si.Pitch,
                        hx: si.HotSpot.x,
                        hy: si.HotSpot.y,
                        data,
                    });
                }
            }
            let _ = self.dup.ReleaseFrame();
            Ok(Some(f))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Yields `frames` frames, then reports access lost (a mode change).
    struct Fake {
        size: (u32, u32),
        frames: u32,
    }

    impl Duplicator for Fake {
        type Frame = (u32, u32);
        fn acquire(&mut self, _: u32) -> Result<Option<(u32, u32)>, Lost> {
            if self.frames == 0 {
                return Err(Lost);
            }
            self.frames -= 1;
            Ok(Some(self.size))
        }
        fn size(&self) -> (u32, u32) {
            self.size
        }
    }

    #[test]
    fn reinit_on_mode_change() {
        let opens = Arc::new(AtomicU32::new(0));
        let o = opens.clone();
        let mut cap = Capture::new(None, move || {
            let n = o.fetch_add(1, Ordering::SeqCst);
            // the "display" switches from 1920x1080 to 1280x720 after the first duplication
            Some(Fake { size: if n == 0 { (1920, 1080) } else { (1280, 720) }, frames: 2 })
        });
        assert_eq!(cap.next(Duration::ZERO), Some((1920, 1080)));
        assert!(std::mem::take(&mut cap.reinit), "first open counts as a (re)init");
        assert_eq!(cap.next(Duration::ZERO), Some((1920, 1080)));
        assert!(!cap.reinit);
        // access lost → re-opened in the same call, new size, flagged for a keyframe
        assert_eq!(cap.next(Duration::ZERO), Some((1280, 720)));
        assert!(cap.reinit);
        assert_eq!(cap.size(), Some((1280, 720)));
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn no_display_returns_none_and_retries() {
        let opens = Arc::new(AtomicU32::new(0));
        let o = opens.clone();
        let mut cap = Capture::<Fake>::new(None, move || {
            o.fetch_add(1, Ordering::SeqCst);
            None
        });
        assert!(cap.next(Duration::ZERO).is_none());
        assert!(cap.next(Duration::ZERO).is_none());
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    fn shared_test_texture(dev: &ID3D11Device) -> FrameTexture {
        use windows::Win32::Graphics::Dxgi::Common::*;
        let pixels: Vec<u8> = (0..64*64).flat_map(|_| [40,60,200,255]).collect();
        let desc = D3D11_TEXTURE2D_DESC { Width:64, Height:64, MipLevels:1, ArraySize:1,
            Format:DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc:DXGI_SAMPLE_DESC {Count:1,Quality:0},
            Usage:D3D11_USAGE_DEFAULT, BindFlags:(D3D11_BIND_RENDER_TARGET.0|D3D11_BIND_SHADER_RESOURCE.0) as u32,
            MiscFlags:(D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0|D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32,
            ..Default::default() };
        let data = D3D11_SUBRESOURCE_DATA {pSysMem:pixels.as_ptr().cast(),SysMemPitch:256,..Default::default()};
        let mut texture=None;
        // SAFETY: initialized pixels outlive CreateTexture2D; GPU owns its copy.
        unsafe {dev.CreateTexture2D(&desc,Some(&data),Some(&mut texture)).unwrap()};
        FrameTexture::new(texture.unwrap()).unwrap()
    }

    #[test]
    fn shared_texture_timeout_does_not_grant_foreign_device_ownership() {
        let (a,_,_,_) = device("").expect("D3D11 device for shared texture test");
        let (b,_,_,_) = device("").expect("second D3D11 device");
        let source=shared_test_texture(&a);
        let other:ID3D11Device1=b.cast().unwrap();
        // SAFETY: source retains the NT handle and texture.
        let imported:ID3D11Texture2D=unsafe {other.OpenSharedResource1(source.shared_handle().unwrap()).unwrap()};
        let held=TextureAccess::lock(&source.0).unwrap();
        assert!(TextureAccess::lock(&imported).is_err(),"timeout is not successful ownership");
        drop(held);
        drop(TextureAccess::lock(&imported).expect("acquires after producer releases"));
        drop(TextureAccess::lock(&source.0).expect("producer can reuse after consumer releases"));
    }

    #[test]
    fn shared_capture_converts_correctly_on_an_independent_encoder_device() {
        let (a,source_ctx,_,_) = device("").expect("capture D3D11 device");
        let (b,ctx,_,_) = device("").expect("encoder D3D11 device");
        let source=shared_test_texture(&a);
        {
            let _write=TextureAccess::lock(&source.0).unwrap();
            // SAFETY: submit initialization before handing off the shared resource.
            unsafe {source_ctx.Flush()};
        }
        let mut converter=crate::engine::convert::Convert::new(&b,&ctx,(64,64),(64,64)).unwrap();
        let nv12=converter.convert_frame(&source).unwrap();
        // SAFETY: staging copy and paired map/unmap on the encoder device.
        let sample=unsafe {
            let mut desc=D3D11_TEXTURE2D_DESC::default();nv12.GetDesc(&mut desc);
            desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;
            desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ.0 as u32;desc.MiscFlags=0;
            let mut staging=None;b.CreateTexture2D(&desc,None,Some(&mut staging)).unwrap();
            let staging=staging.unwrap();ctx.CopyResource(&staging,&nv12);
            let mut map=D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&staging,0,D3D11_MAP_READ,0,Some(&mut map)).unwrap();
            let pixels=map.pData.cast::<u8>();let pitch=map.RowPitch as usize;
            let sample=(*pixels.add(32*pitch+32),*pixels.add(80*pitch+32),*pixels.add(80*pitch+33));
            ctx.Unmap(&staging,0);sample
        };
        assert!((sample.0 as i32-92).abs()<=3,"BT709 luma: {sample:?}");
        assert!((sample.1 as i32-105).abs()<=3,"BT709 blue chroma: {sample:?}");
        assert!((sample.2 as i32-190).abs()<=3,"BT709 red chroma: {sample:?}");
        // Source remains reusable after the encoder's GPU reads.
        drop(TextureAccess::lock(&source.0).unwrap());
    }

}

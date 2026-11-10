//! BGRA desktop → NV12 (BT.709, limited range) with scaling, on the GPU's video processor.

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

/// Output textures in flight: the async encoder may still read one while the next is written.
const RING: usize = 3;

/// Holds the device's immediate-context lock. One conversion is a sequence of state calls and a
/// draw. Every stream shares one device and one immediate context, and the multithread
/// protection only makes single calls atomic. Without this guard two streams interleave their
/// bindings: one stream's render target or shader lands on the other's draw. The planes come out
/// half written (zero chroma is green) on both clients.
pub struct GpuGuard(Option<ID3D11Multithread>);

#[cfg(test)]
thread_local!(static HELD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });

impl GpuGuard {
    /// Test-only: does this thread hold the device lock? State-setting code asserts it.
    #[cfg(test)]
    pub fn held() -> bool {
        HELD.with(|h| h.get() > 0)
    }

    pub fn lock(dev: &ID3D11Device) -> GpuGuard {
        let mt = dev.cast::<ID3D11Multithread>().ok();
        if let Some(mt) = &mt {
            // SAFETY: balanced by Leave in Drop; the lock is recursive per thread.
            unsafe { mt.Enter() };
        }
        #[cfg(test)]
        HELD.with(|h| h.set(h.get() + 1));
        GpuGuard(mt)
    }
}

impl Drop for GpuGuard {
    fn drop(&mut self) {
        #[cfg(test)]
        HELD.with(|h| h.set(h.get() - 1));
        if let Some(mt) = &self.0 {
            // SAFETY: pairs with the Enter in lock.
            unsafe { mt.Leave() };
        }
    }
}

pub struct Convert {
    dev: ID3D11Device,
    /// Filtered-chroma shader passes; the video processor below is the fallback.
    ps: Option<super::convert_ps::Passes>,
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext1,
    en: ID3D11VideoProcessorEnumerator,
    vp: ID3D11VideoProcessor,
    out: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    next: usize,
    input: Option<(usize, ID3D11VideoProcessorInputView)>,
    /// Per-encoder imports of immutable, leased capture textures.
    imports: Vec<(usize, ID3D11Texture2D)>,
}

impl Convert {
    pub fn new(dev: &ID3D11Device, ctx: &ID3D11DeviceContext, (iw, ih): (u32, u32), (ow, oh): (u32, u32)) -> windows::core::Result<Convert> {
        // SAFETY: D3D11 video calls on live objects with initialised descriptors.
        unsafe {
            let vdev: ID3D11VideoDevice = dev.cast()?;
            let vctx: ID3D11VideoContext1 = ctx.cast()?;
            let rate = DXGI_RATIONAL { Numerator: 60, Denominator: 1 };
            let cd = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: iw,
                InputHeight: ih,
                OutputFrameRate: rate,
                OutputWidth: ow,
                OutputHeight: oh,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let en = vdev.CreateVideoProcessorEnumerator(&cd)?;
            let vp = vdev.CreateVideoProcessor(&en, 0)?;
            vctx.VideoProcessorSetStreamAutoProcessingMode(&vp, 0, false);
            vctx.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            vctx.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
            vctx.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
            let desc = D3D11_TEXTURE2D_DESC {
                Width: ow,
                Height: oh,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_VIDEO_ENCODER.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut out = Vec::with_capacity(RING);
            for _ in 0..RING {
                let mut t = None;
                dev.CreateTexture2D(&desc, None, Some(&mut t))?;
                let t = t.expect("texture");
                let vd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC { ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D, ..Default::default() };
                let mut v = None;
                vdev.CreateVideoProcessorOutputView(&t, &en, &vd, Some(&mut v))?;
                out.push((t, v.expect("view")));
            }
            let ps = if std::env::var_os("SECPAR_CONVERT_VP").is_some() {
                None
            } else {
                let textures: Vec<_> = out.iter().map(|(t, _)| t.clone()).collect();
                super::convert_ps::Passes::new(dev, ctx, (iw, ih), (ow, oh), &textures).ok()
            };
            Ok(Convert { dev: dev.clone(), ps, vdev, vctx, en, vp, out, next: 0, input: None, imports: Vec::new() })
        }
    }

    pub fn convert_frame(&mut self, src: &super::capture::FrameTexture) -> windows::core::Result<ID3D11Texture2D> {
        let Some(handle) = src.shared_handle() else { return self.convert(&src.0) };
        let key = src.0.as_raw() as usize;
        let texture = match self.imports.iter().find(|(id, _)| *id == key) {
            Some((_, texture)) => texture.clone(),
            None => {
                let device: ID3D11Device1 = self.dev.cast()?;
                // SAFETY: source owns the NT handle throughout this call. The imported
                // resource stays valid independently of the transport handle.
                let texture: ID3D11Texture2D = unsafe { device.OpenSharedResource1(handle)? };
                if self.imports.len() == 8 { self.imports.remove(0); }
                self.imports.push((key, texture.clone()));
                texture
            }
        };
        // Keep GPU reads ordered against capture writes, independently of the
        // immediate-context locks on the two D3D devices.
        let _access = super::capture::TextureAccess::lock(&texture)?;
        self.convert(&texture)
    }

    /// Converts `src` (BGRA, the input size) into the next NV12 texture of the ring.
    pub fn convert(&mut self, src: &ID3D11Texture2D) -> windows::core::Result<ID3D11Texture2D> {
        let _gpu = GpuGuard::lock(&self.dev);
        if let Some(ps) = self.ps.as_mut() {
            let idx = self.next;
            if ps.run(&self.dev, src, idx).is_ok() {
                self.next = (idx + 1) % RING;
                return Ok(self.out[idx].0.clone());
            }
            self.ps = None; // this GPU can't do it: fall back to the video processor for good
        }
        // SAFETY: as above; the stream struct's view reference is released after the Blt.
        unsafe {
            let key = src.as_raw() as usize;
            if self.input.as_ref().is_none_or(|(k, _)| *k != key) {
                let d = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC { ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D, ..Default::default() };
                let mut v = None;
                self.vdev.CreateVideoProcessorInputView(src, &self.en, &d, Some(&mut v))?;
                self.input = Some((key, v.expect("view")));
            }
            let (tex, view) = &self.out[self.next];
            self.next = (self.next + 1) % RING;
            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(self.input.as_ref().expect("set").1.clone())),
                ..Default::default()
            };
            let mut streams = [stream];
            let r = self.vctx.VideoProcessorBlt(&self.vp, view, 0, &streams);
            std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface);
            r?;
            Ok(tex.clone())
        }
    }
}

/// The stream size for a `want` request (0×0 = native): fits inside it, keeps the display's
/// aspect ratio, never upscales, even dimensions (NV12).
pub fn fit((dw, dh): (u32, u32), (ww, wh): (u32, u32)) -> (u32, u32) {
    let (ww, wh) = if ww == 0 || wh == 0 { (dw, dh) } else { (ww.min(dw), wh.min(dh)) };
    let s = (ww as f64 / dw as f64).min(wh as f64 / dh as f64);
    let even = |v: f64| ((v.round() as u32) & !1).max(2);
    (even(dw as f64 * s), even(dh as f64 * s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fit_keeps_aspect_and_is_even() {
        assert_eq!(fit((2560, 1440), (0, 0)), (2560, 1440));
        assert_eq!(fit((2560, 1440), (1920, 1080)), (1920, 1080));
        assert_eq!(fit((2560, 1600), (1920, 1080)), (1728, 1080));
        assert_eq!(fit((1920, 1080), (2400, 1080)), (1920, 1080), "never upscales");
        assert_eq!(fit((1366, 768), (1280, 720)), (1280, 720));
        assert_eq!(fit((1080, 1920), (1920, 1080)), (608, 1080), "portrait display");
    }

    #[test]
    fn converts_on_this_gpu() {
        let Some((dev, ctx, _, _)) = crate::engine::capture::device("") else { return eprintln!("no D3D11 device") };
        // SAFETY: test-only texture creation.
        let src = unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 640,
                Height: 360,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut t = None;
            dev.CreateTexture2D(&desc, None, Some(&mut t)).unwrap();
            t.unwrap()
        };
        let mut c = Convert::new(&dev, &ctx, (640, 360), (320, 180)).unwrap();
        let a = c.convert(&src).unwrap();
        let b = c.convert(&src).unwrap();
        assert_ne!(a.as_raw(), b.as_raw(), "ring of output textures");
        let mut d = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: GetDesc on a live texture.
        unsafe { a.GetDesc(&mut d) };
        assert_eq!((d.Width, d.Height, d.Format), (320, 180, DXGI_FORMAT_NV12));
    }

    /// ClearType-like columns (red, cyan, red, ...) average to grey: the shader path keeps luma
    /// sharp and gives neutral chroma, where a point-sampled chroma would be fully saturated.
    #[test]
    fn subpixel_columns_give_neutral_chroma_and_sharp_luma() {
        let Some((dev, ctx, _, _)) = crate::engine::capture::device("") else { return eprintln!("no D3D11 device") };
        let (w, h) = (64u32, 64u32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..h {
            for x in 0..w {
                px.extend_from_slice(if x % 2 == 0 { &[0, 0, 255, 255] } else { &[255, 255, 0, 255] }); // BGRA
            }
        }
        // SAFETY: test-only texture creation and readback on live objects.
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let mut src = None;
            dev.CreateTexture2D(&desc, None, Some(&mut src)).unwrap();
            let src = src.unwrap();
            let up = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..desc };
            let mut stage = None;
            dev.CreateTexture2D(&up, None, Some(&mut stage)).unwrap();
            let stage = stage.unwrap();
            let mut mw = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&stage, 0, D3D11_MAP_WRITE, 0, Some(&mut mw)).unwrap();
            for r in 0..h as usize {
                std::ptr::copy_nonoverlapping(px.as_ptr().add(r * w as usize * 4), mw.pData.cast::<u8>().add(r * mw.RowPitch as usize), w as usize * 4);
            }
            ctx.Unmap(&stage, 0);
            ctx.CopyResource(&src, &stage);
            let up = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..desc };
            let mut stage = None;
            dev.CreateTexture2D(&up, None, Some(&mut stage)).unwrap();
            let stage = stage.unwrap();
            let mut mw = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&stage, 0, D3D11_MAP_WRITE, 0, Some(&mut mw)).unwrap();
            for r in 0..h as usize {
                std::ptr::copy_nonoverlapping(px.as_ptr().add(r * w as usize * 4), mw.pData.cast::<u8>().add(r * mw.RowPitch as usize), w as usize * 4);
            }
            ctx.Unmap(&stage, 0);
            ctx.CopyResource(&src, &stage);
            let mut c = Convert::new(&dev, &ctx, (w, h), (w, h)).unwrap();
            if c.ps.is_none() {
                return eprintln!("GPU can't render NV12 planes; video processor in use");
            }
            let out = c.convert(&src).unwrap();
            let sd = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, Format: DXGI_FORMAT_NV12, ..desc };
            let mut st = None;
            dev.CreateTexture2D(&sd, None, Some(&mut st)).unwrap();
            let st = st.unwrap();
            ctx.CopyResource(&st, &out);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).unwrap();
            let base = m.pData.cast::<u8>();
            let pitch = m.RowPitch as usize;
            let y = |x: usize, r: usize| *base.add(r * pitch + x) as i32;
            let uv = |x: usize, r: usize| *base.add((h as usize + r) * pitch + x) as i32;
            let (y0, y1) = (y(20, 20), y(21, 20));
            let worst = (4..28).flat_map(|r| (4..28).map(move |cx| (cx, r))).map(|(cx, r)| (uv(cx * 2, r) - 128).abs().max((uv(cx * 2 + 1, r) - 128).abs())).max().unwrap();
            ctx.Unmap(&st, 0);
            assert!((y1 - y0).abs() > 100, "luma stays sharp: {y0} vs {y1}");
            assert!(worst <= 3, "chroma is neutral, worst deviation {worst}");
        }
    }

    /// Two streams share one device and immediate context. Each converts its own solid colour in
    /// a loop; every output must equal what that stream produces alone. Interleaved state calls
    /// (render target, shader, viewport) show up as wrong or half-written planes.
    #[test]
    fn concurrent_streams_on_one_device_do_not_corrupt_each_other() {
        let Some((dev, ctx, _, _)) = crate::engine::capture::device("") else { return eprintln!("no D3D11 device") };
        let (w, h) = (1280u32, 720u32);
        let solid = |rgba: [f32; 4]| -> ID3D11Texture2D {
            // SAFETY: test-only texture creation on live objects.
            unsafe {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                };
                let mut t = None;
                dev.CreateTexture2D(&desc, None, Some(&mut t)).unwrap();
                let t = t.unwrap();
                let mut rtv = None;
                dev.CreateRenderTargetView(&t, None, Some(&mut rtv)).unwrap();
                let _g = GpuGuard::lock(&dev);
                ctx.ClearRenderTargetView(&rtv.unwrap(), &rgba);
                t
            }
        };
        let read = |nv12: &ID3D11Texture2D| -> Vec<u8> {
            // SAFETY: test-only staging readback under the device lock.
            unsafe {
                let mut d = D3D11_TEXTURE2D_DESC::default();
                nv12.GetDesc(&mut d);
                let sd = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..d };
                let mut st = None;
                dev.CreateTexture2D(&sd, None, Some(&mut st)).unwrap();
                let st = st.unwrap();
                let _g = GpuGuard::lock(&dev);
                ctx.CopyResource(&st, nv12);
                let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).unwrap();
                let rows = (h + h / 2) as usize;
                let mut v = Vec::with_capacity(rows * w as usize);
                for r in 0..rows {
                    v.extend_from_slice(std::slice::from_raw_parts(m.pData.cast::<u8>().add(r * m.RowPitch as usize), w as usize));
                }
                ctx.Unmap(&st, 0);
                v
            }
        };
        let colours = [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let srcs: Vec<_> = colours.iter().map(|c| solid(*c)).collect();
        let reference: Vec<Vec<u8>> = srcs
            .iter()
            .map(|s| {
                let mut c = Convert::new(&dev, &ctx, (w, h), (w, h)).unwrap();
                read(&c.convert(s).unwrap())
            })
            .collect();
        if Convert::new(&dev, &ctx, (w, h), (w, h)).unwrap().ps.is_none() {
            return eprintln!("GPU can't render NV12 planes; video processor in use");
        }
        let bad: Vec<usize> = std::thread::scope(|scope| {
            let jobs: Vec<_> = srcs
                .iter()
                .zip(&reference)
                .map(|(src, want)| {
                    let (dev, ctx, read) = (&dev, &ctx, &read);
                    scope.spawn(move || {
                        let mut c = Convert::new(dev, ctx, (w, h), (w, h)).unwrap();
                        // Back to back conversions keep both streams inside the call sequence at once.
                        (0..3000)
                            .filter(|i| {
                                let out = c.convert(src).unwrap();
                                i % 10 == 9 && read(&out) != *want
                            })
                            .count()
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        assert_eq!(bad, vec![0, 0], "frames that differ from the single-stream result");
    }
}

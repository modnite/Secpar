//! Decoded frames that stay on the graphics card. The decoder's NV12 surface is split into an
//! R8 luma and an RG8 chroma texture by two tiny pixel shaders. The window's Direct3D 12 renderer
//! opens those textures through shared handles and waits on a shared fence before reading them;
//! the OpenGL fallback samples them through WGL_NV_DX_interop2 (gpu.rs). Nothing goes back to system memory, which on
//! integrated graphics cost more than the decoding itself (a ThinkPad T480 fell back to the
//! software decoder at 1080p 4:4:4).
//!
//! The textures form a small ring. A slot is written only while no `GpuFrame` refers to it, so
//! OpenGL never reads a texture Direct3D is writing.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use windows::core::{Interface, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_TEXTURE2D};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource1;

const SLOTS: usize = 6;

const SHADERS: &str = "
cbuffer Place : register(b0) { int2 offset; int2 unused; };
Texture2D<float> luma : register(t0);
Texture2D<float2> chroma : register(t1);
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 c = float2((id << 1) & 2, id & 2);
    return float4(c * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}
float ps_luma(float4 p : SV_Position) : SV_Target { return luma.Load(int3(int2(p.xy) + offset, 0)); }
float2 ps_chroma(float4 p : SV_Position) : SV_Target { return chroma.Load(int3(int2(p.xy) + offset / 2, 0)); }
";

/// One slot: what the renderer samples.
pub struct Slot {
    pub luma: ID3D11Texture2D,
    pub chroma: ID3D11Texture2D,
    /// Handles Direct3D 12 opens the two textures with, when the ring is shared.
    pub shared: Option<[HANDLE; 2]>,
    luma_target: ID3D11RenderTargetView,
    chroma_target: ID3D11RenderTargetView,
    hold: Arc<()>,
}

/// The textures of one stream size, on one device.
pub struct Ring {
    pub id: u64,
    pub device: ID3D11Device,
    pub slots: Vec<Slot>,
    /// Signalled after each split, when the ring is shared: the reader waits for the frame's value.
    pub fence: Option<Fence>,
}

pub struct Fence {
    fence: ID3D11Fence,
    pub handle: HANDLE,
    next: AtomicU64,
}

impl Drop for Ring {
    fn drop(&mut self) {
        // SAFETY: handles this ring created and nobody else closes.
        unsafe {
            for h in self.slots.iter().filter_map(|s| s.shared).flatten() {
                let _ = CloseHandle(h);
            }
            if let Some(f) = &self.fence {
                let _ = CloseHandle(f.handle);
            }
        }
    }
}
// SAFETY: the device is multithread-protected (dxva.rs); OpenGL only reads a slot while it holds
// its `GpuFrame`, and Direct3D only writes slots nobody holds.
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

/// A decoded picture on the graphics card. Holding it keeps its slot from being overwritten.
#[derive(Clone)]
pub struct GpuFrame {
    pub ring: Arc<Ring>,
    pub index: usize,
    /// The ring's fence reaches this once the frame is written.
    pub fence_value: u64,
    _hold: Arc<()>,
}

/// The split pass for one coded surface size and visible area.
pub struct Splitter {
    ring: Arc<Ring>,
    surface: (u32, u32),
    visible: (u32, u32, u32, u32),
    copy: ID3D11Texture2D,
    luma_view: ID3D11ShaderResourceView,
    chroma_view: ID3D11ShaderResourceView,
    place: ID3D11Buffer,
    vs: ID3D11VertexShader,
    ps_luma: ID3D11PixelShader,
    ps_chroma: ID3D11PixelShader,
}

static RINGS: AtomicU64 = AtomicU64::new(1);

fn compile(entry: &str, target: &str) -> windows::core::Result<ID3DBlob> {
    let entry = std::ffi::CString::new(entry).unwrap();
    let target = std::ffi::CString::new(target).unwrap();
    let mut code = None;
    // SAFETY: the source and names outlive the call; the blob is returned owned.
    unsafe {
        D3DCompile(
            SHADERS.as_ptr().cast(),
            SHADERS.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(target.as_ptr().cast()),
            0,
            0,
            &mut code,
            None,
        )?;
    }
    code.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: the blob owns this memory for as long as it lives.
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
}

unsafe fn texture(device: &ID3D11Device, w: u32, h: u32, format: DXGI_FORMAT, bind: D3D11_BIND_FLAG, misc: u32) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: misc,
    };
    let mut out = None;
    device.CreateTexture2D(&desc, None, Some(&mut out))?;
    out.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
}

unsafe fn view(device: &ID3D11Device, texture: &ID3D11Texture2D, format: DXGI_FORMAT) -> windows::core::Result<ID3D11ShaderResourceView> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 } },
    };
    let mut out = None;
    device.CreateShaderResourceView(texture, Some(&desc), Some(&mut out))?;
    out.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
}

impl Splitter {
    /// `surface` is the decoder's coded size, `visible` the area shown (x, y, w, h). `share` makes the
    /// slots and a fence openable by Direct3D 12.
    pub fn new(device: &ID3D11Device, surface: (u32, u32), visible: (u32, u32, u32, u32), share: bool) -> windows::core::Result<Self> {
        let (x, y, w, h) = visible;
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        // SAFETY: plain resource creation on a valid device.
        unsafe {
            let support = device.CheckFormatSupport(DXGI_FORMAT_NV12)?;
            if support & D3D11_FORMAT_SUPPORT_SHADER_LOAD.0 as u32 == 0 {
                return Err(windows::core::Error::from(windows::Win32::Foundation::E_NOTIMPL));
            }
            let copy = texture(device, surface.0, surface.1, DXGI_FORMAT_NV12, D3D11_BIND_SHADER_RESOURCE, 0)?;
            let luma_view = view(device, &copy, DXGI_FORMAT_R8_UNORM)?;
            let chroma_view = view(device, &copy, DXGI_FORMAT_R8G8_UNORM)?;
            let target = D3D11_BIND_FLAG(D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0);
            let misc = if share { (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32 } else { 0 };
            // DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE
            let open = |t: &ID3D11Texture2D| t.cast::<IDXGIResource1>()?.CreateSharedHandle(None, 0x8000_0001, windows::core::PCWSTR::null());
            let mut ring = Ring { id: RINGS.fetch_add(1, Ordering::Relaxed), device: device.clone(), slots: Vec::with_capacity(SLOTS), fence: None };
            for _ in 0..SLOTS {
                let luma = texture(device, w, h, DXGI_FORMAT_R8_UNORM, target, misc)?;
                let chroma = texture(device, cw, ch, DXGI_FORMAT_R8G8_UNORM, target, misc)?;
                let (mut lt, mut ct) = (None, None);
                device.CreateRenderTargetView(&luma, None, Some(&mut lt))?;
                device.CreateRenderTargetView(&chroma, None, Some(&mut ct))?;
                let fail = || windows::core::Error::from(windows::Win32::Foundation::E_FAIL);
                let (luma_target, chroma_target) = (lt.ok_or_else(fail)?, ct.ok_or_else(fail)?);
                // Pushed before the handles are made, so a failure still closes those made so far.
                ring.slots.push(Slot { luma, chroma, shared: None, luma_target, chroma_target, hold: Arc::new(()) });
                if share {
                    let s = ring.slots.last_mut().unwrap();
                    let l = open(&s.luma)?;
                    match open(&s.chroma) {
                        Ok(c) => s.shared = Some([l, c]),
                        Err(e) => {
                            let _ = CloseHandle(l);
                            return Err(e);
                        }
                    }
                }
            }
            if share {
                let mut fence: Option<ID3D11Fence> = None;
                device.cast::<ID3D11Device5>()?.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence)?;
                let fence = fence.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
                // GENERIC_ALL
                let handle = fence.CreateSharedHandle(None, 0x1000_0000, windows::core::PCWSTR::null())?;
                ring.fence = Some(Fence { fence, handle, next: AtomicU64::new(0) });
            }
            let offset: [i32; 4] = [x as i32, y as i32, 0, 0];
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: 16,
                Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let data = D3D11_SUBRESOURCE_DATA { pSysMem: offset.as_ptr().cast(), SysMemPitch: 0, SysMemSlicePitch: 0 };
            let mut place = None;
            device.CreateBuffer(&desc, Some(&data), Some(&mut place))?;
            let (vs_code, luma_code, chroma_code) = (compile("vs", "vs_5_0")?, compile("ps_luma", "ps_5_0")?, compile("ps_chroma", "ps_5_0")?);
            let (mut vs, mut ps_luma, mut ps_chroma) = (None, None, None);
            device.CreateVertexShader(bytes(&vs_code), None, Some(&mut vs))?;
            device.CreatePixelShader(bytes(&luma_code), None, Some(&mut ps_luma))?;
            device.CreatePixelShader(bytes(&chroma_code), None, Some(&mut ps_chroma))?;
            let fail = || windows::core::Error::from(windows::Win32::Foundation::E_FAIL);
            Ok(Splitter {
                ring: Arc::new(ring),
                surface,
                visible,
                copy,
                luma_view,
                chroma_view,
                place: place.ok_or_else(fail)?,
                vs: vs.ok_or_else(fail)?,
                ps_luma: ps_luma.ok_or_else(fail)?,
                ps_chroma: ps_chroma.ok_or_else(fail)?,
            })
        }
    }

    pub fn fits(&self, surface: (u32, u32), visible: (u32, u32, u32, u32)) -> bool {
        self.surface == surface && self.visible == visible
    }

    /// Splits subresource `index` of `source` into a free slot. None when every slot is still held
    /// (the caller then copies the frame back instead).
    pub fn split(&self, ctx: &ID3D11DeviceContext, source: &ID3D11Texture2D, index: u32) -> Option<GpuFrame> {
        let slot = self.ring.slots.iter().position(|s| Arc::strong_count(&s.hold) == 1)?;
        let s = &self.ring.slots[slot];
        let (_, _, w, h) = self.visible;
        let viewport = |w: u32, h: u32| D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w as f32, Height: h as f32, MinDepth: 0.0, MaxDepth: 1.0 };
        let lock = ctx.cast::<ID3D11Multithread>().ok();
        let mut fence_value = 0;
        // SAFETY: every object belongs to this device; the sequence runs under the device lock so
        // Media Foundation's threads do not interleave with it.
        unsafe {
            if let Some(l) = &lock {
                l.Enter();
            }
            ctx.CopySubresourceRegion(&self.copy, 0, 0, 0, 0, source, index, None);
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.RSSetState(None);
            ctx.OMSetBlendState(None, None, u32::MAX);
            ctx.OMSetDepthStencilState(None, 0);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.place.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(self.luma_view.clone()), Some(self.chroma_view.clone())]));
            ctx.OMSetRenderTargets(Some(&[Some(s.luma_target.clone())]), None);
            ctx.RSSetViewports(Some(&[viewport(w, h)]));
            ctx.PSSetShader(&self.ps_luma, None);
            ctx.Draw(3, 0);
            ctx.OMSetRenderTargets(Some(&[Some(s.chroma_target.clone())]), None);
            ctx.RSSetViewports(Some(&[viewport(w.div_ceil(2), h.div_ceil(2))]));
            ctx.PSSetShader(&self.ps_chroma, None);
            ctx.Draw(3, 0);
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None, None]));
            if let Some(f) = &self.ring.fence {
                fence_value = f.next.fetch_add(1, Ordering::Relaxed) + 1;
                if let Ok(ctx4) = ctx.cast::<ID3D11DeviceContext4>() {
                    let _ = ctx4.Signal(&f.fence, fence_value);
                }
            }
            ctx.Flush();
            if let Some(l) = &lock {
                l.Leave();
            }
        }
        Some(GpuFrame { ring: self.ring.clone(), index: slot, fence_value, _hold: s.hold.clone() })
    }
}

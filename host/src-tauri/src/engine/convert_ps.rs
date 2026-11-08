//! BGRA → NV12 (BT.709, limited range) with a filtered chroma downsample, in two pixel-shader passes.
//!
//! ClearType text puts a different colour on each subpixel column. 4:2:0 keeps one chroma sample
//! per 2×2 pixels: taking that sample from a single pixel, or from the video processor's cheap
//! filter, aliases the colour into fringes. Luma stays at full resolution, so the text stays sharp;
//! chroma is a [1 3 3 1] low-pass over 4×4 pixels, which averages the subpixel colours back out.

use windows::core::{s, Interface};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const HLSL: &str = r#"
Texture2D src : register(t0);
SamplerState smp : register(s0);
cbuffer P : register(b0) { float2 outsz; float k; float pad; };
struct V { float4 p : SV_Position; };
V vs(uint id : SV_VertexID) {
    V o;
    float2 t = float2((id << 1) & 2, id & 2);
    o.p = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}
float3 px(float2 o) {
    float3 s = 0;
    int n = (int)k;
    [loop] for (int j = 0; j < n; j++)
        [loop] for (int i = 0; i < n; i++)
            s += src.SampleLevel(smp, (o + (float2(i, j) + 0.5) / n - 0.5) / outsz, 0).rgb;
    return s / (n * n);
}
float luma(float3 c) { return dot(c, float3(0.2126, 0.7152, 0.0722)); }
float main_y(V v) : SV_Target { return (16.0 + 219.0 * luma(px(v.p.xy))) / 255.0; }
float2 main_uv(V v) : SV_Target {
    float2 base = v.p.xy * 2.0;
    float w[4] = { 1, 3, 3, 1 };
    float3 s = 0;
    [unroll] for (int j = 0; j < 4; j++)
        [unroll] for (int i = 0; i < 4; i++)
            s += w[i] * w[j] * px(base + float2(i - 1.5, j - 1.5));
    s /= 64.0;
    float y = luma(s);
    return float2(128.0 + 224.0 * (s.b - y) / 1.8556, 128.0 + 224.0 * (s.r - y) / 1.5748) / 255.0;
}
"#;

pub struct Passes {
    ctx: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps_y: ID3D11PixelShader,
    ps_uv: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    size: (u32, u32),
    /// Per output texture: its luma and chroma plane views.
    views: Vec<(ID3D11RenderTargetView, ID3D11RenderTargetView)>,
    input: Option<(usize, ID3D11ShaderResourceView)>,
}

fn compile(entry: windows::core::PCSTR, target: windows::core::PCSTR) -> windows::core::Result<Vec<u8>> {
    let mut code = None;
    // SAFETY: the source slice and out pointers are valid for the call.
    unsafe {
        D3DCompile(HLSL.as_ptr().cast(), HLSL.len(), None, None, None, entry, target, 0, 0, &mut code, None)?;
        let b = code.ok_or_else(windows::core::Error::empty)?;
        Ok(std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize()).to_vec())
    }
}

impl Passes {
    /// Fails (and the caller keeps the video processor) when the GPU can't render into NV12 planes.
    pub fn new(dev: &ID3D11Device, ctx: &ID3D11DeviceContext, (iw, ih): (u32, u32), size: (u32, u32), outputs: &[ID3D11Texture2D]) -> windows::core::Result<Passes> {
        // SAFETY: D3D11 creation calls with initialised descriptors; blobs are copied out first.
        unsafe {
            let vs_code = compile(s!("vs"), s!("vs_5_0"))?;
            let y_code = compile(s!("main_y"), s!("ps_5_0"))?;
            let uv_code = compile(s!("main_uv"), s!("ps_5_0"))?;
            let (mut vs, mut ps_y, mut ps_uv) = (None, None, None);
            dev.CreateVertexShader(&vs_code, None, Some(&mut vs))?;
            dev.CreatePixelShader(&y_code, None, Some(&mut ps_y))?;
            dev.CreatePixelShader(&uv_code, None, Some(&mut ps_uv))?;
            let sd = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            dev.CreateSamplerState(&sd, Some(&mut sampler))?;
            // taps per axis for luma when scaling down: enough to box-filter the ratio
            let k = ((iw as f32 / size.0 as f32).max(ih as f32 / size.1 as f32).ceil()).clamp(1.0, 4.0);
            let data: [f32; 4] = [size.0 as f32, size.1 as f32, k, 0.0];
            let bd = D3D11_BUFFER_DESC { ByteWidth: 16, Usage: D3D11_USAGE_IMMUTABLE, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            let init = D3D11_SUBRESOURCE_DATA { pSysMem: data.as_ptr().cast(), ..Default::default() };
            let mut params = None;
            dev.CreateBuffer(&bd, Some(&init), Some(&mut params))?;
            let mut views = Vec::with_capacity(outputs.len());
            for t in outputs {
                let mk = |format| -> windows::core::Result<ID3D11RenderTargetView> {
                    let d = D3D11_RENDER_TARGET_VIEW_DESC {
                        Format: format,
                        ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                        ..Default::default()
                    };
                    let mut v = None;
                    dev.CreateRenderTargetView(t, Some(&d), Some(&mut v))?;
                    Ok(v.expect("view"))
                };
                views.push((mk(DXGI_FORMAT_R8_UNORM)?, mk(DXGI_FORMAT_R8G8_UNORM)?));
            }
            Ok(Passes {
                ctx: ctx.clone(),
                vs: vs.expect("vs"),
                ps_y: ps_y.expect("ps"),
                ps_uv: ps_uv.expect("ps"),
                sampler: sampler.expect("sampler"),
                params: params.expect("buffer"),
                size,
                views,
                input: None,
            })
        }
    }

    /// Renders `src` into output texture `idx`: luma pass, then chroma pass.
    pub fn run(&mut self, dev: &ID3D11Device, src: &ID3D11Texture2D, idx: usize) -> windows::core::Result<()> {
        // SAFETY: state-setting calls on live objects; the views outlive the draws.
        unsafe {
            let key = src.as_raw() as usize;
            if self.input.as_ref().is_none_or(|(k, _)| *k != key) {
                let mut v = None;
                dev.CreateShaderResourceView(src, None, Some(&mut v))?;
                self.input = Some((key, v.expect("srv")));
            }
            let srv = self.input.as_ref().expect("set").1.clone();
            let c = &self.ctx;
            c.IASetInputLayout(None);
            c.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.VSSetShader(&self.vs, None);
            c.PSSetShaderResources(0, Some(&[Some(srv)]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            let (w, h) = self.size;
            let (y, uv) = &self.views[idx];
            for (view, ps, vw, vh) in [(y, &self.ps_y, w, h), (uv, &self.ps_uv, w / 2, h / 2)] {
                c.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
                c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: vw as f32, Height: vh as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
                c.PSSetShader(ps, None);
                c.Draw(3, 0);
            }
            c.OMSetRenderTargets(None, None);
            c.PSSetShaderResources(0, Some(&[None]));
        }
        Ok(())
    }
}

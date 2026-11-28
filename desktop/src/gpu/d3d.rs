//! Windows draws the window with Direct3D 12 through wgpu. Decoded frames come from the decoder's
//! Direct3D 11 ring (video/dxgl.rs) through shared handles; before a new frame is drawn, the queue
//! waits on the ring's shared fence, so the picture is never read half written.
use super::{Handle, Shared, INTEROP, SHADER};
use crate::picture::Picture;
use crate::video::dxgl::{GpuFrame, Ring};
use eframe::{egui_wgpu, wgpu};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use windows_d3d12::Win32::Foundation::HANDLE as Handle12;
use windows_d3d12::Win32::Graphics::Direct3D12::{ID3D12Fence, ID3D12Resource};

type Dx12 = wgpu::hal::api::Dx12;

/// Frames kept from being overwritten after they leave the screen: the GPU may still be reading
/// them for the frames in flight.
const HELD: usize = 3;

// Limited-range BT.709, the matrix the Windows host encodes with (as in the OpenGL shader).
const SHADERS: &str = "
struct Place { full: i32, w: i32, h: i32, unused: i32 }
@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;
@group(0) @binding(3) var<uniform> place: Place;

struct Out { @builtin(position) position: vec4<f32>, @location(0) coordinate: vec2<f32> }

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    let c = corners[i];
    var o: Out;
    o.position = vec4(c, 0.0, 1.0);
    o.coordinate = vec2((c.x + 1.0) * 0.5, 1.0 - (c.y + 1.0) * 0.5);
    return o;
}

// One pixel of a packed 4:4:4 frame (see picture.rs): Y, Cb, Cr.
fn full(q: vec2<i32>) -> vec3<f32> {
    let shown = vec2<i32>(place.w, place.h);
    let p = clamp(q, vec2<i32>(0), shown - vec2<i32>(1));
    let cr = textureLoad(chroma, vec2<i32>(p.x / 2, p.y), 0).rg;
    return vec3(textureLoad(luma, p, 0).r, textureLoad(luma, vec2<i32>(p.x, p.y + shown.y), 0).r, select(cr.g, cr.r, p.x % 2 == 0));
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    var s = vec3(textureSample(luma, linear, in.coordinate).r, textureSample(chroma, linear, in.coordinate).rg);
    if place.full == 1 {
        // Bilinear by hand: the planes cannot be filtered as textures.
        let f = in.coordinate * vec2<f32>(f32(place.w), f32(place.h)) - 0.5;
        let i = vec2<i32>(floor(f));
        let t = f - floor(f);
        s = mix(mix(full(i), full(i + vec2<i32>(1, 0)), t.x), mix(full(i + vec2<i32>(0, 1)), full(i + vec2<i32>(1, 1)), t.x), t.y);
    }
    let y = 1.164383 * (s.x - 0.0627451);
    let c = s.yz - vec2(0.5019608);
    return vec4(y + 1.792741 * c.y, y - 0.213249 * c.x - 0.532909 * c.y, y + 2.112402 * c.x, 1.0);
}
";

pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    place: wgpu::Buffer,
    /// Planes uploaded from system memory: their textures, bind group and size.
    cpu: Option<(wgpu::Texture, wgpu::Texture, wgpu::BindGroup, [usize; 2])>,
    ring: Option<Imported>,
    current: Option<wgpu::BindGroup>,
    size: [usize; 2],
    held: VecDeque<GpuFrame>,
}

/// The decoder's ring as Direct3D 12 sees it.
struct Imported {
    id: u64,
    groups: Vec<wgpu::BindGroup>,
    fence: ID3D12Fence,
}
// SAFETY: the fence is a free-threaded Direct3D 12 object; it is used only from the paint callback.
unsafe impl Send for Imported {}

impl Resources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("secpar video"), source: wgpu::ShaderSource::Wgsl(SHADERS.into()) });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("secpar video"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("secpar video"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("secpar video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("secpar video"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let place = device.create_buffer(&wgpu::BufferDescriptor { label: Some("secpar video"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Resources { pipeline, layout, sampler, place, cpu: None, ring: None, current: None, size: [0, 0], held: VecDeque::new() }
    }

    fn group(&self, device: &wgpu::Device, luma: &wgpu::Texture, chroma: &wgpu::Texture) -> wgpu::BindGroup {
        let (l, c) = (luma.create_view(&Default::default()), chroma.create_view(&Default::default()));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("secpar video"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&l) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&c) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: self.place.as_entire_binding() },
            ],
        })
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, picture: &Picture) {
        let [w, h] = picture.size;
        let [cw, ch] = picture.chroma_size();
        if w == 0 || h == 0 || cw == 0 || ch == 0 {
            return;
        }
        let extent = |w: usize, h: usize| wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 };
        if self.cpu.as_ref().is_none_or(|c| c.3 != picture.size) {
            let make = |w, h, format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("secpar video"),
                    size: extent(w, h),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let (l, c) = (make(w, h, wgpu::TextureFormat::R8Unorm), make(cw, ch, wgpu::TextureFormat::Rg8Unorm));
            let group = self.group(device, &l, &c);
            self.cpu = Some((l, c, group, picture.size));
        }
        let (l, c, group, _) = self.cpu.as_ref().unwrap();
        for (texture, data, row, w, h) in [(l, &picture.y, w, w, h), (c, &picture.uv, cw * 2, cw, ch)] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(h as u32) },
                extent(w, h),
            );
        }
        self.current = Some(group.clone());
    }

    /// Opens the decoder's ring in Direct3D 12 once per ring.
    fn import(&mut self, device: &wgpu::Device, ring: &Ring) -> Result<(), String> {
        if self.ring.as_ref().is_some_and(|r| r.id == ring.id) {
            return Ok(());
        }
        self.ring = None;
        let fence_handle = ring.fence.as_ref().ok_or("the ring has no shared fence")?.handle;
        let mut planes: Vec<(ID3D12Resource, ID3D12Resource, (u32, u32))> = Vec::new();
        let fence: ID3D12Fence;
        // SAFETY: the handles are valid while `ring` lives; the opened objects hold their own references.
        unsafe {
            let hal = device.as_hal::<Dx12>().ok_or("not running on Direct3D 12")?;
            let raw = hal.raw_device();
            let mut f: Option<ID3D12Fence> = None;
            raw.OpenSharedHandle(Handle12(fence_handle.0), &mut f).map_err(|e| format!("fence: {e}"))?;
            fence = f.ok_or("fence")?;
            for slot in &ring.slots {
                let [lh, ch] = slot.shared.ok_or("the ring is not shared")?;
                let (mut l, mut c): (Option<ID3D12Resource>, Option<ID3D12Resource>) = (None, None);
                raw.OpenSharedHandle(Handle12(lh.0), &mut l).map_err(|e| format!("luma: {e}"))?;
                raw.OpenSharedHandle(Handle12(ch.0), &mut c).map_err(|e| format!("chroma: {e}"))?;
                let mut desc = Default::default();
                slot.luma.GetDesc(&mut desc);
                let desc: windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC = desc;
                planes.push((l.ok_or("luma")?, c.ok_or("chroma")?, (desc.Width, desc.Height)));
            }
        }
        let mut groups = Vec::with_capacity(planes.len());
        for (l, c, (w, h)) in planes {
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            let open = |resource, format, w, h| {
                let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
                // SAFETY: a 2D, single-mip texture of this format and size, only ever sampled here.
                unsafe {
                    let texture = wgpu::hal::dx12::Device::texture_from_raw(resource, format, wgpu::TextureDimension::D2, size, 1, 1);
                    device.create_texture_from_hal::<Dx12>(
                        texture,
                        &wgpu::TextureDescriptor {
                            label: Some("secpar decoded"),
                            size,
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                    )
                }
            };
            let luma = open(l, wgpu::TextureFormat::R8Unorm, w, h);
            let chroma = open(c, wgpu::TextureFormat::Rg8Unorm, cw, ch);
            groups.push(self.group(device, &luma, &chroma));
        }
        self.ring = Some(Imported { id: ring.id, groups, fence });
        Ok(())
    }

    /// Shows a frame straight from the decoder. Err when Direct3D 12 cannot open its ring.
    fn show(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: GpuFrame) -> Result<(), String> {
        self.import(device, &frame.ring)?;
        let imported = self.ring.as_ref().unwrap();
        let group = imported.groups.get(frame.index).ok_or("no such slot")?.clone();
        // SAFETY: the queue egui submits on; the wait only delays later work on it.
        unsafe {
            if let Some(q) = queue.as_hal::<Dx12>() {
                q.as_raw().Wait(&imported.fence, frame.fence_value).map_err(|e| format!("wait: {e}"))?;
            }
        }
        self.current = Some(group);
        self.held.push_back(frame);
        while self.held.len() > HELD {
            self.held.pop_front();
        }
        Ok(())
    }
}

pub struct Draw(pub Handle);

impl egui_wgpu::CallbackTrait for Draw {
    fn prepare(&self, device: &wgpu::Device, queue: &wgpu::Queue, _: &egui_wgpu::ScreenDescriptor, _: &mut wgpu::CommandEncoder, _: &mut egui_wgpu::CallbackResources) -> Vec<wgpu::CommandBuffer> {
        let mut shared = self.0.lock().unwrap();
        if shared.failed {
            return Vec::new();
        }
        let Some(format) = shared.format else { return Vec::new() };
        let shared = &mut *shared;
        let resources = shared.d3d.get_or_insert_with(|| Resources::new(device, format));
        if let Some(picture) = shared.pending.take() {
            let began = secpar_stream::timing::begin();
            let full = picture.packed;
            match picture.gpu.clone() {
                Some(frame) => {
                    if let Err(error) = resources.show(device, queue, frame) {
                        // The decoder copies frames back from now on.
                        INTEROP.store(2, Ordering::Relaxed);
                        eprintln!("secpar: Direct3D 12 cannot open the decoded frames ({error}); frames are copied back");
                        return Vec::new();
                    }
                    INTEROP.store(1, Ordering::Relaxed);
                }
                None => {
                    resources.held.clear();
                    resources.upload(device, queue, &picture);
                }
            }
            resources.size = picture.size;
            let [w, h] = picture.size;
            let place: [i32; 4] = [full as i32, w as i32, if full { h as i32 / 2 } else { h as i32 }, 0];
            queue.write_buffer(&resources.place, 0, &place.iter().flat_map(|v| v.to_ne_bytes()).collect::<Vec<u8>>());
            secpar_stream::timing::elapsed(secpar_stream::timing::Stage::Upload, began);
            secpar_stream::timing::mark(secpar_stream::timing::Stage::Draw);
        }
        Vec::new()
    }

    fn paint(&self, _: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, _: &egui_wgpu::CallbackResources) {
        let shared = self.0.lock().unwrap();
        let Some(r) = shared.d3d.as_ref() else { return };
        let Some(group) = r.current.as_ref().filter(|_| r.size != [0, 0]) else { return };
        pass.set_pipeline(&r.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

use eframe::egui;

/// Draws with Direct3D 12 when this PC has a hardware Direct3D 12 adapter, preferring the one the
/// decoder uses so frames can be shared. SECPAR_RENDERER=opengl keeps the OpenGL renderer.
pub fn choose(options: &mut eframe::NativeOptions) {
    if std::env::var("SECPAR_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("opengl")) {
        return;
    }
    let descriptor = || wgpu::InstanceDescriptor { backends: wgpu::Backends::DX12, ..Default::default() };
    let wanted = crate::video::adapter_ids();
    let pick = move |adapters: &[wgpu::Adapter]| -> Option<wgpu::Adapter> {
        let hardware: Vec<&wgpu::Adapter> = adapters.iter().filter(|a| a.get_info().device_type != wgpu::DeviceType::Cpu).collect();
        let same = hardware.iter().find(|a| {
            let i = a.get_info();
            wanted == Some((i.vendor, i.device))
        });
        same.or(hardware.first()).map(|a| (*a).clone())
    };
    let found = pick(&wgpu::Instance::new(&descriptor()).enumerate_adapters(wgpu::Backends::DX12));
    let Some(adapter) = found else {
        eprintln!("secpar: no Direct3D 12 adapter; drawing with OpenGL");
        return;
    };
    eprintln!("secpar: drawing with Direct3D 12 on {}", adapter.get_info().name);
    options.renderer = eframe::Renderer::Wgpu;
    options.wgpu_options.wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
        instance_descriptor: descriptor(),
        native_adapter_selector: Some(Arc::new(move |adapters, _| pick(adapters).ok_or_else(|| "no Direct3D 12 adapter".to_string()))),
        ..Default::default()
    });
}

/// Set up for the window's Direct3D 12 renderer.
pub fn new(render: &egui_wgpu::RenderState) -> Handle {
    SHADER.store(true, Ordering::Relaxed);
    super::DIRECT3D.store(true, Ordering::Relaxed);
    Arc::new(std::sync::Mutex::new(Shared { format: Some(render.target_format), ..Default::default() }))
}

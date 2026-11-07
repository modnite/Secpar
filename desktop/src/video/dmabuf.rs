//! VA surface export only: no CPU mapping, readback, or graphics context.
//! ABI references: GStreamer 1.26.2 gstvaallocator.h / gstvadisplay.h and
//! libva va.h / va_drmcommon.h (VADRMPRIMESurfaceDescriptor).
use gstreamer as gst;
use std::{ffi::c_void, os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd}};

const NV12: u32 = u32::from_le_bytes(*b"NV12");
const R8: u32 = u32::from_le_bytes(*b"R8  ");
const GR88: u32 = u32::from_le_bytes(*b"GR88");
const INVALID_MODIFIER: u64 = 0x00ff_ffff_ffff_ffff;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Object { fd: i32, size: u32, modifier: u64 }
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Layer {
    fourcc: u32, num_planes: u32,
    object_index: [u32; 4], offset: [u32; 4], pitch: [u32; 4],
}
#[repr(C)]
#[derive(Default)]
struct Descriptor {
    fourcc: u32, width: u32, height: u32, num_objects: u32,
    objects: [Object; 4], num_layers: u32, layers: [Layer; 4],
}
impl Descriptor {
    fn empty() -> Self {
        Self { objects: [Object { fd: -1, ..Object::default() }; 4], ..Self::default() }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExportError { Unavailable, Unsupported, InvalidDescriptor, Sync(i32), Export(i32) }

/// Owned objects plus their exact negotiated DRM layout. Fields remain private so
/// an importer cannot detach a borrowed fd from its frame lifetime accidentally.
pub(crate) struct ExportedFrame {
    _sample: gst::Sample,
    descriptor: Descriptor,
    visible: (u32, u32),
    _fds: Vec<OwnedFd>,
}
pub(crate) struct Plane<'a> {
    pub fd: BorrowedFd<'a>, pub width: u32, pub height: u32, pub fourcc: u32,
    pub offset: u32, pub pitch: u32, pub modifier: u64,
}
impl ExportedFrame {
    pub(crate) fn dimensions(&self) -> (u32, u32) { self.visible }
    pub(crate) fn storage_dimensions(&self) -> (u32, u32) { (self.descriptor.width, self.descriptor.height) }
    /// Borrowed fd valid only while this frame lives; Y then UV.
    pub(crate) fn plane(&self, index: usize) -> Option<Plane<'_>> {
        let layer = self.descriptor.layers.get(index).filter(|_| index < 2)?;
        let object = &self.descriptor.objects[layer.object_index[0] as usize];
        let (width, height) = self.storage_dimensions();
        let fd = self._fds.iter().find(|fd| fd.as_raw_fd() == object.fd)?.as_fd();
        Some(Plane { fd, offset: layer.offset[0], pitch: layer.pitch[0],
            modifier: object.modifier, fourcc: layer.fourcc,
            width: if index == 0 { width } else { width.div_ceil(2) },
            height: if index == 0 { height } else { height.div_ceil(2) } })
    }
}

type Surface = unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> u32;
type Display = unsafe extern "C" fn(*mut gst::ffi::GstMemory) -> *mut c_void;
type VaDisplay = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type Sync = unsafe extern "C" fn(*mut c_void, u32) -> i32;
type Export = unsafe extern "C" fn(*mut c_void, u32, u32, u32, *mut c_void) -> i32;

pub(crate) struct Exporter {
    surface: Surface, display: Display, va_display: VaDisplay, sync: Sync, export: Export,
    // Keep the symbols valid for this exporter's lifetime.
    _gst_va: libloading::Library, _va: libloading::Library,
}
impl Exporter {
    pub(crate) fn open() -> Result<Self, ExportError> {
        // These are trusted system sonames, never paths supplied by a stream.
        unsafe {
            let gst_va = libloading::Library::new("libgstva-1.0.so.0").map_err(|_| ExportError::Unavailable)?;
            let va = libloading::Library::new("libva.so.2").map_err(|_| ExportError::Unavailable)?;
            Ok(Self {
                surface: *gst_va.get::<Surface>(b"gst_va_memory_get_surface\0").map_err(|_| ExportError::Unavailable)?,
                display: *gst_va.get::<Display>(b"gst_va_memory_peek_display\0").map_err(|_| ExportError::Unavailable)?,
                va_display: *gst_va.get::<VaDisplay>(b"gst_va_display_get_va_dpy\0").map_err(|_| ExportError::Unavailable)?,
                sync: *va.get::<Sync>(b"vaSyncSurface\0").map_err(|_| ExportError::Unavailable)?,
                export: *va.get::<Export>(b"vaExportSurfaceHandle\0").map_err(|_| ExportError::Unavailable)?,
                _gst_va: gst_va, _va: va,
            })
        }
    }

    /// Caller supplies modifiers actually supported by its importer. This helper
    /// does not infer modifier support from a plugin being installed. vaSyncSurface
    /// can block: call on the decoder worker, never on the interface thread.
    pub(crate) fn export(&self, sample: gst::Sample, modifiers: &[u64]) -> Result<ExportedFrame, ExportError> {
        let caps = sample.caps().ok_or(ExportError::Unsupported)?;
        if !caps.is_fixed() || caps.size() != 1 || !caps.features(0).is_some_and(|f| f.contains("memory:VAMemory")) {
            return Err(ExportError::Unsupported);
        }
        let info = gstreamer_video::VideoInfo::from_caps(caps).map_err(|_| ExportError::Unsupported)?;
        if info.format() != gstreamer_video::VideoFormat::Nv12 || !dimensions(info.width(), info.height()) {
            return Err(ExportError::Unsupported);
        }
        let buffer = sample.buffer().ok_or(ExportError::Unsupported)?;
        if buffer.n_memory() != 1 { return Err(ExportError::Unsupported); }
        let memory = buffer.peek_memory(0);
        if !memory.is_type("VAMemory") { return Err(ExportError::Unsupported); }
        let mut descriptor = Descriptor::empty();
        // The sample pins memory and its allocator/display throughout these calls
        // and remains alive until the importer releases the exported frame.
        let status = unsafe {
            let mem = memory.as_ptr() as *mut gst::ffi::GstMemory;
            let display = (self.display)(mem);
            let surface = (self.surface)(mem);
            if display.is_null() || surface == u32::MAX { return Err(ExportError::Unsupported); }
            let va = (self.va_display)(display);
            if va.is_null() { return Err(ExportError::Unsupported); }
            let status = (self.sync)(va, surface);
            if status != 0 { return Err(ExportError::Sync(status)); }
            (self.export)(va, surface, 0x4000_0000, 0x0001 | 0x0004, (&mut descriptor as *mut Descriptor).cast())
        };
        // Adopt every returned fd before checking status or descriptor validity.
        // SAFETY: libva transfers newly exported fds to the caller.
        let fds = unsafe { adopt_fds(&descriptor) };
        if status != 0 { return Err(ExportError::Export(status)); }
        validate(&descriptor, (info.width(), info.height()), modifiers)?;
        let visible = (info.width(), info.height());
        Ok(ExportedFrame { _sample: sample, descriptor, visible, _fds: fds })
    }
}

unsafe fn adopt_fds(d: &Descriptor) -> Vec<OwnedFd> {
    let mut fds: Vec<OwnedFd> = Vec::with_capacity(4);
    for object in &d.objects {
        if object.fd >= 0 && !fds.iter().any(|fd| fd.as_raw_fd() == object.fd) {
            fds.push(OwnedFd::from_raw_fd(object.fd));
        }
    }
    fds
}
fn dimensions(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= 4096 * 2160
}
fn validate(d: &Descriptor, expected: (u32, u32), modifiers: &[u64]) -> Result<(), ExportError> {
    let invalid = || ExportError::InvalidDescriptor;
    if d.fourcc != NV12 || !dimensions(expected.0, expected.1)
        || d.width < expected.0 || d.height < expected.1
        || d.width - expected.0 > 255 || d.height - expected.1 > 255
        || d.width > 8192 || d.height > 8192
        || u64::from(d.width) * u64::from(d.height) > (4096 + 255) * (2160 + 255)
        || !(1..=4).contains(&d.num_objects) || d.num_layers != 2 || modifiers.is_empty() || modifiers.len() > 64 {
        return Err(invalid());
    }
    let objects = &d.objects[..d.num_objects as usize];
    for (i, o) in objects.iter().enumerate() {
        if o.fd < 0 || o.size == 0 || o.modifier == INVALID_MODIFIER || !modifiers.contains(&o.modifier)
            || objects[..i].iter().any(|p| p.fd == o.fd) { return Err(invalid()); }
    }
    let mut used = [false; 4];
    for (i, l) in d.layers[..2].iter().enumerate() {
        if l.fourcc != [R8, GR88][i] || l.num_planes != 1 || l.object_index[0] >= d.num_objects {
            return Err(invalid());
        }
        let index = l.object_index[0] as usize;
        used[index] = true;
        let o = &objects[index];
        let rows = if i == 0 { d.height } else { d.height.div_ceil(2) };
        let bytes = if i == 0 { d.width } else { d.width.div_ceil(2) * 2 };
        // Checked in u64; this is a minimum storage bound even for tiled layouts.
        let end = u64::from(l.offset[0]) + u64::from(l.pitch[0]) * u64::from(rows - 1) + u64::from(bytes);
        if l.pitch[0] < bytes || end > u64::from(o.size) { return Err(invalid()); }
    }
    if used[..objects.len()].contains(&false) { return Err(invalid()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    fn fixture(fd: i32) -> Descriptor {
        let mut d = Descriptor::empty();
        d.fourcc = NV12; d.width = 16; d.height = 8; d.num_objects = 1; d.num_layers = 2;
        d.objects[0] = Object { fd, size: 192, modifier: 0 };
        for i in 0..2 {
            d.layers[i] = Layer { fourcc: [R8, GR88][i], num_planes: 1,
                pitch: [16, 0, 0, 0], offset: [if i == 0 { 0 } else { 128 }, 0, 0, 0], ..Layer::default() };
        }
        d
    }
    #[test]
    fn validates_nv12_and_rejects_malformed_layouts() {
        assert!(validate(&fixture(3), (16, 8), &[0]).is_ok());
        for mutation in 0..13 {
            let mut d = fixture(3);
            match mutation {
                0 => d.num_objects = 5, 1 => d.num_layers = 5,
                2 => d.width = 0, 3 => d.fourcc = R8, 4 => d.layers[0].num_planes = 2,
                5 => d.layers[1].object_index[0] = 1, 6 => d.layers[1].offset[0] = u32::MAX,
                7 => d.layers[0].pitch[0] = 15, 8 => d.objects[0].modifier = INVALID_MODIFIER,
                9 => d.objects[0].fd = -1, 10 => d.layers[1].fourcc = R8,
                11 => d.objects[0].size = 191, _ => d.objects[0].modifier = 99,
            }
            assert_eq!(validate(&d, (16, 8), &[0]), Err(ExportError::InvalidDescriptor));
        }
    }
    #[test]
    fn rejects_duplicate_and_unused_objects() {
        let mut d = fixture(3); d.num_objects = 2; d.objects[1] = d.objects[0];
        assert!(validate(&d, (16, 8), &[0]).is_err());
        d.objects[1].fd = 4;
        assert!(validate(&d, (16, 8), &[0]).is_err());
        d.layers[1].object_index[0] = 1;
        assert!(validate(&d, (16, 8), &[0]).is_ok());
    }
    #[test]
    fn closes_exported_fds_on_invalid_descriptor_without_double_close() {
        use std::os::fd::IntoRawFd;
        let (owner, mut peer) = UnixStream::pair().unwrap();
        let fd = owner.into_raw_fd();
        let mut d = fixture(fd); d.num_objects = 5; d.objects[1].fd = fd;
        let owned = unsafe { adopt_fds(&d) };
        assert_eq!(owned.len(), 1);
        assert!(validate(&d, (16, 8), &[0]).is_err());
        drop(owned);
        use std::io::Read;
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }
    #[test]
    fn descriptor_abi_layout() {
        assert_eq!(std::mem::size_of::<Object>(), 16);
        assert_eq!(std::mem::offset_of!(Descriptor, num_layers), 80);
        assert_eq!(std::mem::offset_of!(Descriptor, layers), 84);
        assert_eq!(std::mem::size_of::<Descriptor>(), 312);
    }
    #[test]
    fn validates_padded_1080_storage_and_full_plane_bounds() {
        let mut d = fixture(3);
        d.width = 1920; d.height = 1088;
        d.objects[0].size = 1920 * 1088 * 3 / 2;
        d.layers[0].pitch[0] = 1920;
        d.layers[1].pitch[0] = 1920;
        d.layers[1].offset[0] = 1920 * 1088;
        assert!(validate(&d, (1920, 1080), &[0]).is_ok());
        assert!(validate(&d, (1664, 1080), &[0]).is_err());
        assert!(validate(&d, (1920, 832), &[0]).is_err());
        assert!(validate(&d, (1921, 1080), &[0]).is_err());
        assert!(validate(&d, (1920, 1089), &[0]).is_err());
        d.objects[0].size = 1920 * 1080 * 3 / 2;
        assert!(validate(&d, (1920, 1080), &[0]).is_err());
    }
    #[test]
    fn frame_owns_fds_and_exposes_plane_geometry() {
        use std::{io::Read, os::fd::IntoRawFd};
        gst::init().unwrap();
        let (owner, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let descriptor = fixture(owner.into_raw_fd());
        let fds = unsafe { adopt_fds(&descriptor) };
        let frame = ExportedFrame { _sample: gst::Sample::builder().build(), descriptor, visible: (15, 7), _fds: fds };
        assert_eq!(frame.dimensions(), (15, 7));
        assert_eq!(frame.storage_dimensions(), (16, 8));
        let plane = frame.plane(1).unwrap();
        assert_eq!((plane.width, plane.height, plane.fourcc, plane.offset, plane.pitch, plane.modifier),
            (8, 4, GR88, 128, 16, 0));
        assert!(plane.fd.as_raw_fd() >= 0);
        assert!(frame.plane(2).is_none());
        assert_eq!(peer.read(&mut [0]).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        drop(frame);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }
}

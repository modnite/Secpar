//! The virtual display: a screen that exists only for streaming, for computers with no monitor
//! attached or to stream a second screen. It uses the signed Virtual Display Driver (MIT,
//! github.com/VirtualDrivers/Virtual-Display-Driver), installed with Secpar in `vdd\`.
//! Turning it on adds one root display device; turning it off removes the device again.
//! The server runs as SYSTEM, which may install drivers and devices.

use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;

const HARDWARE_ID: &str = "Root\\MttVDD";

/// The sizes the virtual display offers, as `WIDTHxHEIGHT`.
pub const SIZES: [(u32, u32); 8] = [(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (1920, 1200), (2560, 1440), (2560, 1600), (3840, 2160)];

/// A size from the settings, or 1920x1080 when it is not one of [SIZES].
pub fn parse_size(s: &str) -> (u32, u32) {
    s.split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .filter(|size| SIZES.contains(size))
        .unwrap_or((1920, 1080))
}

/// Where the installer puts the driver files.
fn inf() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join("vdd").join("MttVDD.inf");
    p.exists().then_some(p)
}

/// Whether this installation carries the driver.
pub fn available() -> bool {
    inf().is_some()
}

/// The driver reads its settings from the folder named in the registry. It runs as LOCAL
/// SERVICE, which cannot read Secpar's own (administrators-only) data folder: without its
/// settings it paints the pointer into the picture and ignores the chosen size.
fn settings_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
    base.join("Secpar Virtual Display")
}

/// One screen, with the sizes clients commonly ask for. A hardware cursor keeps the pointer out
/// of the picture, as on a real monitor: clients draw it from the position the host sends.
pub fn settings_xml(size: (u32, u32)) -> String {
    // The chosen size first: the driver offers it as the screen's preferred size.
    let sizes = std::iter::once(size).chain(SIZES.into_iter().filter(|s| *s != size));
    let mut x = String::from("<?xml version='1.0' encoding='utf-8'?>\n<vdd_settings>\n<monitors><count>1</count></monitors>\n<gpu><friendlyname>default</friendlyname></gpu>\n<global>");
    for r in [60, 120, 144] {
        x += &format!("<g_refresh_rate>{r}</g_refresh_rate>");
    }
    x += "</global>\n<resolutions>\n";
    for (w, h) in sizes {
        x += &format!("<resolution><width>{w}</width><height>{h}</height><refresh_rate>60</refresh_rate></resolution>\n");
    }
    x += "</resolutions>\n<options><CustomEdid>false</CustomEdid><PreventSpoof>false</PreventSpoof><EdidCeaOverride>false</EdidCeaOverride><HardwareCursor>true</HardwareCursor><SDR10bit>false</SDR10bit><HDRPlus>false</HDRPlus><logging>false</logging><debuglogging>false</debuglogging></options>\n</vdd_settings>\n";
    x
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The virtual display devices present now (normally zero or one).
fn devices() -> Vec<(HDEVINFO, SP_DEVINFO_DATA)> {
    let mut found = Vec::new();
    // SAFETY: a device list of the display class; destroyed by the caller through `close`.
    let Ok(set) = (unsafe { SetupDiGetClassDevsW(Some(&GUID_DEVCLASS_DISPLAY), PCWSTR::null(), None, SETUP_DI_GET_CLASS_DEVS_FLAGS(0)) }) else { return found };
    let mut i = 0;
    loop {
        let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        // SAFETY: `info` is sized for this call; the loop ends at the first index past the list.
        if unsafe { SetupDiEnumDeviceInfo(set, i, &mut info) }.is_err() {
            break;
        }
        i += 1;
        let mut buf = [0u8; 1024];
        // SAFETY: the buffer is writable and its length is passed.
        if unsafe { SetupDiGetDeviceRegistryPropertyW(set, &info, SPDRP_HARDWAREID, None, Some(&mut buf), None) }.is_ok() {
            let ids: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let ids = String::from_utf16_lossy(&ids);
            if ids.split('\0').any(|id| id.eq_ignore_ascii_case(HARDWARE_ID)) {
                found.push((set, info));
            }
        }
    }
    if found.is_empty() {
        // SAFETY: nothing refers to the list any more.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(set) };
    }
    found
}

fn present() -> bool {
    let found = devices();
    if let Some((set, _)) = found.first() {
        // SAFETY: the list is not used after this.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(*set) };
    }
    !found.is_empty()
}

/// Turns the virtual display on (at `size`) or off. Errors are short sentences for the owner.
pub fn apply(on: bool, size: (u32, u32)) -> Result<(), String> {
    if on {
        add(size)?;
        resize(size)
    } else {
        remove()
    }
}

/// The virtual screen's name for the display settings (`\\.\DISPLAYn`), once Windows shows it.
fn screen() -> Option<Vec<u16>> {
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP};
    for i in 0.. {
        let mut device = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        // SAFETY: `device` is sized for the call.
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut device, 0) }.as_bool() {
            return None;
        }
        let adapter = String::from_utf16_lossy(&device.DeviceString);
        if adapter.contains("Virtual Display Driver") && device.StateFlags.0 & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP.0 != 0 {
            return Some(device.DeviceName.to_vec());
        }
    }
    None
}

/// Sets the virtual screen's size. It may take Windows a few seconds to show a new screen.
pub fn resize((w, h): (u32, u32)) -> Result<(), String> {
    use windows::Win32::Graphics::Gdi::{ChangeDisplaySettingsExW, CDS_UPDATEREGISTRY, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH};
    let mut name = None;
    for _ in 0..50 {
        name = screen();
        if name.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let name = name.ok_or("The virtual display did not appear.")?;
    let mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, dmFields: DM_PELSWIDTH | DM_PELSHEIGHT, dmPelsWidth: w, dmPelsHeight: h, ..Default::default() };
    // SAFETY: `name` is a null-terminated device name and `mode` lives through the call.
    let result = unsafe { ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&mode), None, CDS_UPDATEREGISTRY, None) };
    if result == DISP_CHANGE_SUCCESSFUL { Ok(()) } else { Err(format!("Windows did not set the virtual display to {w}×{h}.")) }
}

fn add(size: (u32, u32)) -> Result<(), String> {
    let inf = inf().ok_or("This installation has no virtual display driver.")?;
    let dir = settings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not prepare the virtual display: {e}"))?;
    use std::os::windows::process::CommandExt;
    // Read access for LOCAL SERVICE (S-1-5-19), the driver's account.
    let _ = std::process::Command::new("icacls").arg(&dir).args(["/grant", "*S-1-5-19:(OI)(CI)RX"]).creation_flags(0x0800_0000).status();
    let file = dir.join("vdd_settings.xml");
    let changed = std::fs::read_to_string(&file).ok().as_deref() != Some(settings_xml(size).as_str());
    std::fs::write(&file, settings_xml(size)).map_err(|e| format!("Could not prepare the virtual display: {e}"))?;
    // The driver reads its settings when it starts: a running display is added again to take new ones.
    if changed && present() {
        remove()?;
    }
    let reg = std::process::Command::new("reg")
        .args(["add", r"HKLM\SOFTWARE\MikeTheTech\VirtualDisplayDriver", "/v", "VDDPATH", "/t", "REG_SZ", "/d"])
        .arg(&dir)
        .arg("/f")
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map_err(|e| e.to_string())?;
    if !reg.success() {
        return Err("Could not prepare the virtual display.".into());
    }
    if !present() {
        // SAFETY: a fresh one-device list; every pointer below is to live local data.
        unsafe {
            let set = SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_DISPLAY), None).map_err(|e| e.to_string())?;
            let mut info = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
            let made = SetupDiCreateDeviceInfoW(set, w!("Display"), &GUID_DEVCLASS_DISPLAY, PCWSTR::null(), None, DICD_GENERATE_ID, Some(&mut info))
                .and_then(|_| {
                    let ids: Vec<u8> = HARDWARE_ID.encode_utf16().chain([0, 0]).flat_map(u16::to_le_bytes).collect();
                    SetupDiSetDeviceRegistryPropertyW(set, &mut info, SPDRP_HARDWAREID, Some(&ids))
                })
                .and_then(|_| SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set, Some(&info)));
            let _ = SetupDiDestroyDeviceInfoList(set);
            made.map_err(|e| format!("Windows did not add the virtual display ({e})."))?;
        }
    }
    let (id, path) = (wide(HARDWARE_ID), wide(&inf.to_string_lossy()));
    // SAFETY: both strings are null-terminated and outlive the call.
    unsafe { UpdateDriverForPlugAndPlayDevicesW(None, PCWSTR(id.as_ptr()), PCWSTR(path.as_ptr()), INSTALLFLAG_FORCE, None) }
        .map_err(|e| format!("Windows did not install the virtual display driver ({e})."))
}

fn remove() -> Result<(), String> {
    let found = devices();
    let mut result = Ok(());
    for (set, info) in &found {
        // SAFETY: `info` belongs to `set`, which is still open.
        if let Err(e) = unsafe { SetupDiCallClassInstaller(DIF_REMOVE, *set, Some(info)) } {
            result = Err(format!("Windows did not remove the virtual display ({e})."));
        }
    }
    if let Some((set, _)) = found.first() {
        // SAFETY: the list is not used after this.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(*set) };
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_ask_for_one_screen_at_common_sizes() {
        let x = super::settings_xml((2560, 1440));
        assert!(x.find("<width>2560</width><height>1440</height>").unwrap() < x.find("<width>1280</width>").unwrap(), "the chosen size comes first");
        assert_eq!(super::parse_size("2560x1440"), (2560, 1440));
        assert_eq!(super::parse_size("123x45"), (1920, 1080));
        assert!(x.contains("<count>1</count>"));
        assert!(x.contains("<width>1920</width><height>1080</height>"));
        assert!(x.contains("<width>3840</width><height>2160</height>"));
        assert!(x.starts_with("<?xml"));
        assert!(x.contains("<HardwareCursor>true</HardwareCursor>"), "the pointer stays out of the picture");
    }
}

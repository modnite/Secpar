//! Input is confined to the exact consented portal session used for capture.
use crate::portal::Portal;
use ashpd::desktop::remote_desktop::{Axis, KeyState};
use crate::pointer::{absolute, to_stream, CursorMode, Space, Tracker};
use secpar_host_core::proto::ClientMsg;
use std::{
    collections::BTreeSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

pub struct Input {
    portal: Arc<Portal>,
    keys: BTreeSet<i32>,
    buttons: BTreeSet<i32>,
    /// Where the remote pointer is. Only reported to the client when the frames do not carry it.
    tracker: Tracker,
    /// The encoded picture size, which follows live resizes.
    stream: Arc<AtomicU64>,
}
/// A picture size shared between the video thread and the input task.
pub fn pack_size(w: u32, h: u32) -> u64 {
    (w as u64) << 32 | h as u64
}
impl Input {
    pub fn new(portal: Arc<Portal>, stream: Arc<AtomicU64>) -> Self {
        Self {
            tracker: Tracker::new((portal.width, portal.height)),
            portal,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            stream,
        }
    }
    /// The pointer position in stream pixels once after it moved. None when the compositor paints the
    /// pointer into the video itself: the client must then draw nothing of its own.
    pub fn take_pos(&mut self) -> Option<(i32, i32)> {
        let pos = self.tracker.take()?;
        if self.portal.cursor == CursorMode::Embedded {
            return None;
        }
        let packed = self.stream.load(Ordering::Relaxed);
        let stream = ((packed >> 32) as u32, packed as u32);
        let space = Space { logical: (self.portal.width, self.portal.height), stream };
        Some(to_stream(pos, &space))
    }
    pub async fn apply(&mut self, message: ClientMsg) -> Result<(), String> {
        if self.portal.is_closed() {
            return Err("Desktop sharing was closed.".into());
        }
        let p = &self.portal;
        let s = &p.session;
        let state = |down| {
            if down {
                KeyState::Pressed
            } else {
                KeyState::Released
            }
        };
        let result = match message.sanitize() {
            ClientMsg::MouseMove { dx, dy } => {
                // The compositor will not say where its pointer is, so a relative move is only
                // followed from a place we put it: the middle of the monitor, once per session.
                if !self.tracker.known() && self.portal.cursor == CursorMode::Hidden {
                    let center = self.tracker.center();
                    p.remote
                        .notify_pointer_motion_absolute(s, p.node_id, center.0, center.1, Default::default())
                        .await
                        .map_err(|_| "The desktop portal rejected remote input.")?;
                    self.tracker.set(center);
                }
                self.tracker.relative(dx, dy);
                p.remote
                    .notify_pointer_motion(s, dx as f64, dy as f64, Default::default())
                    .await
            }
            ClientMsg::MousePos { x, y } => {
                let (ax, ay) = absolute(x, y, (p.width, p.height));
                self.tracker.set((ax, ay));
                p.remote
                    .notify_pointer_motion_absolute(s, p.node_id, ax, ay, Default::default())
                    .await
            }
            ClientMsg::MouseButton { b, down } => {
                let Some(button) = button_code(&b) else {
                    return Ok(());
                };
                if down {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
                p.remote
                    .notify_pointer_button(s, button, state(down), Default::default())
                    .await
            }
            ClientMsg::MouseScroll { dx, dy } => {
                if dy != 0 {
                    p.remote
                        .notify_pointer_axis_discrete(s, Axis::Vertical, -dy, Default::default())
                        .await
                        .map_err(|_| "Could not send scroll input.")?;
                }
                if dx != 0 {
                    p.remote
                        .notify_pointer_axis_discrete(s, Axis::Horizontal, dx, Default::default())
                        .await
                } else {
                    Ok(())
                }
            }
            ClientMsg::Key { vk, down } => {
                let Some(code) = evdev_key(vk) else {
                    return Ok(());
                };
                if down {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
                p.remote
                    .notify_keyboard_keycode(s, code, state(down), Default::default())
                    .await
            }
            ClientMsg::Text { s: text } => {
                for c in text.chars().take(1000) {
                    let symbol = keysym(c);
                    p.remote
                        .notify_keyboard_keysym(s, symbol, KeyState::Pressed, Default::default())
                        .await
                        .map_err(|_| "Could not send text input.")?;
                    p.remote
                        .notify_keyboard_keysym(s, symbol, KeyState::Released, Default::default())
                        .await
                        .map_err(|_| "Could not release text input.")?;
                }
                Ok(())
            }
            _ => Ok(()),
        };
        result.map_err(|_| "The desktop portal rejected remote input.".into())
    }
    /// Release only this session's held input before disconnect/revocation.
    pub async fn release_all(&mut self) {
        for code in std::mem::take(&mut self.keys) {
            let _ = self
                .portal
                .remote
                .notify_keyboard_keycode(
                    &self.portal.session,
                    code,
                    KeyState::Released,
                    Default::default(),
                )
                .await;
        }
        for button in std::mem::take(&mut self.buttons) {
            let _ = self
                .portal
                .remote
                .notify_pointer_button(
                    &self.portal.session,
                    button,
                    KeyState::Released,
                    Default::default(),
                )
                .await;
        }
    }
}
fn button_code(name: &str) -> Option<i32> {
    Some(match name {
        "left" => 0x110,
        "right" => 0x111,
        "middle" => 0x112,
        "back" | "x1" => 0x113,
        "forward" | "x2" => 0x114,
        _ => return None,
    })
}
fn keysym(c: char) -> i32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        c if c as u32 <= 0xff => c as i32,
        c => (0x0100_0000 | c as u32) as i32,
    }
}
/// PRS uses Windows virtual keys; the portal expects Linux evdev key codes.
pub(crate) fn evdev_key(vk: u32) -> Option<i32> {
    Some(match vk {
        0x08 => 14,
        0x09 => 15,
        0x0d => 28,
        0x10 | 0xa0 => 42,
        0xa1 => 54,
        0x11 | 0xa2 => 29,
        0xa3 => 97,
        0x12 | 0xa4 => 56,
        0xa5 => 100,
        0x13 => 119,
        0x14 => 58,
        0x1b => 1,
        0x20 => 57,
        0x21 => 104,
        0x22 => 109,
        0x23 => 107,
        0x24 => 102,
        0x25 => 105,
        0x26 => 103,
        0x27 => 106,
        0x28 => 108,
        0x2c => 99,
        0x2d => 110,
        0x2e => 111,
        0x30 => 11,
        0x31..=0x39 => (vk - 0x31 + 2) as i32,
        0x41 => 30,
        0x42 => 48,
        0x43 => 46,
        0x44 => 32,
        0x45 => 18,
        0x46 => 33,
        0x47 => 34,
        0x48 => 35,
        0x49 => 23,
        0x4a => 36,
        0x4b => 37,
        0x4c => 38,
        0x4d => 50,
        0x4e => 49,
        0x4f => 24,
        0x50 => 25,
        0x51 => 16,
        0x52 => 19,
        0x53 => 31,
        0x54 => 20,
        0x55 => 22,
        0x56 => 47,
        0x57 => 17,
        0x58 => 45,
        0x59 => 21,
        0x5a => 44,
        0x5b => 125,
        0x5c => 126,
        0x5d => 127,
        0x60 => 82,
        0x61 => 79,
        0x62 => 80,
        0x63 => 81,
        0x64 => 75,
        0x65 => 76,
        0x66 => 77,
        0x67 => 71,
        0x68 => 72,
        0x69 => 73,
        0x6a => 55,
        0x6b => 78,
        0x6d => 74,
        0x6e => 83,
        0x6f => 98,
        0x70..=0x79 => (vk - 0x70 + 59) as i32,
        0x7a => 87,
        0x7b => 88,
        0x90 => 69,
        0x91 => 70,
        0xba => 39,
        0xbb => 13,
        0xbc => 51,
        0xbd => 12,
        0xbe => 52,
        0xbf => 53,
        0xc0 => 41,
        0xdb => 26,
        0xdc => 43,
        0xdd => 27,
        0xde => 40,
        0xe2 => 86,
        _ => return None,
    })
}
/// Inverse of the host's physical-key mapping, shared with the Wayland client.
/// Prefer left/right modifier identities over their generic Windows aliases.
pub fn virtual_key(code:u32)->Option<u16> {
    [0xA0u32,0xA1,0xA2,0xA3,0xA4,0xA5].into_iter().chain(1..=255)
        .find(|vk|evdev_key(*vk)==Some(code as i32)).map(|vk|vk as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverse_key_mapping_preserves_physical_keys_and_modifier_sides() {
        for (code,vk) in [(125,0x5B),(126,0x5C),(29,0xA2),(97,0xA3),(56,0xA4),(100,0xA5),(42,0xA0),(54,0xA1),(18,0x45),(15,0x09)] {
            assert_eq!(virtual_key(code),Some(vk));
        }
        for vk in 1..=255 {if let Some(code)=evdev_key(vk) {let inverse=virtual_key(code as u32).unwrap();assert_eq!(evdev_key(inverse as u32),Some(code));}}
        assert_eq!(virtual_key(u32::MAX),None);
    }

    #[test]
    fn physical_modifiers_arrows_and_oem_keys_translate() {
        assert_eq!(evdev_key(0xa3), Some(97));
        assert_eq!(evdev_key(0xa5), Some(100));
        assert_eq!(evdev_key(0x25), Some(105));
        assert_eq!(evdev_key(0x41), Some(30));
        assert_eq!(evdev_key(0x5a), Some(44));
        assert_eq!(evdev_key(0xe2), Some(86));
        assert_eq!(evdev_key(0xffff), None);
    }
    #[test]
    fn unicode_text_and_auxiliary_buttons_are_preserved() {
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('界'), 0x0100_754c);
        assert_eq!(keysym('\n'), 0xff0d);
        assert_eq!(button_code("back"), Some(0x113));
    }
}

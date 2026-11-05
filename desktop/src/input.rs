use eframe::egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Input {
    pub captured: bool,
    held: BTreeSet<u32>,
    buttons: BTreeSet<&'static str>,
    ignore_ctrl: bool,
    fractional: [f32; 2],
}
impl Input {
    pub fn capture(&mut self) {
        self.captured = true;
        self.ignore_ctrl = true;
        self.fractional = [0.; 2];
    }
    pub fn release(&mut self) -> Vec<Value> {
        self.captured = false;
        self.fractional = [0.; 2];
        let mut out: Vec<_> = std::mem::take(&mut self.held)
            .into_iter()
            .map(|vk| json!({"t":"key","vk":vk,"down":false}))
            .collect();
        out.extend(
            std::mem::take(&mut self.buttons)
                .into_iter()
                .map(|b| json!({"t":"mouse.button","b":b,"down":false})),
        );
        out
    }
    pub fn modifiers(&mut self, m: Modifiers) -> Vec<Value> {
        if !self.captured {
            return vec![];
        }
        if !m.ctrl {
            self.ignore_ctrl = false;
        }
        let mut out = vec![];
        for (vk, down) in [
            (0x10, m.shift),
            (0x11, m.ctrl && !self.ignore_ctrl),
            (0x12, m.alt),
            (0x5B, m.mac_cmd),
        ] {
            if down && self.held.insert(vk) || !down && self.held.remove(&vk) {
                out.push(json!({"t":"key","vk":vk,"down":down}));
            }
        }
        out
    }
    pub fn event(&mut self, event: &Event) -> Vec<Value> {
        if !self.captured {
            return vec![];
        }
        match event {
            Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
                ..
            } => {
                if *key == Key::F12 && modifiers.ctrl {
                    return vec![];
                }
                if *repeat {
                    return vec![];
                }
                let Some(vk) = vk(physical_key.unwrap_or(*key)) else {
                    return vec![];
                };
                if *pressed {
                    self.held.insert(vk);
                } else {
                    self.held.remove(&vk);
                }
                vec![json!({"t":"key","vk":vk,"down":pressed})]
            }
            Event::MouseMoved(d) => {
                self.fractional[0] += d.x;
                self.fractional[1] += d.y;
                let (dx, dy) = (
                    self.fractional[0].trunc() as i32,
                    self.fractional[1].trunc() as i32,
                );
                self.fractional[0] -= dx as f32;
                self.fractional[1] -= dy as f32;
                if dx == 0 && dy == 0 {
                    vec![]
                } else {
                    vec![
                        json!({"t":"mouse.move","dx":dx.clamp(-2000,2000),"dy":dy.clamp(-2000,2000)}),
                    ]
                }
            }
            Event::PointerButton {
                button, pressed, ..
            } => {
                let b = match button {
                    PointerButton::Primary => "left",
                    PointerButton::Secondary => "right",
                    PointerButton::Middle => "middle",
                    PointerButton::Extra1 => "x1",
                    PointerButton::Extra2 => "x2",
                };
                if *pressed {
                    self.buttons.insert(b);
                } else {
                    self.buttons.remove(b);
                }
                vec![json!({"t":"mouse.button","b":b,"down":pressed})]
            }
            Event::MouseWheel { unit, delta, .. } => {
                let scale = match unit {
                    MouseWheelUnit::Line => 120.,
                    MouseWheelUnit::Point => 3.,
                    MouseWheelUnit::Page => 1200.,
                };
                vec![
                    json!({"t":"mouse.scroll","dx":(delta.x*scale) as i32,"dy":(delta.y*scale) as i32}),
                ]
            }
            _ => vec![],
        }
    }
}
fn vk(key: Key) -> Option<u32> {
    let name = format!("{key:?}");
    if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() {
        return Some(name.as_bytes()[0] as u32);
    }
    if let Some(n) = name
        .strip_prefix("Num")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n < 10)
    {
        return Some(0x30 + n);
    }
    if let Some(n) = name
        .strip_prefix('F')
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Some(0x6F + n);
    }
    Some(match key {
        Key::ArrowLeft => 0x25,
        Key::ArrowUp => 0x26,
        Key::ArrowRight => 0x27,
        Key::ArrowDown => 0x28,
        Key::Escape => 0x1B,
        Key::Tab => 0x09,
        Key::Backspace => 0x08,
        Key::Enter => 0x0D,
        Key::Space => 0x20,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::Minus => 0xBD,
        Key::Plus | Key::Equals => 0xBB,
        Key::OpenBracket => 0xDB,
        Key::CloseBracket => 0xDD,
        Key::Backslash => 0xDC,
        Key::Semicolon => 0xBA,
        Key::Quote => 0xDE,
        Key::Comma => 0xBC,
        Key::Period => 0xBE,
        Key::Slash => 0xBF,
        Key::Backtick => 0xC0,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_lifts_held_keys_and_buttons() {
        let mut i = Input::default();
        i.capture();
        i.held.insert(65);
        i.buttons.insert("left");
        let events = i.release();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e["down"] == false));
        assert!(i.release().is_empty());
        assert!(i
            .event(&Event::MouseMoved(eframe::egui::vec2(5., 6.)))
            .is_empty());
    }
    #[test]
    fn recapture_does_not_forward_the_shortcut_control_key() {
        let mut i = Input::default();
        i.capture();
        assert!(i
            .modifiers(Modifiers {
                ctrl: true,
                ..Default::default()
            })
            .is_empty());
        i.modifiers(Modifiers::default());
        assert_eq!(
            i.modifiers(Modifiers {
                ctrl: true,
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(i.release().len(), 1);
    }
    #[test]
    fn preserves_subpixel_mouse_motion() {
        let mut i = Input::default();
        i.capture();
        for _ in 0..3 {
            assert!(i
                .event(&Event::MouseMoved(eframe::egui::vec2(0.25, 0.)))
                .is_empty());
        }
        assert_eq!(
            i.event(&Event::MouseMoved(eframe::egui::vec2(0.25, 0.)))[0]["dx"],
            1
        );
    }
}

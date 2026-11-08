use eframe::egui::{Event, Key, Modifiers, MouseWheelUnit, PointerButton};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn fullscreen_shortcut(event: &Event) -> bool {
    matches!(event,Event::Key{key:Key::F,pressed:true,repeat:false,modifiers,..}
        if modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.mac_cmd)
}

/// Client hotkeys, with Parsec's defaults where it has them: Ctrl+Shift+W windowed/fullscreen,
/// Ctrl+Alt+` disconnect. Alt+F (windowed/fullscreen) stays, and Ctrl+Alt+S hides or shows the
/// Secpar button.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shortcut {
    Fullscreen,
    Disconnect,
    MenuButton,
}

pub fn shortcut(event: &Event) -> Option<(Shortcut, Key)> {
    let Event::Key { key, pressed: true, repeat: false, modifiers: m, .. } = event else { return None };
    if m.mac_cmd {
        return None;
    }
    let ctrl_alt = m.ctrl && m.alt && !m.shift;
    let ctrl_shift = m.ctrl && m.shift && !m.alt;
    let found = match key {
        Key::F if m.alt && !m.ctrl && !m.shift => Shortcut::Fullscreen,
        Key::W if ctrl_shift => Shortcut::Fullscreen,
        Key::Backtick if ctrl_alt => Shortcut::Disconnect,
        Key::S if ctrl_alt => Shortcut::MenuButton,
        _ => return None,
    };
    Some((found, *key))
}

#[derive(Default)]
pub struct Input {
    pub captured: bool,
    pub enabled: bool,
    held: BTreeSet<u32>,
    buttons: BTreeSet<&'static str>,
    ignore_ctrl: bool,
    ignore_alt: bool,
    ignore_shift: bool,
    fractional: [f32; 2],
}
impl Input {
    pub fn capture(&mut self) {
        self.captured = true;
        self.enabled = true;
        self.ignore_ctrl = true;
        self.ignore_alt = true;
        self.ignore_shift = true;
        self.fractional = [0.; 2];
    }
    pub fn free(&mut self) {
        self.capture();
        self.captured = false;
    }
    pub fn release_buttons(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.buttons).into_iter()
            .map(|b| json!({"t":"mouse.button","b":b,"down":false})).collect()
    }
    pub fn absolute(pos: eframe::egui::Pos2, picture: eframe::egui::Rect) -> Option<Value> {
        if !picture.is_positive() || !picture.contains(pos) { return None; }
        Some(json!({"t":"mouse.pos","x":((pos.x-picture.min.x)/picture.width()).clamp(0.,1.),
            "y":((pos.y-picture.min.y)/picture.height()).clamp(0.,1.)}))
    }
    pub fn release(&mut self) -> Vec<Value> {
        self.captured = false;
        self.enabled = false;
        self.fractional = [0.; 2];
        let mut out: Vec<_> = std::mem::take(&mut self.held)
            .into_iter()
            .map(|vk| json!({"t":"key","vk":vk,"down":false}))
            .collect();
        out.extend(self.release_buttons());
        out
    }
    pub fn modifiers(&mut self, m: Modifiers) -> Vec<Value> {
        if !self.enabled {
            return vec![];
        }
        if !m.ctrl {
            self.ignore_ctrl = false;
        }
        if !m.alt {self.ignore_alt=false;}
        if !m.shift {self.ignore_shift=false;}
        let mut out = vec![];
        for (vk, down) in [
            (0x10, m.shift && !self.ignore_shift),
            (0x11, m.ctrl && !self.ignore_ctrl),
            (0x12, m.alt && !self.ignore_alt),
            (0x5B, m.mac_cmd),
        ] {
            if down && self.held.insert(vk) || !down && self.held.remove(&vk) {
                out.push(json!({"t":"key","vk":vk,"down":down}));
            }
        }
        out
    }
    pub fn event(&mut self, event: &Event) -> Vec<Value> {
        if !self.enabled {
            return vec![];
        }
        match event {
            Event::Copy => self.clipboard_chord(0x43),
            Event::Cut => self.clipboard_chord(0x58),
            Event::Paste(_) => self.clipboard_chord(0x56),
            Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                ..
            } => {
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
                if !self.captured { return vec![]; }
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
    fn clipboard_chord(&self,vk:u32)->Vec<Value> {
        let held=self.held.contains(&0x11);
        let mut out=vec![];
        if !held {out.push(json!({"t":"key","vk":0x11,"down":true}));}
        out.push(json!({"t":"key","vk":vk,"down":true}));
        out.push(json!({"t":"key","vk":vk,"down":false}));
        if !held {out.push(json!({"t":"key","vk":0x11,"down":false}));}
        out
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
    #[test]
    fn toolkit_clipboard_events_forward_remote_shortcuts_without_releasing_held_control() {
        let mut input=super::Input::default();input.free();
        let copy=input.event(&eframe::egui::Event::Copy);
        assert_eq!(copy.iter().map(|v|(v["vk"].as_u64().unwrap(),v["down"].as_bool().unwrap())).collect::<Vec<_>>(),
            vec![(0x11,true),(0x43,true),(0x43,false),(0x11,false)]);
        input.modifiers(eframe::egui::Modifiers::default());
        input.modifiers(eframe::egui::Modifiers{ctrl:true,..Default::default()});
        let paste=input.event(&eframe::egui::Event::Paste("Synthetic text".into()));
        assert_eq!(paste.len(),2);assert_eq!(paste[0]["vk"],0x56);assert_eq!(paste[1]["down"],false);
        assert!(input.release().iter().any(|v|v["vk"]==0x11 && v["down"]==false));
    }
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
    fn one_hand_recapture_suppresses_both_modifiers_until_released() {
        let mut input=Input::default();input.capture();
        let chord=Modifiers{ctrl:true,alt:true,..Default::default()};
        assert!(input.modifiers(chord).is_empty());
        assert!(input.modifiers(Modifiers::default()).is_empty());
        let pressed=input.modifiers(chord);
        assert_eq!(pressed.len(),2);
        assert!(pressed.iter().all(|event|event["down"]==true));
        let released=input.release();
        assert_eq!(released.len(),2);
        assert!(released.iter().all(|event|event["down"]==false));
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

    #[test]
    fn free_pointer_forwards_keyboard_and_buttons_without_relative_mouse_motion() {
        let mut input=Input::default();input.free();
        assert!(input.enabled && !input.captured);
        assert!(input.event(&Event::MouseMoved(eframe::egui::vec2(30.,10.))).is_empty());
        let event=Event::Key {key:Key::A,physical_key:Some(Key::A),pressed:true,repeat:false,modifiers:Modifiers::default()};
        assert_eq!(input.event(&event)[0]["vk"],65);
        assert_eq!(input.release()[0]["down"],false);
        assert!(!input.enabled && !input.captured);
    }

    #[test]
    fn absolute_pointer_maps_only_the_displayed_picture_and_excludes_letterbox() {
        use eframe::egui::{pos2,Rect};
        let picture=Rect::from_min_max(pos2(20.,40.),pos2(620.,340.));
        let middle=Input::absolute(pos2(320.,190.),picture).unwrap();
        assert_eq!(middle["x"],0.5);assert_eq!(middle["y"],0.5);
        assert!(Input::absolute(pos2(10.,190.),picture).is_none());
        assert!(Input::absolute(pos2(320.,20.),picture).is_none());
        assert!(Input::absolute(pos2(0.,0.),Rect::NOTHING).is_none());
    }

    #[test]
    fn leaving_picture_releases_buttons_without_releasing_remote_keyboard() {
        let mut input=Input::default();input.free();input.held.insert(65);input.buttons.insert("left");
        let events=input.release_buttons();assert_eq!(events.len(),1);assert_eq!(events[0]["b"],"left");
        assert!(input.held.contains(&65));assert!(input.enabled);assert!(input.release_buttons().is_empty());
    }

    #[test]
    fn client_hotkeys_follow_parsec_and_need_the_exact_modifiers() {
        let key=|key,modifiers,repeat| Event::Key{key,physical_key:Some(key),pressed:true,repeat,modifiers};
        let ctrl_shift=Modifiers{ctrl:true,shift:true,..Default::default()};
        let ctrl_alt=Modifiers{ctrl:true,alt:true,..Default::default()};
        assert_eq!(shortcut(&key(Key::W,ctrl_shift,false)).map(|s|s.0),Some(Shortcut::Fullscreen));
        assert_eq!(shortcut(&key(Key::Backtick,ctrl_alt,false)).map(|s|s.0),Some(Shortcut::Disconnect));
        assert_eq!(shortcut(&key(Key::S,ctrl_alt,false)).map(|s|s.0),Some(Shortcut::MenuButton));
        assert_eq!(shortcut(&key(Key::F,Modifiers{alt:true,..Default::default()},false)).map(|s|s.0),Some(Shortcut::Fullscreen));
        assert!(shortcut(&key(Key::W,ctrl_shift,true)).is_none(), "no key repeat");
        assert!(shortcut(&key(Key::W,Modifiers{ctrl:true,..Default::default()},false)).is_none());
        assert!(shortcut(&key(Key::S,Modifiers{ctrl:true,alt:true,shift:true,..Default::default()},false)).is_none());
        assert!(shortcut(&key(Key::Backtick,Modifiers{alt:true,..Default::default()},false)).is_none());
    }

    #[test]
    fn fullscreen_shortcut_requires_exact_alt_f_and_no_repeat_or_release() {
        let event=|modifiers,pressed,repeat| Event::Key{key:Key::F,physical_key:Some(Key::F),pressed,repeat,modifiers};
        let alt=Modifiers{alt:true,..Default::default()};
        assert!(fullscreen_shortcut(&event(alt,true,false)));
        assert!(!fullscreen_shortcut(&event(alt,false,false)));
        assert!(!fullscreen_shortcut(&event(alt,true,true)));
        assert!(!fullscreen_shortcut(&event(Modifiers::default(),true,false)));
        assert!(!fullscreen_shortcut(&event(Modifiers{ctrl:true,..alt},true,false)));
        assert!(!fullscreen_shortcut(&event(Modifiers{shift:true,..alt},true,false)));
    }

    #[test]
    fn painted_video_layer_is_hittable_but_local_popups_take_precedence() {
        use eframe::egui::{self,Context,RawInput,Rect,pos2,vec2};
        let ctx=Context::default();
        let mut video=None;
        for _ in 0..2 {
            let _=ctx.run(RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.,0.),vec2(640.,400.))),..Default::default()},|ctx| {
                egui::CentralPanel::default().show(ctx,|ui| {
                    let (_,rect)=ui.allocate_space(ui.available_size());
                    ui.painter().rect_filled(rect,0.,egui::Color32::BLACK);
                    video=Some((rect,ui.layer_id()));
                });
            });
        }
        let (rect,layer)=video.unwrap();
        assert_eq!(ctx.layer_id_at(rect.center()),Some(layer),"A painted video surface must accept free-pointer input");
        for _ in 0..2 {
            let _=ctx.run(RawInput{screen_rect:Some(Rect::from_min_size(pos2(0.,0.),vec2(640.,400.))),..Default::default()},|ctx| {
                egui::CentralPanel::default().show(ctx,|ui|{ui.allocate_space(ui.available_size());});
                egui::Area::new(egui::Id::new("local-stream-menu")).order(egui::Order::Foreground)
                    .fixed_pos(rect.center()-vec2(40.,20.)).show(ctx,|ui|{ui.allocate_space(vec2(80.,40.));});
            });
        }
        assert_ne!(ctx.layer_id_at(rect.center()),Some(layer),"A local popup must not forward input into the remote desktop");
    }
}

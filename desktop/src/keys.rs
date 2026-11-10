// The hook half of this is Windows only; other platforms use the escape chord and fullscreen rules.
#![cfg_attr(not(windows), allow(dead_code))]
//! Keyboard priority for a stream. While a stream is on screen and the window has focus, the host
//! gets every key first. The client keeps one deliberate escape chord. Everything here is pure
//! logic so the rules can be tested without touching real input.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;

/// The modifiers of the escape chord. The last key picks the action. Persisted by name.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EscapeKeys {
    /// Rare enough that a host program will not want it.
    #[default]
    CtrlAltShift,
    CtrlShift,
    /// The chord of 3.16 and earlier. Host programs and AltGr layouts can collide with it.
    CtrlAlt,
}
impl EscapeKeys {
    pub const ALL: [Self; 3] = [Self::CtrlAltShift, Self::CtrlShift, Self::CtrlAlt];
    pub fn label(self) -> &'static str {
        match self {
            Self::CtrlAltShift => "Ctrl+Alt+Shift",
            Self::CtrlShift => "Ctrl+Shift",
            Self::CtrlAlt => "Ctrl+Alt",
        }
    }
    /// Exactly these modifiers must be down: (ctrl, alt, shift).
    pub fn needs(self) -> (bool, bool, bool) {
        match self {
            Self::CtrlAltShift => (true, true, true),
            Self::CtrlShift => (true, false, true),
            Self::CtrlAlt => (true, true, false),
        }
    }
    pub fn matches(self, ctrl: bool, alt: bool, shift: bool) -> bool {
        self.needs() == (ctrl, alt, shift)
    }
    pub fn chord(self, action: Escape) -> String {
        format!("{}+{}", self.label(), action.letter())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Escape {
    Fullscreen,
    Disconnect,
    MenuButton,
}
impl Escape {
    pub fn letter(self) -> char {
        match self {
            Self::Fullscreen => 'W',
            Self::Disconnect => 'Q',
            Self::MenuButton => 'S',
        }
    }
    pub fn from_vk(vk: u16) -> Option<Self> {
        match vk {
            0x57 => Some(Self::Fullscreen),
            0x51 => Some(Self::Disconnect),
            0x53 => Some(Self::MenuButton),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Host,
    Local,
}
/// Where a key goes while the stream owns the keyboard. Only keys whose job is on this computer
/// stay here: the audio plays locally, so volume and media keys keep their local meaning, and
/// input-method and sleep keys are not keystrokes for another computer. Everything else goes to
/// the host, including Win, Alt+Tab, Alt+F4, Ctrl+Esc, PrintScreen, the Menu key and F10/F11.
pub fn route(vk: u16) -> Route {
    match vk {
        0 | 0xFF => Route::Local,
        0x15..=0x1A | 0x1C..=0x1F => Route::Local, // IME mode keys
        0x5F => Route::Local,                      // sleep
        0xAD..=0xB3 => Route::Local,               // mute, volume, track and play/pause keys
        0xE5 | 0xE7 => Route::Local,               // IME process key, injected unicode packet
        _ => Route::Host,
    }
}

/// What the hook should do with one key event.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Verdict {
    /// Keep the key from the rest of this computer.
    pub swallow: bool,
    /// Key events for the host, in order: (virtual key, down).
    pub send: Vec<(u16, bool)>,
    /// The escape chord was pressed. The client acts, the host never sees it.
    pub escape: Option<Escape>,
}
impl Verdict {
    fn pass() -> Self {
        Self::default()
    }
    fn swallow(send: Vec<(u16, bool)>) -> Self {
        Self { swallow: true, send, escape: None }
    }
}

/// The per-stream key state machine behind the Windows hook.
pub struct Filter {
    escape: EscapeKeys,
    /// False when keyboard control is off for the session: nothing is touched.
    keyboard: bool,
    /// Every key seen down and not yet up.
    down: BTreeSet<u16>,
    /// Down before the hook was up, or routed local: left to this computer until released.
    local: BTreeSet<u16>,
    /// Sent to the host as down.
    host: BTreeSet<u16>,
    /// Part of an escape chord: swallowed and not forwarded until released.
    ignored: BTreeSet<u16>,
}
impl Filter {
    pub fn new(escape: EscapeKeys, keyboard: bool, already_down: impl IntoIterator<Item = u16>) -> Self {
        let local: BTreeSet<u16> = already_down.into_iter().collect();
        Self { escape, keyboard, down: local.clone(), local, host: BTreeSet::new(), ignored: BTreeSet::new() }
    }
    #[cfg(test)]
    pub fn held_on_host(&self) -> Vec<u16> {
        self.host.iter().copied().collect()
    }
    fn mods(&self) -> (bool, bool, bool) {
        let any = |keys: &[u16]| keys.iter().any(|k| self.down.contains(k));
        let win = any(&[VK_LWIN, VK_RWIN]);
        let (ctrl, alt, shift) = (
            any(&[0x11, VK_LCONTROL, VK_RCONTROL]),
            any(&[0x12, VK_LMENU, VK_RMENU]),
            any(&[0x10, VK_LSHIFT, VK_RSHIFT]),
        );
        // A chord with the Win key is somebody's host shortcut, not ours.
        if win { (false, false, false) } else { (ctrl, alt, shift) }
    }
    pub fn key(&mut self, vk: u16, down: bool) -> Verdict {
        if !self.keyboard {
            return Verdict::pass();
        }
        if self.local.contains(&vk) {
            if !down {
                self.local.remove(&vk);
                self.down.remove(&vk);
            }
            return Verdict::pass();
        }
        if self.ignored.contains(&vk) {
            if !down {
                self.ignored.remove(&vk);
                self.down.remove(&vk);
            }
            return Verdict::swallow(vec![]);
        }
        if !down {
            self.down.remove(&vk);
            // An up for a key this hook never saw go down belongs to this computer.
            return if self.host.remove(&vk) { Verdict::swallow(vec![(vk, false)]) } else { Verdict::pass() };
        }
        if route(vk) == Route::Local {
            self.down.insert(vk);
            self.local.insert(vk);
            return Verdict::pass();
        }
        if self.host.contains(&vk) {
            // Auto-repeat. The host keeps the key down, as the window path always did.
            return Verdict::swallow(vec![]);
        }
        self.down.insert(vk);
        if let Some(action) = Escape::from_vk(vk) {
            let (ctrl, alt, shift) = self.mods();
            if self.escape.matches(ctrl, alt, shift) {
                // Lift whatever the host holds, then keep the chord away from it until released.
                let send: Vec<_> = std::mem::take(&mut self.host).into_iter().map(|k| (k, false)).collect();
                self.ignored = self.down.difference(&self.local).copied().collect();
                return Verdict { swallow: true, send, escape: Some(action) };
            }
        }
        self.host.insert(vk);
        Verdict::swallow(vec![(vk, true)])
    }
    /// Focus lost, hook removed, or session over: the host must not keep Alt, Ctrl or Win down.
    pub fn release_all(&mut self) -> Vec<(u16, bool)> {
        let send = std::mem::take(&mut self.host).into_iter().map(|k| (k, false)).collect();
        self.down.clear();
        self.local.clear();
        self.ignored.clear();
        send
    }
}

/// Everything that has to be true before the keyboard hook may exist.
#[derive(Clone, Copy, Default)]
pub struct HookWanted {
    pub setting: bool,
    pub stream_on_screen: bool,
    pub window_focused: bool,
    /// False while a local stream menu or popup has the keyboard.
    pub keys_go_to_host: bool,
    pub keyboard_allowed: bool,
}
impl HookWanted {
    pub fn yes(self) -> bool {
        self.setting && self.stream_on_screen && self.window_focused && self.keys_go_to_host && self.keyboard_allowed
    }
}

/// Window geometry in points, outer position and inner size.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Geometry {
    pub pos: [f32; 2],
    pub size: [f32; 2],
}
#[derive(PartialEq, Debug)]
pub enum Step {
    Nothing,
    Enter,
    /// Back to a window, with the geometry from before fullscreen when it is known.
    Leave(Option<Geometry>),
}
/// Fullscreen belongs to the stream. The app window never keeps it.
#[derive(Default)]
pub struct Fullscreen {
    pub on: bool,
    saved: Option<Geometry>,
}
impl Fullscreen {
    pub fn request(&mut self, want: bool, streaming: bool, window: Option<Geometry>) -> Step {
        if want && streaming && !self.on {
            self.on = true;
            self.saved = window;
            Step::Enter
        } else if !(want && streaming) && self.on {
            self.on = false;
            Step::Leave(self.saved.take())
        } else {
            Step::Nothing
        }
    }
    /// The OS reports its fullscreen state. Outside a stream it is always forced back.
    pub fn observed(&mut self, actual: bool, streaming: bool) -> Step {
        match (actual, self.on) {
            (true, false) if streaming => {
                self.on = true;
                Step::Nothing
            }
            (true, false) => Step::Leave(self.saved.take()),
            (false, true) => {
                self.on = false;
                self.saved = None;
                Step::Nothing
            }
            _ => Step::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CTRL: u16 = VK_LCONTROL;
    const ALT: u16 = VK_LMENU;
    const SHIFT: u16 = VK_LSHIFT;

    fn filter() -> Filter {
        Filter::new(EscapeKeys::CtrlAltShift, true, [])
    }

    #[test]
    fn escape_chord_text_and_exact_modifiers() {
        assert_eq!(EscapeKeys::default(), EscapeKeys::CtrlAltShift);
        assert_eq!(EscapeKeys::CtrlAltShift.chord(Escape::Disconnect), "Ctrl+Alt+Shift+Q");
        assert_eq!(EscapeKeys::CtrlAlt.chord(Escape::Fullscreen), "Ctrl+Alt+W");
        assert_eq!(EscapeKeys::CtrlShift.chord(Escape::MenuButton), "Ctrl+Shift+S");
        assert!(EscapeKeys::CtrlAltShift.matches(true, true, true));
        assert!(!EscapeKeys::CtrlAltShift.matches(true, true, false));
        assert!(!EscapeKeys::CtrlAlt.matches(true, true, true));
        assert!(!EscapeKeys::CtrlShift.matches(true, false, false));
    }

    #[test]
    fn route_table_keeps_only_local_jobs_here() {
        for vk in [0x5B, 0x5C, 0x09, 0x1B, 0x73, 0x5D, 0x2C, 0x7A, 0x79, 0x20, 0x11, 0x12, 0xA4, 0xA6, 0xB4, 0x70] {
            assert_eq!(route(vk), Route::Host, "{vk:#x} goes to the host");
        }
        for vk in [0xAD, 0xAE, 0xAF, 0xB0, 0xB1, 0xB2, 0xB3, 0x5F, 0x15, 0x19, 0xE5, 0xE7, 0, 0xFF] {
            assert_eq!(route(vk), Route::Local, "{vk:#x} stays here");
        }
    }

    #[test]
    fn system_shortcuts_reach_the_host_as_ordered_pairs() {
        let mut f = filter();
        // Alt+Tab
        assert_eq!(f.key(ALT, true), Verdict::swallow(vec![(ALT, true)]));
        assert_eq!(f.key(0x09, true), Verdict::swallow(vec![(0x09, true)]));
        assert_eq!(f.key(0x09, false), Verdict::swallow(vec![(0x09, false)]));
        assert_eq!(f.key(ALT, false), Verdict::swallow(vec![(ALT, false)]));
        // Win alone must not open the local Start menu on release
        assert!(f.key(VK_LWIN, true).swallow);
        assert!(f.key(VK_LWIN, false).swallow);
        // Alt+F4, Ctrl+Esc, Alt+Space, PrintScreen, Menu
        for vk in [0x73, 0x1B, 0x20, 0x2C, 0x5D] {
            assert_eq!(f.key(vk, true).send, vec![(vk, true)]);
            assert_eq!(f.key(vk, false).send, vec![(vk, false)]);
        }
        assert!(f.held_on_host().is_empty());
    }

    #[test]
    fn media_keys_and_unseen_ups_stay_local() {
        let mut f = filter();
        assert_eq!(f.key(0xAF, true), Verdict::pass());
        assert_eq!(f.key(0xAF, false), Verdict::pass());
        // an up for a key this hook never saw down is not the host's
        assert_eq!(f.key(0x41, false), Verdict::pass());
    }

    #[test]
    fn keys_held_before_the_hook_stay_local_until_released() {
        let mut f = Filter::new(EscapeKeys::CtrlAltShift, true, [ALT]);
        assert_eq!(f.key(ALT, true), Verdict::pass(), "repeat of a pre-held key");
        assert_eq!(f.key(ALT, false), Verdict::pass());
        assert_eq!(f.key(ALT, true), Verdict::swallow(vec![(ALT, true)]), "a fresh press is the host's");
    }

    #[test]
    fn keyboard_control_off_touches_nothing() {
        let mut f = Filter::new(EscapeKeys::CtrlAltShift, false, []);
        for (vk, down) in [(VK_LWIN, true), (0x09, true), (CTRL, true), (ALT, true), (SHIFT, true), (0x51, true)] {
            assert_eq!(f.key(vk, down), Verdict::pass());
        }
    }

    #[test]
    fn repeat_is_swallowed_without_resending() {
        let mut f = filter();
        assert_eq!(f.key(0x41, true).send, vec![(0x41, true)]);
        assert_eq!(f.key(0x41, true), Verdict::swallow(vec![]));
        assert_eq!(f.key(0x41, false).send, vec![(0x41, false)]);
    }

    #[test]
    fn escape_chord_is_never_forwarded_and_clears_the_host() {
        let mut f = filter();
        for vk in [CTRL, ALT, SHIFT] {
            assert!(f.key(vk, true).swallow);
        }
        let v = f.key(0x51, true);
        assert!(v.swallow);
        assert_eq!(v.escape, Some(Escape::Disconnect));
        let mut lifted: Vec<_> = v.send.clone();
        lifted.sort();
        assert_eq!(lifted, vec![(SHIFT, false), (CTRL, false), (ALT, false)]);
        assert!(f.held_on_host().is_empty());
        // The held chord stays away from the host until released, repeats do not fire again.
        assert_eq!(f.key(0x51, true), Verdict::swallow(vec![]));
        assert_eq!(f.key(0x51, false), Verdict::swallow(vec![]));
        assert_eq!(f.key(CTRL, false), Verdict::swallow(vec![]));
        assert_eq!(f.key(ALT, false), Verdict::swallow(vec![]));
        assert_eq!(f.key(SHIFT, false), Verdict::swallow(vec![]));
        // Everything is back to normal afterwards.
        assert_eq!(f.key(0x41, true).send, vec![(0x41, true)]);
    }

    #[test]
    fn near_misses_of_the_chord_go_to_the_host() {
        let mut f = filter();
        f.key(CTRL, true);
        f.key(ALT, true);
        let v = f.key(0x51, true);
        assert_eq!(v.escape, None, "Ctrl+Alt+Q is the host's with the default chord");
        assert_eq!(v.send, vec![(0x51, true)]);
        // the Win key in the mix makes it a host chord too
        let mut f = filter();
        for vk in [CTRL, ALT, SHIFT, VK_LWIN] {
            f.key(vk, true);
        }
        assert_eq!(f.key(0x51, true).escape, None);
    }

    #[test]
    fn each_escape_action_and_alternative_modifiers() {
        for (keys, mods) in [
            (EscapeKeys::CtrlAltShift, vec![CTRL, ALT, SHIFT]),
            (EscapeKeys::CtrlShift, vec![VK_RCONTROL, VK_RSHIFT]),
            (EscapeKeys::CtrlAlt, vec![CTRL, VK_RMENU]),
        ] {
            for (vk, action) in [(0x57, Escape::Fullscreen), (0x51, Escape::Disconnect), (0x53, Escape::MenuButton)] {
                let mut f = Filter::new(keys, true, []);
                for m in &mods {
                    f.key(*m, true);
                }
                assert_eq!(f.key(vk, true).escape, Some(action));
            }
        }
    }

    #[test]
    fn release_all_lifts_every_held_key_exactly_once() {
        let mut f = filter();
        for vk in [ALT, VK_LWIN, 0x09] {
            f.key(vk, true);
        }
        let mut up = f.release_all();
        up.sort();
        assert_eq!(up, vec![(0x09, false), (VK_LWIN, false), (ALT, false)]);
        assert!(f.release_all().is_empty());
        assert!(f.held_on_host().is_empty());
    }

    #[test]
    fn hook_is_wanted_only_when_everything_holds() {
        let all = HookWanted { setting: true, stream_on_screen: true, window_focused: true, keys_go_to_host: true, keyboard_allowed: true };
        assert!(all.yes());
        assert!(!HookWanted { setting: false, ..all }.yes());
        assert!(!HookWanted { stream_on_screen: false, ..all }.yes());
        assert!(!HookWanted { window_focused: false, ..all }.yes());
        assert!(!HookWanted { keys_go_to_host: false, ..all }.yes());
        assert!(!HookWanted { keyboard_allowed: false, ..all }.yes());
        assert!(!HookWanted::default().yes());
    }

    fn geo(x: f32) -> Option<Geometry> {
        Some(Geometry { pos: [x, 20.], size: [800., 600.] })
    }
    #[test]
    fn fullscreen_is_only_for_the_stream() {
        let mut fs = Fullscreen::default();
        assert_eq!(fs.request(true, false, geo(5.)), Step::Nothing, "the app window never goes fullscreen");
        assert!(!fs.on);
        assert_eq!(fs.request(true, true, geo(5.)), Step::Enter);
        assert!(fs.on);
        assert_eq!(fs.request(true, true, geo(9.)), Step::Nothing);
        // the stream ends: back to a window with the old geometry
        assert_eq!(fs.request(false, false, None), Step::Leave(geo(5.)));
        assert!(!fs.on);
        assert_eq!(fs.request(false, false, None), Step::Nothing);
    }

    #[test]
    fn os_fullscreen_outside_a_stream_is_undone() {
        let mut fs = Fullscreen::default();
        assert_eq!(fs.observed(true, false), Step::Leave(None));
        assert!(!fs.on);
        assert_eq!(fs.observed(true, true), Step::Nothing, "a stream adopts it");
        assert!(fs.on);
        assert_eq!(fs.observed(false, true), Step::Nothing);
        assert!(!fs.on);
        assert_eq!(fs.observed(false, false), Step::Nothing);
    }

    #[test]
    fn leaving_the_stream_while_fullscreen_restores_the_window() {
        let mut fs = Fullscreen::default();
        fs.request(true, true, geo(40.));
        assert_eq!(fs.request(true, false, None), Step::Leave(geo(40.)));
    }
}

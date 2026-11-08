//! Media mode types shared by every host: the command set, the pushed state and change detection.

use crate::proto::ServerMsg;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    PlayPause,
    Next,
    Prev,
    VolUp,
    VolDown,
    Mute,
    VolSet(u8),
}

impl Cmd {
    pub fn parse(c: &str, v: Option<u8>) -> Result<Self, &'static str> {
        Ok(match c {
            "play_pause" => Cmd::PlayPause,
            "next" => Cmd::Next,
            "prev" => Cmd::Prev,
            "vol_up" => Cmd::VolUp,
            "vol_down" => Cmd::VolDown,
            "mute" => Cmd::Mute,
            "vol_set" => Cmd::VolSet(v.ok_or("bad_request")?.min(100)),
            _ => return Err("bad_request"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaState {
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
    pub vol: u8,
    pub muted: bool,
}

/// Remembers the last pushed state so `media` is only sent on change.
#[derive(Default)]
pub struct Diff(Option<MediaState>);

impl Diff {
    pub fn changed(&mut self, s: MediaState) -> Option<ServerMsg> {
        if self.0.as_ref() == Some(&s) {
            return None;
        }
        self.0 = Some(s.clone());
        Some(ServerMsg::Media { title: s.title, artist: s.artist, app: s.app, playing: s.playing, vol: s.vol, muted: s.muted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(Cmd::parse("play_pause", None), Ok(Cmd::PlayPause));
        assert_eq!(Cmd::parse("next", None), Ok(Cmd::Next));
        assert_eq!(Cmd::parse("prev", None), Ok(Cmd::Prev));
        assert_eq!(Cmd::parse("vol_up", None), Ok(Cmd::VolUp));
        assert_eq!(Cmd::parse("vol_down", None), Ok(Cmd::VolDown));
        assert_eq!(Cmd::parse("mute", None), Ok(Cmd::Mute));
        assert_eq!(Cmd::parse("vol_set", Some(45)), Ok(Cmd::VolSet(45)));
        assert_eq!(Cmd::parse("vol_set", None), Err("bad_request"));
        assert_eq!(Cmd::parse("rewind", None), Err("bad_request"));
    }

    #[test]
    fn vol_set_is_clamped() {
        assert_eq!(Cmd::parse("vol_set", Some(250)), Ok(Cmd::VolSet(100)));
        assert_eq!(Cmd::parse("vol_set", Some(0)), Ok(Cmd::VolSet(0)));
    }

    fn st(title: &str, vol: u8) -> MediaState {
        MediaState { title: title.into(), artist: "A".into(), app: "Spotify".into(), playing: true, vol, muted: false }
    }

    #[test]
    fn diff_pushes_only_on_change() {
        let mut d = Diff::default();
        assert!(d.changed(st("x", 10)).is_some(), "first state is always pushed");
        assert!(d.changed(st("x", 10)).is_none());
        let m = d.changed(st("y", 10)).unwrap();
        assert_eq!(m, ServerMsg::Media { title: "y".into(), artist: "A".into(), app: "Spotify".into(), playing: true, vol: 10, muted: false });
        assert!(d.changed(st("y", 11)).is_some());
    }
}

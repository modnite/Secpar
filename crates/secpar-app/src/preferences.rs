//! What the person chose for the stream, kept between runs.
use serde::{de, Deserialize, Deserializer, Serialize};
use std::path::Path;

/// Qt writes whole numbers as JSON floats; accept those and plain strings too.
fn integer<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Whole(u32),
        Float(f64),
        Text(String),
    }
    match Number::deserialize(deserializer)? {
        Number::Whole(value) => Ok(value),
        Number::Float(value) if value.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(&value) => Ok(value as u32),
        Number::Float(_) => Err(de::Error::custom("whole number")),
        Number::Text(text) => text.parse().map_err(de::Error::custom),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default = "default_resolution")]
    pub resolution: String,
    #[serde(default = "default_fps", deserialize_with = "integer")]
    pub fps: u32,
    #[serde(default = "default_bitrate", deserialize_with = "integer")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_decoder")]
    pub decoder: String,
    #[serde(default = "default_display")]
    pub display: String,
    #[serde(default = "default_audio")]
    pub audio: String,
    #[serde(default)]
    pub mute_on_focus_loss: bool,
    /// Keep this computer's clipboard text in step with the connected computer.
    /// On unless the person turns it off.
    #[serde(default = "default_share_clipboard")]
    pub share_clipboard: bool,
}
fn default_resolution() -> String { "1920x1080".into() }
fn default_fps() -> u32 { 60 }
fn default_bitrate() -> u32 { 20_000 }
fn default_decoder() -> String { "auto".into() }
fn default_display() -> String { "windowed".into() }
fn default_audio() -> String { "stereo".into() }
fn default_share_clipboard() -> bool { true }

impl Default for Preferences {
    fn default() -> Self {
        Self {
            resolution: default_resolution(),
            fps: default_fps(),
            bitrate_kbps: default_bitrate(),
            decoder: default_decoder(),
            display: default_display(),
            audio: default_audio(),
            mute_on_focus_loss: false,
            share_clipboard: default_share_clipboard(),
        }
    }
}

impl Preferences {
    /// A saved file that cannot be read or is out of range falls back to the
    /// defaults instead of stopping the window.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
            .and_then(|preferences| preferences.checked().ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    /// The stream component only accepts these ranges and names. Reject
    /// anything else before it reaches a command line.
    pub fn checked(self) -> Result<Self, &'static str> {
        let (width, height) = self.resolution.split_once('x').ok_or("resolution")?;
        let (width, height): (u32, u32) = (width.parse().map_err(|_| "resolution")?, height.parse().map_err(|_| "resolution")?);
        if !(320..=7680).contains(&width) || !(320..=7680).contains(&height) || width % 2 != 0 || height % 2 != 0 {
            return Err("resolution");
        }
        if !(10..=240).contains(&self.fps) {
            return Err("fps");
        }
        if !(500..=200_000).contains(&self.bitrate_kbps) {
            return Err("bitrate");
        }
        if !["auto", "hardware", "software"].contains(&self.decoder.as_str()) {
            return Err("decoder");
        }
        if !["windowed", "fullscreen", "borderless"].contains(&self.display.as_str()) {
            return Err("display");
        }
        if !["stereo", "5.1-surround", "7.1-surround"].contains(&self.audio.as_str()) {
            return Err("audio");
        }
        Ok(self)
    }

    /// The component's command-line arguments, without the component or host.
    pub fn stream_arguments(&self, address: &str) -> Vec<String> {
        let mut arguments = vec![
            "stream".into(),
            address.into(),
            "Desktop".into(),
            "--packet-size".into(),
            "1024".into(),
            "--resolution".into(),
            self.resolution.clone(),
            "--fps".into(),
            self.fps.to_string(),
            "--bitrate".into(),
            self.bitrate_kbps.to_string(),
            "--video-decoder".into(),
            self.decoder.clone(),
            "--display-mode".into(),
            self.display.clone(),
            "--audio-config".into(),
            self.audio.clone(),
            "--absolute-mouse".into(),
            "--keep-awake".into(),
            "--frame-pacing".into(),
            "--capture-system-keys".into(),
            "fullscreen".into(),
        ];
        arguments.push(if self.mute_on_focus_loss { "--mute-on-focus-loss".into() } else { "--no-mute-on-focus-loss".into() });
        arguments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_reach_the_component() {
        let preferences = Preferences::default().checked().unwrap();
        let arguments = preferences.stream_arguments("127.0.0.2:46989");
        assert!(arguments.windows(2).any(|pair| pair == ["--resolution", "1920x1080"]));
        assert!(arguments.windows(2).any(|pair| pair == ["--fps", "60"]));
        assert!(arguments.windows(2).any(|pair| pair == ["--bitrate", "20000"]));
        assert!(arguments.contains(&"--no-mute-on-focus-loss".to_string()));
    }

    #[test]
    fn rejected_values_do_not_reach_a_command_line() {
        let mut preferences = Preferences::default();
        preferences.fps = 500;
        assert_eq!(preferences.checked().unwrap_err(), "fps");
        let mut preferences = Preferences::default();
        preferences.decoder = "gpu".into();
        assert_eq!(preferences.checked().unwrap_err(), "decoder");
        let mut preferences = Preferences::default();
        preferences.resolution = "1921x1080".into();
        assert_eq!(preferences.checked().unwrap_err(), "resolution");
    }

    #[test]
    fn a_saved_file_round_trips_and_a_bad_one_falls_back() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("preferences.json");
        let chosen = Preferences { resolution: "1280x720".into(), fps: 120, bitrate_kbps: 40_000, decoder: "hardware".into(), display: "fullscreen".into(), audio: "5.1-surround".into(), mute_on_focus_loss: true, share_clipboard: false };
        chosen.save(&path).unwrap();
        assert_eq!(Preferences::load(&path), chosen.checked().unwrap());
        std::fs::write(&path, b"{\"fps\":999}").unwrap();
        assert_eq!(Preferences::load(&path), Preferences::default());
    }

    #[test]
    fn a_file_from_before_clipboard_sharing_turns_it_on() {
        let saved: Preferences = serde_json::from_str(r#"{"resolution":"1280x720"}"#).unwrap();
        assert!(saved.share_clipboard);
        assert!(!serde_json::from_str::<Preferences>(r#"{"share_clipboard":false}"#).unwrap().share_clipboard);
    }

    #[test]
    fn integral_numbers_written_by_qt_are_accepted() {
        let preferences: Preferences = serde_json::from_str(r#"{"fps":120.0,"bitrate_kbps":40000.0}"#).unwrap();
        assert_eq!((preferences.fps, preferences.bitrate_kbps), (120, 40_000));
    }
}

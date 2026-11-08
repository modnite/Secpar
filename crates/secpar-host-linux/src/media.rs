//! Media mode on Linux: now playing and transport through MPRIS (the D-Bus interface that
//! Spotify, Firefox, Chrome, VLC, mpv and the KDE and GNOME players speak), and system volume
//! through wpctl (PipeWire) or pactl (PulseAudio). The wire messages match the Windows host.
use crate::push::Push;
use secpar_host_core::media::{Cmd, Diff, MediaState};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

/// A future that may borrow the object it came from.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One MPRIS player as found on the session bus.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Player {
    /// Bus name, e.g. `org.mpris.MediaPlayer2.spotify`.
    pub bus: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

/// The session bus side. The real one is D-Bus; tests use a fake.
pub trait Mpris: Send + Sync {
    fn players(&self) -> Fut<'_, Vec<Player>>;
    /// Calls a method of `org.mpris.MediaPlayer2.Player` (`PlayPause`, `Next`, `Previous`).
    fn call(&self, bus: String, method: &'static str) -> Fut<'_, bool>;
}

/// The system volume. Blocking calls: run them on a blocking thread.
pub trait Mixer: Send + Sync {
    /// Volume 0..=100 and mute, or None when no mixer tool works.
    fn read(&self) -> Option<(u8, bool)>;
    fn apply(&self, cmd: Cmd) -> bool;
}

/// Which player the phone sees: one that is playing, else the first with something loaded.
pub fn pick(mut players: Vec<Player>) -> Option<Player> {
    players.sort_by(|a, b| a.bus.cmp(&b.bus));
    let playing = players.iter().position(|p| p.playing);
    let loaded = players.iter().position(|p| !p.title.is_empty());
    let index = playing.or(loaded).or((!players.is_empty()).then_some(0))?;
    Some(players.swap_remove(index))
}

/// `org.mpris.MediaPlayer2.firefox.instance_1_42` becomes `firefox`.
pub fn app_from_bus(bus: &str) -> String {
    let name = bus.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(bus);
    name.split('.').next().unwrap_or(name).to_string()
}

pub struct Media {
    mpris: Box<dyn Mpris>,
    mixer: Arc<dyn Mixer>,
}

impl Media {
    pub fn new(mpris: Box<dyn Mpris>, mixer: Arc<dyn Mixer>) -> Self {
        Media { mpris, mixer }
    }

    /// The real D-Bus and the real mixer tools.
    pub fn system() -> Self {
        Self::new(Box::new(DbusMpris::default()), Arc::new(CommandMixer::new(SystemRunner)))
    }

    /// Now playing plus volume, in the shape of the Windows host's `media` message.
    pub async fn read(&self) -> MediaState {
        let mixer = self.mixer.clone();
        let volume = tokio::task::spawn_blocking(move || mixer.read()).await.ok().flatten();
        let mut state = MediaState::default();
        if let Some(p) = pick(self.mpris.players().await) {
            state.app = if p.app.is_empty() { app_from_bus(&p.bus) } else { p.app };
            state.title = p.title;
            state.artist = p.artist;
            state.playing = p.playing;
        }
        if let Some((vol, muted)) = volume {
            state.vol = vol;
            state.muted = muted;
        }
        state
    }

    /// Runs a phone command. Transport goes to the chosen player; with no player it does nothing.
    pub async fn run(&self, cmd: Cmd) {
        let method = match cmd {
            Cmd::PlayPause => "PlayPause",
            Cmd::Next => "Next",
            Cmd::Prev => "Previous",
            _ => {
                let mixer = self.mixer.clone();
                let _ = tokio::task::spawn_blocking(move || mixer.apply(cmd)).await;
                return;
            }
        };
        if let Some(player) = pick(self.mpris.players().await) {
            self.mpris.call(player.bus, method).await;
        }
    }
}

/// While subscribed: push `media` once now, then whenever it changes (checked once a second).
pub fn subscribe<P: Push>(media: Arc<Media>, out: P) -> tokio::task::JoinHandle<()> {
    subscribe_every(media, out, Duration::from_secs(1))
}

fn subscribe_every<P: Push>(media: Arc<Media>, out: P, period: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut diff = Diff::default();
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while !out.closed() {
            tick.tick().await;
            if let Some(m) = diff.changed(media.read().await) {
                if !out.msg(m) {
                    break;
                }
            }
        }
    })
}

// ---- D-Bus ----

/// MPRIS over the session bus with zbus (already part of the portal stack).
#[derive(Default)]
pub struct DbusMpris {
    conn: tokio::sync::OnceCell<zbus::Connection>,
}

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const TIMEOUT: Duration = Duration::from_secs(2);

/// A string out of a D-Bus value: `s`, or the first entry of `as`.
fn text(value: &zbus::zvariant::Value<'_>) -> String {
    use zbus::zvariant::Value;
    match value {
        Value::Str(s) => s.to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(|v| if let Value::Str(s) = v { Some(s.to_string()) } else { None })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Value(inner) => text(inner),
        _ => String::new(),
    }
}

impl DbusMpris {
    async fn connection(&self) -> Option<&zbus::Connection> {
        self.conn.get_or_try_init(|| async { zbus::Connection::session().await }).await.ok()
    }

    async fn properties(
        conn: &zbus::Connection,
        bus: &str,
        interface: &str,
    ) -> Option<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> {
        let reply = conn
            .call_method(Some(bus), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(interface,))
            .await
            .ok()?;
        reply.body().deserialize().ok()
    }

    async fn player(conn: &zbus::Connection, bus: &str) -> Option<Player> {
        let state = Self::properties(conn, bus, PLAYER).await?;
        let root = Self::properties(conn, bus, "org.mpris.MediaPlayer2").await.unwrap_or_default();
        let get = |map: &std::collections::HashMap<String, zbus::zvariant::OwnedValue>, key: &str| map.get(key).map(|v| text(v)).unwrap_or_default();
        let (mut title, mut artist) = (String::new(), String::new());
        if let Some(metadata) = state.get("Metadata") {
            if let zbus::zvariant::Value::Dict(dict) = &**metadata {
                for (key, value) in dict.iter() {
                    match text(key).as_str() {
                        "xesam:title" => title = text(value),
                        "xesam:artist" => artist = text(value),
                        _ => {}
                    }
                }
            }
        }
        Some(Player { bus: bus.into(), app: get(&root, "Identity"), title, artist, playing: get(&state, "PlaybackStatus") == "Playing" })
    }
}

impl Mpris for DbusMpris {
    fn players(&self) -> Fut<'_, Vec<Player>> {
        Box::pin(async move {
            let Some(conn) = self.connection().await else { return vec![] };
            let Ok(Ok(names)) = tokio::time::timeout(TIMEOUT, async {
                let reply = conn
                    .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
                    .await?;
                reply.body().deserialize::<Vec<String>>()
            })
            .await
            else {
                return vec![];
            };
            let mut found = Vec::new();
            for bus in names.iter().filter(|n| n.starts_with(PREFIX)).take(16) {
                if let Ok(Some(player)) = tokio::time::timeout(TIMEOUT, Self::player(conn, bus)).await {
                    found.push(player);
                }
            }
            found
        })
    }

    fn call(&self, bus: String, method: &'static str) -> Fut<'_, bool> {
        Box::pin(async move {
            let Some(conn) = self.connection().await else { return false };
            matches!(
                tokio::time::timeout(TIMEOUT, conn.call_method(Some(bus.as_str()), PATH, Some(PLAYER), method, &())).await,
                Ok(Ok(_))
            )
        })
    }
}

// ---- Volume ----

/// Runs a program and returns its stdout when it succeeded.
pub trait Runner: Send + Sync + 'static {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let output = std::process::Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// wpctl first (PipeWire), pactl when that is missing or fails.
pub struct CommandMixer<R: Runner> {
    runner: R,
}

const WP_SINK: &str = "@DEFAULT_AUDIO_SINK@";
const PA_SINK: &str = "@DEFAULT_SINK@";
/// Volume step for `vol_up` and `vol_down`, in percent.
const STEP: u8 = 5;

/// `Volume: 0.45` and `Volume: 0.45 [MUTED]`.
pub fn parse_wpctl(out: &str) -> Option<(u8, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?;
    let value: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some(((value * 100.0).round().clamp(0.0, 100.0) as u8, rest.contains("[MUTED]")))
}

/// The first `NN%` of `pactl get-sink-volume`.
pub fn parse_pactl_volume(out: &str) -> Option<u8> {
    let percent = out.split_whitespace().find(|w| w.ends_with('%'))?;
    Some(percent.trim_end_matches('%').parse::<u32>().ok()?.min(100) as u8)
}

/// `Mute: yes` or `Mute: no`.
pub fn parse_pactl_mute(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

impl<R: Runner> CommandMixer<R> {
    pub fn new(runner: R) -> Self {
        CommandMixer { runner }
    }
}

impl<R: Runner> Mixer for CommandMixer<R> {
    fn read(&self) -> Option<(u8, bool)> {
        if let Some(state) = self.runner.run("wpctl", &["get-volume", WP_SINK]).and_then(|o| parse_wpctl(&o)) {
            return Some(state);
        }
        let volume = self.runner.run("pactl", &["get-sink-volume", PA_SINK]).and_then(|o| parse_pactl_volume(&o))?;
        let muted = self.runner.run("pactl", &["get-sink-mute", PA_SINK]).and_then(|o| parse_pactl_mute(&o)).unwrap_or(false);
        Some((volume, muted))
    }

    fn apply(&self, cmd: Cmd) -> bool {
        let up = format!("{STEP}%+");
        let down = format!("{STEP}%-");
        let (wp, pa): (Vec<String>, Vec<String>) = match cmd {
            Cmd::VolUp => (
                ["set-volume", "-l", "1.0", WP_SINK, up.as_str()].map(String::from).to_vec(),
                ["set-sink-volume", PA_SINK, &format!("+{STEP}%")].map(String::from).to_vec(),
            ),
            Cmd::VolDown => (
                ["set-volume", WP_SINK, down.as_str()].map(String::from).to_vec(),
                ["set-sink-volume", PA_SINK, &format!("-{STEP}%")].map(String::from).to_vec(),
            ),
            Cmd::Mute => (
                ["set-mute", WP_SINK, "toggle"].map(String::from).to_vec(),
                ["set-sink-mute", PA_SINK, "toggle"].map(String::from).to_vec(),
            ),
            Cmd::VolSet(v) => {
                let v = v.min(100);
                (
                    ["set-volume", WP_SINK, &format!("{:.2}", v as f64 / 100.0)].map(String::from).to_vec(),
                    ["set-sink-volume", PA_SINK, &format!("{v}%")].map(String::from).to_vec(),
                )
            }
            Cmd::PlayPause | Cmd::Next | Cmd::Prev => return false,
        };
        fn args(v: &[String]) -> Vec<&str> {
            v.iter().map(String::as_str).collect()
        }
        self.runner.run("wpctl", &args(&wp)).is_some() || self.runner.run("pactl", &args(&pa)).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn player(bus: &str, title: &str, playing: bool) -> Player {
        Player { bus: bus.into(), app: String::new(), title: title.into(), artist: "A".into(), playing }
    }

    #[test]
    fn the_playing_player_wins_then_the_one_with_a_title() {
        let all = vec![player("org.mpris.MediaPlayer2.b", "", false), player("org.mpris.MediaPlayer2.c", "Song", false), player("org.mpris.MediaPlayer2.d", "Live", true)];
        assert_eq!(pick(all.clone()).unwrap().bus, "org.mpris.MediaPlayer2.d");
        assert_eq!(pick(all[..2].to_vec()).unwrap().bus, "org.mpris.MediaPlayer2.c");
        assert_eq!(pick(all[..1].to_vec()).unwrap().bus, "org.mpris.MediaPlayer2.b");
        assert_eq!(pick(vec![]), None);
    }

    #[test]
    fn app_name_comes_from_the_bus_name() {
        assert_eq!(app_from_bus("org.mpris.MediaPlayer2.spotify"), "spotify");
        assert_eq!(app_from_bus("org.mpris.MediaPlayer2.firefox.instance_1_42"), "firefox");
        assert_eq!(app_from_bus("org.mpris.MediaPlayer2.vlc"), "vlc");
        assert_eq!(app_from_bus("plain"), "plain");
    }

    #[test]
    fn dbus_values_become_text() {
        use zbus::zvariant::{Array, Value};
        assert_eq!(text(&Value::from("Song")), "Song");
        let artists = Value::from(vec!["A", "B"]);
        assert_eq!(text(&artists), "A, B");
        assert_eq!(text(&Value::Array(Array::from(Vec::<String>::new()))), "");
        assert_eq!(text(&Value::from(7u32)), "");
        assert_eq!(text(&Value::Value(Box::new(Value::from("wrapped")))), "wrapped");
    }

    struct FakeMpris {
        players: Vec<Player>,
        calls: Arc<Mutex<Vec<(String, &'static str)>>>,
    }
    impl Mpris for FakeMpris {
        fn players(&self) -> Fut<'_, Vec<Player>> {
            let p = self.players.clone();
            Box::pin(async move { p })
        }
        fn call(&self, bus: String, method: &'static str) -> Fut<'_, bool> {
            self.calls.lock().unwrap().push((bus, method));
            Box::pin(async { true })
        }
    }
    #[derive(Default)]
    struct FakeMixer {
        volume: Mutex<Option<(u8, bool)>>,
        applied: Mutex<Vec<Cmd>>,
    }
    impl Mixer for FakeMixer {
        fn read(&self) -> Option<(u8, bool)> {
            *self.volume.lock().unwrap()
        }
        fn apply(&self, cmd: Cmd) -> bool {
            self.applied.lock().unwrap().push(cmd);
            true
        }
    }

    fn rig(players: Vec<Player>) -> (Media, Arc<Mutex<Vec<(String, &'static str)>>>, Arc<FakeMixer>) {
        let calls = Arc::new(Mutex::new(vec![]));
        let mixer = Arc::new(FakeMixer::default());
        *mixer.volume.lock().unwrap() = Some((45, true));
        (Media::new(Box::new(FakeMpris { players, calls: calls.clone() }), mixer.clone()), calls, mixer)
    }

    #[tokio::test]
    async fn state_combines_player_and_volume_like_the_windows_host() {
        let mut spotify = player("org.mpris.MediaPlayer2.spotify", "Song", true);
        spotify.app = "Spotify".into();
        let (media, _, _) = rig(vec![spotify]);
        assert_eq!(
            media.read().await,
            MediaState { title: "Song".into(), artist: "A".into(), app: "Spotify".into(), playing: true, vol: 45, muted: true }
        );
        let (media, _, mixer) = rig(vec![player("org.mpris.MediaPlayer2.vlc", "", false)]);
        *mixer.volume.lock().unwrap() = None;
        let state = media.read().await;
        assert_eq!((state.app.as_str(), state.vol, state.muted, state.playing), ("vlc", 0, false, false), "app falls back to the bus name; no mixer means 0");
        let (media, _, _) = rig(vec![]);
        let state = media.read().await;
        assert_eq!((state.title.as_str(), state.app.as_str(), state.vol), ("", "", 45), "no player still reports volume");
    }

    #[tokio::test]
    async fn transport_goes_to_the_player_and_volume_to_the_mixer() {
        let (media, calls, mixer) = rig(vec![player("org.mpris.MediaPlayer2.a", "", false), player("org.mpris.MediaPlayer2.b", "x", true)]);
        media.run(Cmd::PlayPause).await;
        media.run(Cmd::Next).await;
        media.run(Cmd::Prev).await;
        media.run(Cmd::VolUp).await;
        media.run(Cmd::VolSet(30)).await;
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                ("org.mpris.MediaPlayer2.b".to_string(), "PlayPause"),
                ("org.mpris.MediaPlayer2.b".to_string(), "Next"),
                ("org.mpris.MediaPlayer2.b".to_string(), "Previous"),
            ]
        );
        assert_eq!(*mixer.applied.lock().unwrap(), vec![Cmd::VolUp, Cmd::VolSet(30)]);
        let (media, calls, _) = rig(vec![]);
        media.run(Cmd::PlayPause).await;
        assert!(calls.lock().unwrap().is_empty(), "no player: nothing to control");
    }

    #[test]
    fn wpctl_and_pactl_output_is_parsed() {
        assert_eq!(parse_wpctl("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_wpctl("Volume: 0.45 [MUTED]\n"), Some((45, true)));
        assert_eq!(parse_wpctl("Volume: 1.50\n"), Some((100, false)), "over-amplified shows as 100");
        assert_eq!(parse_wpctl("Volume: 0.00 [MUTED]"), Some((0, true)));
        assert_eq!(parse_wpctl("nonsense"), None);
        let line = "Volume: front-left: 29491 /  45% / -20.00 dB,   front-right: 29491 /  45% / -20.00 dB\n        balance 0.00";
        assert_eq!(parse_pactl_volume(line), Some(45));
        assert_eq!(parse_pactl_volume("Volume: mono: 70000 / 107% / 1.80 dB"), Some(100));
        assert_eq!(parse_pactl_volume("nothing"), None);
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no\n"), Some(false));
        assert_eq!(parse_pactl_mute("?"), None);
    }

    /// Answers by program name; records every call.
    struct FakeRunner {
        wpctl: Option<String>,
        pactl_volume: Option<String>,
        pactl_mute: Option<String>,
        log: Mutex<Vec<String>>,
    }
    impl FakeRunner {
        fn new(wpctl: Option<&str>, pactl_volume: Option<&str>, pactl_mute: Option<&str>) -> Self {
            FakeRunner { wpctl: wpctl.map(Into::into), pactl_volume: pactl_volume.map(Into::into), pactl_mute: pactl_mute.map(Into::into), log: Mutex::default() }
        }
    }
    impl Runner for Arc<FakeRunner> {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            self.log.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            match (program, args.first().copied()) {
                ("wpctl", _) => self.wpctl.clone(),
                ("pactl", Some("get-sink-volume")) => self.pactl_volume.clone(),
                ("pactl", Some("get-sink-mute")) => self.pactl_mute.clone(),
                ("pactl", _) => self.pactl_volume.as_ref().map(|_| String::new()),
                _ => None,
            }
        }
    }

    #[test]
    fn mixer_prefers_wpctl_and_falls_back_to_pactl() {
        let runner = Arc::new(FakeRunner::new(Some("Volume: 0.30 [MUTED]"), Some("Volume: 50%"), Some("Mute: no")));
        let mixer = CommandMixer::new(runner.clone());
        assert_eq!(mixer.read(), Some((30, true)));
        assert_eq!(runner.log.lock().unwrap().as_slice(), ["wpctl get-volume @DEFAULT_AUDIO_SINK@"]);
        let runner = Arc::new(FakeRunner::new(None, Some("Volume: front-left: 1 /  50% / 0 dB"), Some("Mute: yes")));
        assert_eq!(CommandMixer::new(runner.clone()).read(), Some((50, true)));
        assert_eq!(
            runner.log.lock().unwrap().as_slice(),
            ["wpctl get-volume @DEFAULT_AUDIO_SINK@", "pactl get-sink-volume @DEFAULT_SINK@", "pactl get-sink-mute @DEFAULT_SINK@"]
        );
        assert_eq!(CommandMixer::new(Arc::new(FakeRunner::new(None, None, None))).read(), None);
    }

    #[test]
    fn volume_commands_use_the_expected_arguments() {
        let run = |cmd: Cmd, wpctl: bool| {
            let runner = Arc::new(FakeRunner::new(wpctl.then(String::new).as_deref(), Some("x"), None));
            let ok = CommandMixer::new(runner.clone()).apply(cmd);
            let log = runner.log.lock().unwrap().clone();
            (ok, log)
        };
        assert_eq!(run(Cmd::VolUp, true), (true, vec!["wpctl set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 5%+".to_string()]));
        assert_eq!(run(Cmd::VolDown, true).1, ["wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"]);
        assert_eq!(run(Cmd::Mute, true).1, ["wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"]);
        assert_eq!(run(Cmd::VolSet(45), true).1, ["wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.45"]);
        assert_eq!(run(Cmd::VolSet(250), true).1, ["wpctl set-volume @DEFAULT_AUDIO_SINK@ 1.00"], "clamped to 100");
        assert_eq!(run(Cmd::VolSet(45), false), (true, vec!["wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.45".to_string(), "pactl set-sink-volume @DEFAULT_SINK@ 45%".to_string()]));
        assert_eq!(run(Cmd::VolUp, false).1[1], "pactl set-sink-volume @DEFAULT_SINK@ +5%");
        assert_eq!(run(Cmd::Mute, false).1[1], "pactl set-sink-mute @DEFAULT_SINK@ toggle");
        assert!(!run(Cmd::PlayPause, true).0, "transport is not a mixer command");
    }

    async fn until(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..200 {
            if ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn subscription_pushes_once_then_only_on_change_and_stops_with_the_connection() {
        use crate::push::fake::Fake;
        use secpar_host_core::proto::ServerMsg;
        let (media, _, mixer) = rig(vec![player("org.mpris.MediaPlayer2.a", "One", true)]);
        let out = Fake::default();
        let task = subscribe_every(Arc::new(media), out.clone(), Duration::from_millis(20));
        until("first push", || out.msgs.lock().unwrap().len() == 1).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(out.msgs.lock().unwrap().len(), 1, "nothing more while unchanged");
        assert!(matches!(&out.msgs.lock().unwrap()[0], ServerMsg::Media { title, vol: 45, muted: true, playing: true, .. } if title == "One"));
        *mixer.volume.lock().unwrap() = Some((46, true));
        until("volume push", || out.msgs.lock().unwrap().len() == 2).await;
        out.close();
        until("loop end", || task.is_finished()).await;
    }

    // ---- real D-Bus: a private session bus with a fake MPRIS player ----

    struct FakeRoot;
    #[zbus::interface(name = "org.mpris.MediaPlayer2")]
    impl FakeRoot {
        #[zbus(property)]
        fn identity(&self) -> String {
            "Test Player".into()
        }
    }
    struct FakePlayer(Arc<Mutex<Vec<String>>>);
    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        fn play_pause(&self) {
            self.0.lock().unwrap().push("PlayPause".into());
        }
        fn next(&self) {
            self.0.lock().unwrap().push("Next".into());
        }
        fn previous(&self) {
            self.0.lock().unwrap().push("Previous".into());
        }
        #[zbus(property)]
        fn playback_status(&self) -> String {
            "Playing".into()
        }
        #[zbus(property)]
        fn metadata(&self) -> std::collections::HashMap<String, zbus::zvariant::OwnedValue> {
            use zbus::zvariant::{ObjectPath, OwnedValue, Value};
            let mut m = std::collections::HashMap::new();
            m.insert("xesam:title".to_string(), OwnedValue::try_from(Value::from("Song")).unwrap());
            m.insert("xesam:artist".to_string(), OwnedValue::try_from(Value::from(vec!["A", "B"])).unwrap());
            m.insert("mpris:trackid".to_string(), OwnedValue::try_from(Value::from(ObjectPath::try_from("/track/1").unwrap())).unwrap());
            m.insert("mpris:length".to_string(), OwnedValue::try_from(Value::from(215_000_000i64)).unwrap());
            m
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_session_bus_lists_the_player_and_forwards_transport() {
        use std::io::BufRead;
        let Ok(mut daemon) = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            eprintln!("dbus-daemon is not installed: skipped");
            return;
        };
        let mut address = String::new();
        std::io::BufReader::new(daemon.stdout.take().unwrap()).read_line(&mut address).unwrap();
        // Only this test reads the variable; the other tests use fakes.
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", address.trim());
        let calls = Arc::new(Mutex::new(vec![]));
        let _service = zbus::connection::Builder::session()
            .unwrap()
            .name("org.mpris.MediaPlayer2.secpartest.instance42")
            .unwrap()
            .serve_at("/org/mpris/MediaPlayer2", FakeRoot)
            .unwrap()
            .serve_at("/org/mpris/MediaPlayer2", FakePlayer(calls.clone()))
            .unwrap()
            .build()
            .await
            .unwrap();
        let mixer = Arc::new(FakeMixer::default());
        *mixer.volume.lock().unwrap() = Some((20, false));
        let media = Media::new(Box::new(DbusMpris::default()), mixer);
        let state = media.read().await;
        assert_eq!(
            state,
            MediaState { title: "Song".into(), artist: "A, B".into(), app: "Test Player".into(), playing: true, vol: 20, muted: false }
        );
        media.run(Cmd::PlayPause).await;
        media.run(Cmd::Next).await;
        media.run(Cmd::Prev).await;
        assert_eq!(*calls.lock().unwrap(), ["PlayPause", "Next", "Previous"]);
        let _ = daemon.kill();
        let _ = daemon.wait();
    }
}

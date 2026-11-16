//! The connecting role's engine path: pair through the authenticated
//! connection, open the tunnel to the computer's engine, and run the Moonlight
//! protocol inside this app. The picture goes to the app's own decoder and the
//! sound to its own output, in the same stream window as before; there is no
//! separate stream program. No PIN is typed: it only travels between this app
//! and the computer over the tunnel and the authenticated connection.
use crate::client_settings;
use secpar_client::{
    control::{self, Connected},
    gamestream,
    receiver::VideoFrame,
    session::{Command, Event},
};
use secpar_moonlight::{http, serde_like::Message, Output};
use secpar_stream::frame::{Codec, FrameHeader};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// How the stream is set up for this computer, from the client settings.
#[derive(Clone)]
pub struct Settings {
    data: PathBuf,
    /// None asks the computer for its own size.
    resolution: Option<(u32, u32)>,
    fps: u32,
    kbps: u32,
    hevc: bool,
    audio: bool,
}

fn data_folder() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join("Secpar"));
    }
    #[cfg(target_os = "linux")]
    {
        return std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".config")))
            .map(|path| path.join("secpar"));
    }
    #[allow(unreachable_code)]
    None
}

impl Settings {
    pub fn from_client(client: &client_settings::Settings) -> Option<Self> {
        Some(Self {
            data: data_folder()?,
            resolution: (client.width != 0 && client.height != 0).then_some((client.width, client.height)),
            fps: client.fps.clamp(10, 240),
            kbps: client.kbps.clamp(500, 200_000),
            hevc: !matches!(client.decoder, crate::video::Preference::Software),
            audio: client.audio,
        })
    }

    /// One folder per computer: this device's certificate towards that engine.
    fn profile(&self, host: &str) -> PathBuf {
        let name: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(64).collect();
        let name = if name.is_empty() { "computer".to_string() } else { name };
        self.data.join("stream").join(name)
    }
}

fn pin() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("{:04}", nanos % 10_000)
}

/// The engine connection is one per app; stopping it is safe when none runs.
pub fn stop_running() {
    secpar_moonlight::stop();
}

fn status(events: &mpsc::Sender<Event>, text: &str) {
    let _ = events.try_send(Event::Status(text.to_string()));
}

/// The window's input messages in Moonlight's form.
fn message(value: &Value) -> Option<Message> {
    let int = |k: &str| value[k].as_i64().unwrap_or(0) as i32;
    Some(match value["t"].as_str()? {
        "key" => Message::Key { vk: value["vk"].as_u64()? as u32, down: value["down"].as_bool()? },
        "mouse.pos" => Message::Position { x: value["x"].as_f64()? as f32, y: value["y"].as_f64()? as f32 },
        "mouse.move" => Message::Move { dx: int("dx"), dy: int("dy") },
        "mouse.button" => Message::Button {
            button: match value["b"].as_str()? {
                "left" => 1,
                "middle" => 2,
                "right" => 3,
                "x1" => 4,
                "x2" => 5,
                _ => return None,
            },
            down: value["down"].as_bool()?,
        },
        "mouse.scroll" => Message::Scroll { dx: int("dx"), dy: int("dy") },
        _ => return None,
    })
}

/// This device's tunnel identity for one computer, kept in the OS credential
/// store so the grant the host recorded still matches on the next stream. A
/// locked store only means the grant is re-recorded on each stream.
fn tunnel_identity(host: &str) -> Result<gamestream::TunnelIdentity, String> {
    use base64::Engine;
    let name: String = format!("stream-tunnel-{}", host.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect::<String>());
    if let Ok(Some(text)) = crate::vault::load(&name) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            let cert = value["cert"].as_str().and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok());
            let key = value["key"].as_str().and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok());
            if let (Some(cert), Some(key)) = (cert, key) {
                return Ok(gamestream::TunnelIdentity::from_der(cert, key));
            }
        }
    }
    let identity = gamestream::TunnelIdentity::generate().map_err(|_| "Could not create a tunnel identity")?;
    let text = json!({
        "cert": base64::engine::general_purpose::STANDARD.encode(identity.certificate_der()),
        "key": base64::engine::general_purpose::STANDARD.encode(identity.key_der()),
    })
    .to_string();
    let _ = crate::vault::save(&name, &text);
    Ok(identity)
}


/// Pairs this app's certificate with the computer's engine. The engine waits
/// for the PIN on its first request; the computer is told the same PIN through
/// the authenticated connection, repeatedly until the engine shows the pairing.
async fn pair(conn: &mut Connected, engine: &http::Engine, pin: &str, pairing: &control::GameStreamPairing) -> Result<(), String> {
    let pairing_run = engine.pair(pin);
    tokio::pin!(pairing_run);
    let deadline = tokio::time::sleep(Duration::from_secs(45));
    tokio::pin!(deadline);
    let mut approved = false;
    loop {
        tokio::select! {
            result = &mut pairing_run => return result.map_err(|_| "This computer did not accept this device's stream".to_string()),
            _ = &mut deadline => return Err("This computer did not accept this device's stream".into()),
            _ = tokio::time::sleep(Duration::from_millis(700)), if !approved => {
                approved = conn.gamestream_pair("gs-approve", pairing).await.is_ok();
            }
        }
    }
}

/// Runs the stream until it ends, the tunnel closes or the window stops it. The
/// authenticated connection stays open meanwhile for status and clipboard.
pub async fn run(
    mut conn: Connected,
    settings: Settings,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
) -> Result<(), String> {
    let _ = tokio::time::timeout(Duration::from_secs(2), events.send(Event::Paired(conn.computer.clone()))).await;
    if !conn.caps.is_empty() {
        let _ = events.try_send(Event::Caps(conn.caps.clone()));
    }
    status(&events, "Starting the stream");
    let info = conn.gamestream_info("gs-info").await?;
    let folder = settings.profile(&conn.computer.id);
    let identity = http::Identity::load_or_create(&folder)?;
    let certificate: String = {
        let body: String = identity.certificate_pem.lines().filter(|l| !l.starts_with("-----")).collect();
        use base64::Engine;
        let der = base64::engine::general_purpose::STANDARD.decode(body).map_err(|_| "certificate")?;
        ring::digest::digest(&ring::digest::SHA256, &der).as_ref().iter().map(|b| format!("{b:02x}")).collect()
    };
    let pin = pin();
    let prepared = gamestream::prepare(&info, conn.direct_host.as_deref(), tunnel_identity(&conn.computer.id)?).await?;
    let pairing = prepared.pairing(("0123456789ABCDEF", &certificate), &pin);
    let grant = conn.gamestream_pair("gs-grant", &pairing).await.err();
    let tunnel = match gamestream::connect(prepared).await {
        Ok(tunnel) => tunnel,
        Err(tunnel_error) => return Err(grant.unwrap_or(tunnel_error)),
    };
    let engine = http::Engine::new(gamestream::LOCAL_IP, gamestream::ENGINE_PORT, gamestream::ENGINE_PORT - 5, identity)?;
    let mut server = engine.server_info().await;
    if !server.as_ref().is_ok_and(|s| s.paired) {
        status(&events, "Pairing the stream");
        pair(&mut conn, &engine, &pin, &pairing).await?;
        server = engine.server_info().await;
    }
    let server = server?;
    let (width, height) = settings.resolution.unwrap_or((1920, 1080));
    let input_key: [u8; 16] = {
        let mut key = [0u8; 16];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key).map_err(|_| "random")?;
        key
    };
    let input_key_id = (Instant::now().elapsed().as_nanos() as i32 ^ std::process::id() as i32) & 0x7fff_ffff;
    let url = engine.launch(width, height, settings.fps, &input_key, input_key_id, server.current_game != 0).await?;

    // The engine's output becomes the same events the window's own stream used.
    let (ended_tx, mut ended) = mpsc::unbounded_channel::<Result<(), String>>();
    let frames = Arc::new(AtomicU32::new(0));
    let sink = {
        let events = events.clone();
        let audio = settings.audio;
        let (fps, ended_tx) = (settings.fps, ended_tx.clone());
        Arc::new(move |output: Output| match output {
            Output::Video { data, keyframe, hevc } => {
                let id = frames.fetch_add(1, Ordering::Relaxed);
                let header = FrameHeader { len: data.len() as u32, capture_us: 0, keyframe, codec: if hevc { Codec::Hevc } else { Codec::H264 } };
                let _ = events.blocking_send(Event::Video(VideoFrame { id, header, data }));
            }
            Output::Audio(packet) if audio => {
                let _ = events.try_send(Event::Audio { packet, received_at: Instant::now() });
            }
            Output::Audio(_) => {}
            Output::Started => {
                let _ = events.try_send(Event::StreamInfo { width, height, fps });
                let _ = events.try_send(Event::Status("Streaming".into()));
            }
            Output::Poor(poor) => {
                if poor {
                    let _ = events.try_send(Event::Notice("The connection is slow".into()));
                }
            }
            Output::Ended(code) => {
                let _ = ended_tx.send(if code == 0 { Ok(()) } else { Err(format!("The stream ended ({code})")) });
            }
        }) as Arc<dyn Fn(Output) + Send + Sync>
    };
    let start = secpar_moonlight::Start {
        address: gamestream::LOCAL_IP.into(),
        info: server,
        rtsp_url: url,
        width,
        height,
        fps: settings.fps,
        kbps: settings.kbps,
        remote: false,
        hevc: settings.hevc,
        input_key,
        input_key_id,
    };
    status(&events, "Connecting the stream");
    tokio::task::spawn_blocking(move || secpar_moonlight::connect(start, sink)).await.map_err(|_| "The stream could not start")??;

    let result = loop {
        tokio::select! {
            _ = tunnel.closed() => break Err("The stream tunnel closed".to_string()),
            done = ended.recv() => break done.unwrap_or(Ok(())),
            command = commands.recv() => match command {
                None | Some(Command::Stop) => break Ok(()),
                Some(Command::Input(value)) => {
                    if let Some(message) = message(&value) {
                        secpar_moonlight::input(&message);
                    }
                }
                Some(Command::Keyframe) => secpar_moonlight::keyframe(),
                Some(Command::Clipboard(text)) => {
                    if text.len() <= 1_000_000 {
                        let _ = control::send(&mut conn.socket, json!({"t":"clip","mime":"text/plain","data":text})).await;
                    }
                }
                Some(Command::ClipboardSharing(on)) => {
                    let _ = control::send(&mut conn.socket, json!({"t":"clip.sub","on":on})).await;
                }
                Some(_) => {}
            },
            message = control::next(&mut conn.socket) => match message {
                Ok(value) => match value["t"].as_str() {
                    Some("clip") if value["mime"] == "text/plain" => {
                        if let Some(text) = value["data"].as_str().filter(|text| text.len() <= 1_000_000) {
                            let _ = events.try_send(Event::Clipboard(text.into()));
                        }
                    }
                    Some("stream.error") | Some("error") => {
                        break Err(value["msg"].as_str().unwrap_or("The computer ended the stream").to_string());
                    }
                    _ => {}
                },
                Err(error) => break Err(error),
            },
        }
    };
    tokio::task::spawn_blocking(secpar_moonlight::stop).await.ok();
    result
}

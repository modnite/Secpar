//! The account's live channel. The service says "changed" whenever something this account sees
//! changes (its computers, friends, their computers, requests) and the account loop fetches its
//! lists again at once. Polling stays as the fallback, so a lost channel only costs speed.
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

const URL: &str = "wss://accounts.secpar.win/v1/events";

/// True while the channel is connected: the account loop then polls only every ten minutes.
pub static LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn live(on: bool) { LIVE.store(on, std::sync::atomic::Ordering::Relaxed); }
pub fn is_live() -> bool { LIVE.load(std::sync::atomic::Ordering::Relaxed) }

/// Runs until aborted: connects, reports each change, and reconnects with a short backoff.
pub async fn run(token: String, changed: mpsc::Sender<crate::account::Command>) {
    let mut wait = Duration::from_secs(1);
    loop {
        let started = std::time::Instant::now();
        let _ = session(&token, &changed).await;
        live(false);
        // A channel that stayed up a while starts over quickly; one that keeps failing backs off.
        if started.elapsed() > Duration::from_secs(60) { wait = Duration::from_secs(1); }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(30));
    }
}

async fn session(token: &str, changed: &mpsc::Sender<crate::account::Command>) -> Result<(), ()> {
    let mut request = URL.into_client_request().map_err(|_| ())?;
    request.headers_mut().insert("authorization", HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ())?);
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(request))
        .await.map_err(|_| ())?.map_err(|_| ())?;
    live(true);
    // Anything missed while the channel was down.
    let _ = changed.try_send(crate::account::Command::Changed);
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    let mut answered = true;
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) if text.as_str() == "changed" => { answered = true; let _ = changed.try_send(crate::account::Command::Changed); }
                Some(Ok(Message::Text(_))) => answered = true,
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Err(()),
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                // No answer to the last ping: the route is dead even if the socket says otherwise.
                if !answered { return Err(()); }
                answered = false;
                socket.send(Message::Text("ping".into())).await.map_err(|_| ())?;
            }
        }
    }
}

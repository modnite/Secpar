//! Card previews for friends who let you connect at any time. Each one is a view-only session that asks
//! the account for a grant, connects through the relay, subscribes to thumbnails and hands the pictures to
//! the window. The host ends it when a real connection comes, so a preview never gets in the way.
use crate::account::{Api, Event, FriendComputer};
use eframe::egui;
use secpar_client::control::{self, Connected, Incoming};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Pause before trying again after a refusal or a lost connection. It grows to the cap while nothing works.
const FIRST: Duration = Duration::from_secs(10);
const CAP: Duration = Duration::from_secs(300);
/// A computer that cannot give thumbnails, or whose owner now wants to be asked, is left alone this long.
const LONG: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct Tasks(HashMap<String, JoinHandle<()>>);

impl Tasks {
    /// Starts a preview for each computer that may be watched without asking, and stops the others.
    pub fn sync(&mut self, api: &Api, computers: &[FriendComputer], events: &mpsc::Sender<Event>, ctx: &egui::Context) {
        let wanted: Vec<&FriendComputer> = computers.iter().filter(|c| c.connect == "always").collect();
        self.0.retain(|id, task| {
            let keep = wanted.iter().any(|c| &c.id == id);
            if !keep {
                task.abort();
            }
            keep
        });
        for computer in wanted {
            if !self.0.contains_key(&computer.id) {
                let task = tokio::spawn(watch(api.clone(), computer.clone(), events.clone(), ctx.clone()));
                self.0.insert(computer.id.clone(), task);
            }
        }
    }
    /// Signed out or shutting down.
    pub fn stop(&mut self) {
        for (_, task) in self.0.drain() {
            task.abort();
        }
    }
}

async fn watch(api: Api, computer: FriendComputer, events: mpsc::Sender<Event>, ctx: egui::Context) {
    let mut wait = FIRST;
    loop {
        let started = Instant::now();
        let long = session(&api, &computer, &events, &ctx).await;
        let _ = events.send(Event::FriendPreview { id: computer.id.clone(), image: None }).await;
        ctx.request_repaint();
        wait = if long {
            LONG
        } else if started.elapsed() > Duration::from_secs(60) {
            FIRST
        } else {
            (wait * 2).min(CAP)
        };
        tokio::time::sleep(wait).await;
    }
}

/// One preview session. True when this computer should be left alone for a while.
async fn session(api: &Api, computer: &FriendComputer, events: &mpsc::Sender<Event>, ctx: &egui::Context) -> bool {
    let view = vec!["view".to_string()];
    let Ok((id, state, _, kept)) = api.request_access(&computer.id, &view).await else { return false };
    if state != "approved" {
        return true;
    }
    let Ok(grant) = api.claim(&id, computer, kept).await else { return false };
    let Ok(mut conn) = Connected::guest(&grant.relay, &grant.pc, &grant.grant, &grant.name).await else { return false };
    if !conn.caps.iter().any(|cap| cap == "thumb") {
        return true;
    }
    if control::send(&mut conn.socket, json!({"t": "thumb.sub", "on": true})).await.is_err() {
        return false;
    }
    loop {
        match control::next_incoming(&mut conn.socket).await {
            Ok(Incoming::Binary(frame)) => {
                if let Some(image) = control::thumb_jpeg(&frame).and_then(crate::previews::decode) {
                    if events.send(Event::FriendPreview { id: computer.id.clone(), image: Some(image) }).await.is_err() {
                        return true;
                    }
                    ctx.request_repaint();
                }
            }
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

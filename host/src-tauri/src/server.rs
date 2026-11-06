//! `--server`: everything phones talk to, running as SYSTEM in the console session (started by the
//! service). It serves the window over the pipe (crate::ipc) and starts the tray at sign-in.

use crate::agent::{AgentHandle, Event};
use crate::{files, ipc, mdns, net, remote, store::Store, tls, win};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

/// Shared state for the window's commands (crate::commands).
pub struct App {
    pub store: Arc<Mutex<Store>>,
    pub agent: AgentHandle,
    pub fp: String,
    pub identity: tls::Identity,
    pub account: Result<Arc<crate::host_account::Account>, String>,
    pub outgoing: files::Outgoing,
    pub net: Arc<Mutex<net::NetStatus>>,
}

pub fn run() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    if let Err(e) = rt.block_on(serve()) {
        crate::log::line(&format!("server: stopped: {e}"));
    }
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let dir = crate::data_dir();
    std::fs::create_dir_all(&dir)?;
    let identity = tls::Identity::load_or_create(&dir)?;
    let store = Arc::new(Mutex::new(Store::open(&dir)?));
    let host = Arc::new(crate::host::Host::new(store.clone()));
    let (outgoing, net) = (host.outgoing.clone(), host.net.clone());
    if store.lock().unwrap().config().anywhere {
        let net = net.clone();
        tokio::spawn(async move { *net.lock().unwrap() = net::apply_now(true).await });
    }
    let agent = crate::agent::Agent::start(&identity, store.clone(), host, "0.0.0.0:48999".parse()?, Duration::from_secs(30)).await?;
    if let Some(url) = remote::url() {
        tokio::spawn(remote::rendezvous::run(agent.clone(), store.clone(), url.into()));
    }
    let _mdns = mdns::Advert::start(store.lock().unwrap().pc_id(), &win::sys::pc_name());
    crate::log::line(&format!("server: up in session {}", win::session::current()));

    let ui = broadcast::channel::<Value>(32).0;
    let tx = ui.clone();
    crate::notify::set_hook(Box::new(move |title, body| {
        let _ = tx.send(json!({"ev": "toast", "title": title, "body": body}));
    }));
    let (mut events, tx) = (agent.events(), ui.clone());
    tokio::spawn(async move {
        while let Ok(e) = events.recv().await {
            let _ = tx.send(json!({"ev": if matches!(e, Event::Paired { .. }) { "paired" } else { "devices" }}));
        }
    });
    tokio::spawn(watch_lock(agent.clone(), ui.clone()));
    tokio::spawn(start_tray(store.clone()));

    // Account-link failure must never interrupt existing owner connections.
    let account = crate::host_account::open(&dir, &identity.fingerprint_hex);
    if let Ok(account) = &account { tokio::spawn(account.clone().run()); }
    let app = Arc::new(App { store, agent, fp: identity.fingerprint_hex.clone(), identity, account, outgoing, net });
    let weak = Arc::downgrade(&app);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(3));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let Some(app) = weak.upgrade() else { break };
            let Ok(account) = &app.account else { continue };
            let owner = account.state().await.owner;
            let revoked = app.store.lock().unwrap().set_account_owner(owner.clone());
            for device in revoked { app.agent.revoke(&device); }
            if let Some(owner) = owner {
                let _ = account.poll_pairings(|id| {
                    let app = app.clone(); let owner = owner.clone();
                    async move { crate::commands::owner_pair_url(&app,&id,&owner) }
                }).await;
            }
        }
    });
    let handler: ipc::Handler = Arc::new(move |cmd, args| Box::pin(crate::commands::dispatch(app.clone(), cmd, args)));
    ipc::serve(ipc::PIPE, handler, ui).await?;
    Ok(())
}

/// Tells phones (and the window) when the PC locks or unlocks.
async fn watch_lock(agent: AgentHandle, ui: broadcast::Sender<Value>) {
    let mut last = None;
    loop {
        let locked = tokio::task::spawn_blocking(win::desk::is_locked).await.unwrap_or(false);
        if last != Some(locked) {
            if last.is_some() {
                agent.broadcast(crate::proto::ServerMsg::Lock { locked });
                let _ = ui.send(json!({"ev": "devices"}));
            }
            last = Some(locked);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Starts the tray as the signed-in user once per sign-in (when "show in tray" is on).
async fn start_tray(store: Arc<Mutex<Store>>) {
    let mut started = false;
    loop {
        match win::session::user_token() {
            None => started = false,
            Some(t) if !started && store.lock().unwrap().config().autostart => {
                started = win::session::spawn(*t, "--hidden", true).is_some();
            }
            Some(_) => {}
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Copies the store and TLS identity from `old` (2.x) into `dir`; `key` is the decrypted TLS key.
/// Without the key the identity is left behind (a new one means re-pairing, but nothing breaks).
pub fn copy_data(old: &Path, dir: &Path, key: Option<&[u8]>) -> std::io::Result<bool> {
    if !old.join("store.json").exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    if let (Some(k), Ok(cert)) = (key, std::fs::read(old.join("tls.cer"))) {
        std::fs::write(dir.join("tls.key"), win::dpapi::protect(k).map_err(std::io::Error::other)?)?;
        std::fs::write(dir.join("tls.cer"), cert)?;
    }
    std::fs::copy(old.join("store.json"), dir.join("store.json"))?; // last: its presence marks "migrated"
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_keeps_identity_and_store() {
        let base = std::env::temp_dir().join(format!("pcr-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (old, new) = (base.join("old"), base.join("new"));
        let id = tls::Identity::load_or_create(&old).unwrap();
        Store::open(&old).unwrap();
        assert!(!copy_data(&base.join("none"), &new, None).unwrap());
        assert!(copy_data(&old, &new, Some(&id.key_der)).unwrap());
        let moved = tls::Identity::load_or_create(&new).unwrap();
        assert_eq!(moved.fingerprint_hex, id.fingerprint_hex, "phones pin this fingerprint");
        assert_eq!(Store::open(&new).unwrap().pc_id(), Store::open(&old).unwrap().pc_id());
    }
}

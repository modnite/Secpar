pub mod account_link;
pub mod agent;
pub mod clip;
pub mod commands;
pub mod engine;
pub mod files;
pub mod host;
pub mod host_account;
pub mod input;
pub mod ipc;
pub mod log;
pub mod mdns;
pub mod media;
pub mod net;
pub mod notify;
pub mod pad;
pub mod power;
pub mod proto;
pub mod prs;
pub mod remote;
pub mod server;
pub mod service;
pub mod store;
pub mod thumb;
pub mod tls;
pub mod unlock;
pub mod update;
pub mod win;

use std::path::PathBuf;


#[cfg(feature = "webui")]
mod webui;
#[cfg(feature = "webui")]
pub use webui::{run, run_with, show_window};

/// `C:\ProgramData\Secpar`: the server's store, TLS identity and log (SYSTEM + Administrators only).
pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".into())).join("Secpar")
}

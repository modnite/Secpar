//! A hosting computer for trying the window against, without a real engine and
//! with a throwaway identity. It enrols itself under a test account, reports
//! where it can be reached on this computer, and lets the account's devices in.
//! Pairing requests are accepted by a stand-in; nothing can be streamed.
//!
//! Usage: test_host <accounts file> <account number> <computer name>
//! Only works against a test account service on this computer.
use secpar_core::account::Service;
use secpar_hosting::{Engine, Pairing, Pending, Refusal};
use secpar_service::{Core, Settings};
use secpar_tunnel::Identity;
use std::sync::Arc;

struct NoEngine;
impl Engine for NoEngine {
    fn approve<'a>(&'a self, _: &'a str, _: &'a str, _: &'a Pairing, _: &'a str) -> Pending<'a, Result<(), Refusal>> {
        Box::pin(async { Ok(()) })
    }
    fn revoke<'a>(&'a self, _: &'a str, _: &'a str) -> Pending<'a, bool> {
        Box::pin(async { true })
    }
    fn granted(&self) -> Pending<'_, Vec<(String, String)>> {
        Box::pin(async { Vec::new() })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(args.next().ok_or("accounts file")?)?)?;
    let account = &file["accounts"][args.next().ok_or("account number")?.parse::<usize>()?];
    let name = args.next().ok_or("computer name")?;
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_string();
    let service = || Service::local_test(&text(&file["origin"]), &text(&file["address"]));

    let state = std::env::temp_dir().join(format!("secpar-test-host-{}", std::process::id()));
    let core = Core::start(Settings { account: service()?, state, bind: "127.0.0.1:0".parse()? }, Identity::generate()?, Some(Arc::new(NoEngine))).await?;
    let session = service()?.sign_in(&text(&account["email"]), &text(&account["password"])).await?;
    let challenge = service()?.challenge(&session).await?;
    let answer = core.answer(&serde_json::json!({"do": "enrolment", "challenge": challenge, "name": name}).to_string()).await;
    service()?.submit(&session, &serde_json::from_value(answer["proof"].clone())?).await?;
    println!("hosting as {name}");
    core.keep_current().await;
    Ok(())
}

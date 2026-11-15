//! The window's side against the real account service code running locally and a
//! real background part on the real local channel. The engine is a stand-in and
//! the session stays in memory.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test window -- --ignored`.
use secpar_core::account::Service;
use secpar_hosting::{Engine, Pairing, Pending, Refusal};
use secpar_service::{channel, Core, Settings};
use secpar_tunnel::Identity;
use serde::Deserialize;
use serde_json::{json, Value};
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

#[derive(Deserialize)]
struct Account {
    handle: String,
    email: String,
    password: String,
}
#[derive(Deserialize)]
struct Accounts {
    origin: String,
    address: String,
    accounts: Vec<Account>,
}
fn call(request: Value) -> Value {
    secpar_app::call(&request.to_string())
}

#[test]
#[ignore = "needs the local account service"]
fn signing_in_enrols_this_computer_and_signing_out_takes_it_off_the_account() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) { format!("secpar-test-window-{}", std::process::id()) } else { folder.path().join("b.sock").to_str().unwrap().to_string() };

    assert_eq!(call(json!({"do": "state"}))["error"], "not_configured");
    assert_eq!(call(json!({"do": "configure", "channel": name, "test_account": [origin, address], "memory_store": true}))["ok"], true);

    // Nobody is signed in and the background part is not running yet.
    let state = call(json!({"do": "state"}));
    assert_eq!((state["signed_in"].clone(), state["background"].clone()), (json!(false), json!(false)));
    assert_eq!(call(json!({"do": "devices"}))["error"], "signed_out");

    // A wrong password comes back with the service's own wording.
    let alice = &accounts[0];
    let wrong = call(json!({"do": "sign_in", "email": alice.email, "password": "not the password", "computer": "Desktop"}));
    assert_eq!(wrong["error"], "bad_credentials");
    assert!(wrong["message"].as_str().is_some_and(|m| !m.is_empty()));

    // Signing in without the background part works, but cannot enrol the computer.
    let alone = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "Desktop"}));
    assert_eq!((alone["signed_in"].clone(), alone["enrolled"].clone(), alone["enrol_error"].clone()), (json!(true), json!(false), json!("background_not_running")));
    assert_eq!(call(json!({"do": "devices"}))["devices"], json!([]));

    // The background part of this hosting computer starts.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let core = runtime.block_on(async {
        let settings = Settings { account: Service::local_test(&origin, &address).unwrap(), state: folder.path().join("background"), bind: "127.0.0.1:0".parse().unwrap() };
        Core::start(settings, Identity::generate().unwrap(), Some(Arc::new(NoEngine))).await.unwrap()
    });
    {
        let (core, name) = (core.clone(), name.clone());
        runtime.spawn(async move { channel::serve(core, &name).await });
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Signing in again enrols it, as one device, and it fetches its access list.
    for _ in 0..2 {
        let state = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "Desktop"}));
        assert_eq!(state["enrolled"], true, "{state}");
        assert_eq!((state["handle"].as_str(), state["hosts"].clone()), (Some(alice.handle.as_str()), json!(true)));
    }
    let devices = call(json!({"do": "devices"}));
    assert_eq!(devices["devices"].as_array().unwrap().len(), 1);
    assert_eq!(devices["devices"][0], json!({"id": devices["devices"][0]["id"], "name": "Desktop", "platform": std::env::consts::OS, "hosts": true, "online": true, "this": true}));
    let status = runtime.block_on(channel::ask(&name, &json!({"do": "status"}))).unwrap();
    assert!(status["access_expires_at"].as_u64().is_some());

    // Signing out takes this computer off the account, and it stops hosting for it.
    assert_eq!(call(json!({"do": "sign_out"})), json!({"ok": true, "device_removed": true}));
    assert_eq!(call(json!({"do": "state"}))["signed_in"], false);
    assert_eq!(call(json!({"do": "devices"}))["error"], "signed_out");
    let status = runtime.block_on(channel::ask(&name, &json!({"do": "status"}))).unwrap();
    assert_eq!(status["access_expires_at"], Value::Null);

    // The account really has no devices left.
    let again = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "Desktop again"}));
    assert_eq!(again["enrolled"], true);
    let devices = call(json!({"do": "devices"}));
    assert_eq!(devices["devices"].as_array().unwrap().len(), 1);
    assert_eq!(devices["devices"][0]["name"], "Desktop again");
    // Removing it from the list in the window has the same effect on hosting.
    assert_eq!(call(json!({"do": "remove_device", "id": devices["devices"][0]["id"]}))["ok"], true);
    let status = runtime.block_on(channel::ask(&name, &json!({"do": "status"}))).unwrap();
    assert_eq!(status["access_expires_at"], Value::Null);
}

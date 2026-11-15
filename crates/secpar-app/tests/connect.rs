//! Connect from the window's side: a hosting desktop that reports where it can
//! be reached, and this computer, a laptop the person has just signed in on.
//! Real account service code running locally, real background parts, channel
//! and tunnel; the engine is a stand-in.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --features fake-stream --test connect -- --ignored`.
//! The test-only `fake-stream` component writes the same settings identity as the
//! real Moonlight component and then stays alive until the window disconnects.
use secpar_core::account::{Platform, Proof, Service};
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
fn a_computer_just_signed_in_on_connects_to_the_accounts_hosting_computer() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let alice = &accounts[0];
    let folder = tempfile::tempdir().unwrap();
    let channel_name = |part: &str| {
        if cfg!(windows) {
            format!("secpar-test-connect-{part}-{}", std::process::id())
        } else {
            folder.path().join(format!("{part}.sock")).to_str().unwrap().to_string()
        }
    };
    let (desk, lap) = (channel_name("desktop"), channel_name("laptop"));
    let service = || Service::local_test(&origin, &address).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // The desktop was set up earlier: enrolled as a hosting computer, and its
    // background part keeps its list current and reports where it is.
    let (desktop_id, desktop_fingerprint, switched_off) = runtime.block_on(async {
        let settings = |part: &str| Settings { account: service(), state: folder.path().join(part), bind: "127.0.0.1:0".parse().unwrap() };
        let desktop = Core::start(settings("desktop"), Identity::generate().unwrap(), Some(Arc::new(NoEngine))).await.unwrap();
        let laptop = Core::<NoEngine>::start(settings("laptop"), Identity::generate().unwrap(), None).await.unwrap();
        for (core, name) in [(desktop.clone(), desk.clone()), (laptop, lap.clone())] {
            tokio::spawn(async move { channel::serve(core, &name).await });
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let account = service();
        let session = account.sign_in(&alice.email, &alice.password).await.unwrap();
        let challenge = account.challenge(&session).await.unwrap();
        let answer = channel::ask(&desk, &json!({"do": "enrolment", "challenge": challenge, "name": "Desktop"})).await.unwrap();
        let device = account.submit(&session, &serde_json::from_value::<Proof>(answer["proof"].clone()).unwrap()).await.unwrap();
        // A second hosting computer that is switched off: enrolled, never reported in.
        let off = account.enrol(&session, &Identity::generate().unwrap(), "Attic server", Platform::Linux, true).await.unwrap();
        tokio::spawn(async move { desktop.keep_current().await });
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        (device.id, device.fingerprint, off.id)
    });

    // On the laptop, the person opens the window and signs in for the first time.
    let component = option_env!("CARGO_BIN_EXE_fake-stream");
    assert!(component.is_some(), "run this with --features fake-stream");
    let data = folder.path().join("data");
    let mut configuration = json!({"do": "configure", "channel": lap, "test_account": [origin, address], "memory_store": true});
    if let Some(component) = component {
        configuration["stream_component"] = json!(component);
        configuration["data_folder"] = json!(data.to_string_lossy());
    }
    assert_eq!(call(configuration)["ok"], true);
    assert_eq!(call(json!({"do": "connect", "id": desktop_id}))["error"], "signed_out");
    let state = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "Laptop"}));
    assert_eq!((state["enrolled"].clone(), state["hosts"].clone()), (json!(true), json!(false)));

    // The list shows which hosting computers are on.
    let devices = call(json!({"do": "devices"}));
    let online = |id: &str| devices["devices"].as_array().unwrap().iter().find(|d| d["id"] == id).unwrap()["online"].clone();
    assert_eq!((online(&desktop_id), online(&switched_off)), (json!(true), json!(false)));

    // Connect works at once, although the desktop's list was fetched before the
    // laptop existed: being refused makes the desktop fetch a new one.
    let connected = call(json!({"do": "connect", "id": desktop_id}));
    assert_eq!(connected["ok"], true, "{connected}");
    assert_eq!(connected["streaming"], true, "{connected}");
    assert_eq!(call(json!({"do": "state"}))["connected_to_host"], true);
    assert_eq!(call(json!({"do": "state"}))["streaming"], true);
    // The stream component was started in its own profile for that computer.
    let profile = data.join("stream").join(&desktop_fingerprint);
    for _ in 0..50 {
        if profile.join("stream-running").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(profile.join("stream-running").exists(), "the stream component did not start");
    assert!(profile.join("paired").exists(), "the profile was not recorded as paired");
    let status = runtime.block_on(channel::ask(&desk, &json!({"do": "status"}))).unwrap();
    assert_eq!(status["connected_devices"].as_array().unwrap().len(), 1);

    assert_eq!(call(json!({"do": "disconnect"}))["ok"], true);
    assert_eq!(call(json!({"do": "state"}))["connected_to_host"], false);
    assert_eq!(call(json!({"do": "state"}))["streaming"], false);
    // A computer that is off, and one that is not on the account.
    assert_eq!(call(json!({"do": "connect", "id": switched_off}))["error"], "offline");
    assert_eq!(call(json!({"do": "connect", "id": "00000000-0000-4000-8000-000000000000"}))["error"], "device_not_found");
}

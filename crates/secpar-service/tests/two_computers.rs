//! Two background parts on one computer, the real account service code running
//! locally, real tunnels and the real local channel. The engine is a stand-in.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test two_computers -- --ignored`.
use secpar_core::account::{Proof, Service, Session};
use secpar_hosting::{Engine, Pairing, Pending, Refusal};
use secpar_service::{channel, Core, Settings};
use secpar_tunnel::Identity;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct FakeEngine {
    grants: Mutex<Vec<(String, String)>>,
}
impl Engine for FakeEngine {
    fn approve<'a>(&'a self, owner: &'a str, device: &'a str, _: &'a Pairing, _: &'a str) -> Pending<'a, Result<(), Refusal>> {
        Box::pin(async move {
            self.grants.lock().unwrap().push((owner.into(), device.into()));
            Ok(())
        })
    }
    fn revoke<'a>(&'a self, owner: &'a str, device: &'a str) -> Pending<'a, bool> {
        Box::pin(async move {
            self.grants.lock().unwrap().retain(|g| !(g.0 == owner && g.1 == device));
            true
        })
    }
    fn granted(&self) -> Pending<'_, Vec<(String, String)>> {
        Box::pin(async move { self.grants.lock().unwrap().clone() })
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

/// What the window does to enrol the computer its background part runs on.
async fn enrol(account: &Service, session: &Session, name: &str, label: &str) -> secpar_core::account::Device {
    let challenge = account.challenge(session).await.unwrap();
    let answer = channel::ask(name, &json!({"do": "enrolment", "challenge": challenge, "name": label})).await.unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    let proof: Proof = serde_json::from_value(answer["proof"].clone()).unwrap();
    account.submit(session, &proof).await.unwrap()
}
async fn ask(name: &str, request: Value) -> Value {
    channel::ask(name, &request).await.expect("the background part answers")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the local account service"]
async fn a_desktop_hosts_and_a_laptop_of_the_same_account_connects() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let service = || Service::local_test(&origin, &address).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let unique = std::process::id();
    let channel_name = |part: &str| {
        if cfg!(windows) {
            format!("secpar-test-{part}-{unique}")
        } else {
            folder.path().join(format!("{part}.sock")).to_str().unwrap().to_string()
        }
    };
    let settings = |part: &str| Settings { account: service(), state: folder.path().join(part), bind: "127.0.0.1:0".parse().unwrap() };

    // The desktop hosts; the laptop only connects.
    let engine = Arc::new(FakeEngine::default());
    let desktop = Core::start(settings("desktop"), Identity::generate().unwrap(), Some(engine.clone())).await.unwrap();
    let laptop = Core::<FakeEngine>::start(settings("laptop"), Identity::generate().unwrap(), None).await.unwrap();
    let stranger = Core::<FakeEngine>::start(settings("stranger"), Identity::generate().unwrap(), None).await.unwrap();
    let (desk, lap, other) = (channel_name("desktop"), channel_name("laptop"), channel_name("stranger"));
    for (core, name) in [(desktop.clone(), desk.clone()), (laptop, lap.clone()), (stranger, other.clone())] {
        tokio::spawn(async move { channel::serve(core, &name).await });
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let status = ask(&desk, json!({"do": "status"})).await;
    assert_eq!((status["hosts"].clone(), status["allowed"].clone()), (json!(true), json!(0)));
    assert_eq!(ask(&lap, json!({"do": "status"})).await["hosts"], false);
    assert_eq!(ask(&lap, json!({"do": "refresh"})).await["error"], "not_hosting");
    assert_eq!(ask(&desk, json!({"do": "format_disk"})).await["error"], "unknown_request");

    // The person signs in in the window on each computer. Only the window has the session.
    let account = service();
    let session = account.sign_in(&accounts[0].email, &accounts[0].password).await.unwrap();
    let desktop_device = enrol(&account, &session, &desk, "Desktop").await;
    let laptop_device = enrol(&account, &session, &lap, "Laptop").await;
    assert!(desktop_device.hosts && !laptop_device.hosts);
    assert_eq!(desktop_device.fingerprint, status["device"]);

    // Before the desktop has its list, the laptop is not let in.
    let host = desktop.tunnel_address().unwrap();
    let connect = json!({"do": "connect", "host": desktop_device.fingerprint, "addresses": [host]});
    assert_eq!(ask(&lap, connect.clone()).await["error"], "not_connected");

    // The desktop fetches its list with its own key; now the laptop gets through and pairs.
    assert_eq!(ask(&desk, json!({"do": "refresh"})).await["result"], "updated");
    assert_eq!(ask(&desk, json!({"do": "status"})).await["allowed"], 1);
    let connected = ask(&lap, connect.clone()).await;
    assert_eq!(connected["ok"], true, "{connected}");
    assert_eq!(ask(&lap, json!({"do": "status"})).await["connected_to_host"], true);
    assert_eq!(ask(&desk, json!({"do": "status"})).await["connected_devices"], json!([laptop_device.fingerprint]));
    let pairing = json!({"client_id": "0123456789ABCDEF", "certificate_sha256": "ab".repeat(32), "pin": "1234"});
    assert_eq!(ask(&lap, json!({"do": "pair", "pairing": pairing})).await["answer"], "paired");
    assert_eq!(*engine.grants.lock().unwrap(), vec![(session.user.id.clone(), laptop_device.fingerprint.clone())]);

    // A computer of another account is enrolled, and is still refused by this desktop.
    let bob = account.sign_in(&accounts[1].email, &accounts[1].password).await.unwrap();
    enrol(&account, &bob, &other, "Other PC").await;
    assert_eq!(ask(&desk, json!({"do": "refresh"})).await["result"], "updated");
    assert_eq!(ask(&other, connect.clone()).await["error"], "not_connected");

    // The laptop is removed from the account in the window; the next refresh ends it.
    account.remove(&session, &laptop_device.id).await.unwrap();
    assert_eq!(ask(&desk, json!({"do": "refresh"})).await["result"], "updated");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(ask(&lap, json!({"do": "status"})).await["connected_to_host"], false);
    assert_eq!(ask(&lap, connect).await["error"], "not_connected");
    assert!(engine.grants.lock().unwrap().is_empty());

    // The desktop itself is removed: it keeps no list and hosts nobody.
    account.remove(&session, &desktop_device.id).await.unwrap();
    assert_eq!(ask(&desk, json!({"do": "refresh"})).await["result"], "removed");
    assert_eq!(ask(&desk, json!({"do": "status"})).await["access_expires_at"], Value::Null);
}

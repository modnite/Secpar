//! Removing stale device records through the window's request, with a real
//! background part and the real account service code running locally. The other
//! records are enrolled with throwaway identities, the way old versions left a
//! second record for one computer.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test duplicates -- --ignored`.
use secpar_core::account::{Platform, Service};
use secpar_hosting::{Engine, Pairing, Pending, Refusal};
use secpar_service::{channel, Core, Settings};
use secpar_tunnel::Identity;
use serde::Deserialize;
use serde_json::{json, Value};

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
fn extra_records_with_one_name_are_removed_except_this_computer() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let alice = &accounts[0];
    let folder = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) { format!("secpar-test-duplicates-{}", std::process::id()) } else { folder.path().join("b.sock").to_str().unwrap().to_string() };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let core = runtime.block_on(async {
        let settings = Settings { account: Service::local_test(&origin, &address).unwrap(), state: folder.path().join("background"), bind: "127.0.0.1:0".parse().unwrap() };
        Core::<NoEngine>::start(settings, Identity::generate().unwrap(), None).await.unwrap()
    });
    {
        let (core, name) = (core.clone(), name.clone());
        runtime.spawn(async move { channel::serve(core, &name).await });
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    let configuration = json!({"do": "configure", "channel": name, "test_account": [origin, address], "memory_store": true});
    assert_eq!(call(configuration)["ok"], true);
    let state = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "This computer"}));
    assert_eq!(state["enrolled"], true, "{state}");

    // Old records: two computers that each left one extra row, and one extra row
    // with this computer's own name.
    runtime.block_on(async {
        let service = Service::local_test(&origin, &address).unwrap();
        let session = service.sign_in(&alice.email, &alice.password).await.unwrap();
        for name in ["x250", "x250 ", "This computer", "Attic"] {
            service.enrol(&session, &Identity::generate().unwrap(), name, Platform::Linux, false).await.unwrap();
        }
    });
    let before = call(json!({"do": "devices"}));
    let all_names: Vec<String> = before["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_owned())
        .collect();
    let mut distinct = all_names.clone();
    distinct.sort();
    distinct.dedup();
    // Each name with more than one record loses all but the record kept below.
    let expected = all_names.len() - distinct.len();
    assert_eq!(expected, 2, "the test's own stale records are missing: {before}");

    let removed = call(json!({"do": "remove_duplicate_devices"}));
    assert_eq!(removed, json!({"ok": true, "removed": expected}), "{removed}");

    let after = call(json!({"do": "devices"}));
    let devices = after["devices"].as_array().unwrap();
    assert_eq!(devices.len(), distinct.len(), "{after}");
    let mut after_names: Vec<String> = devices.iter().map(|d| d["name"].as_str().unwrap().to_owned()).collect();
    after_names.sort();
    assert_eq!(after_names, distinct, "a name still has two records: {after}");
    // The record kept under this computer's name is really this computer.
    let mine = devices.iter().find(|d| d["name"] == "This computer").unwrap();
    assert_eq!(mine["this"], true, "{after}");
    assert!(devices.iter().any(|d| d["name"] == "x250"));
}

//! Friend permissions as the window sends them: one JSON request into the app
//! library and back, against the real account service code running locally.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test friend_permissions -- --ignored`.
use secpar_core::account::Service;
use serde::Deserialize;
use serde_json::{json, Value};

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
fn the_window_saves_a_friend_s_permissions_and_lists_the_result() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let (alice, bob) = (&accounts[0], &accounts[1]);
    let channel = if cfg!(windows) { format!("secpar-test-permissions-{}", std::process::id()) } else { format!("/tmp/secpar-test-permissions-{}.sock", std::process::id()) };

    let configured = call(json!({"do": "configure", "channel": channel, "test_account": [&origin, &address], "memory_store": true}));
    assert_eq!(configured["ok"], true, "{configured}");

    // The Add friend row sends a request by account name.
    let state = call(json!({"do": "sign_in", "email": alice.email, "password": alice.password, "computer": "Desktop"}));
    assert_eq!(state["signed_in"], true, "{state}");
    assert_eq!(call(json!({"do": "request_friend", "handle": "@bob"}))["ok"], true);
    let sent = call(json!({"do": "friends"}))["friends"].clone();
    assert_eq!((sent[0]["state"].as_str(), sent[0]["incoming"].clone()), (Some("pending"), json!(false)));

    // Bob accepts. The friend appears with the service's default permissions.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let other = Service::local_test(&origin, &address).unwrap();
    runtime.block_on(async {
        let b = other.sign_in(&bob.email, &bob.password).await.unwrap();
        let incoming = other.friends(&b).await.unwrap();
        other.accept_friend(&b, &incoming[0].id).await.unwrap();
    });
    let listed = call(json!({"do": "friends"}))["friends"].clone();
    assert_eq!((listed[0]["state"].as_str(), listed[0]["connect"].as_str(), listed[0]["input"].as_str()), (Some("accepted"), Some("ask"), Some("view")));
    assert_eq!((listed[0]["keyboard"].clone(), listed[0]["mouse"].clone()), (json!(false), json!(false)));
    let id = listed[0]["id"].as_str().unwrap().to_string();

    // Allow to connect: Always. The answer is the settings now in force.
    let saved = call(json!({"do": "save_friend_settings", "id": id, "connect": "always"}));
    assert_eq!(saved["ok"], true, "{saved}");
    assert_eq!(saved["settings"]["connect"], "always");
    // Control: Keyboard and mouse. The service derives "full" from the two flags.
    let saved = call(json!({"do": "save_friend_settings", "id": id, "keyboard": true, "mouse": true}));
    assert_eq!((saved["settings"]["input"].as_str(), saved["settings"]["keyboard"].clone(), saved["settings"]["mouse"].clone()), (Some("full"), json!(true), json!(true)));
    // Both changes are on the list the page draws.
    let listed = call(json!({"do": "friends"}))["friends"].clone();
    assert_eq!((listed[0]["connect"].as_str(), listed[0]["trusted"].clone()), (Some("always"), json!(true)));
    assert_eq!((listed[0]["keyboard"].clone(), listed[0]["mouse"].clone()), (json!(true), json!(true)));
    // And on the list this account reads, which its own computers follow.
    let mine = runtime.block_on(async {
        let a = other.sign_in(&alice.email, &alice.password).await.unwrap();
        other.friends(&a).await.unwrap()
    });
    assert_eq!((mine[0].connect.as_str(), mine[0].input.as_str(), mine[0].keyboard, mine[0].mouse), ("always", "full", true, true));
    // The friend's own settings are their grant to this account, not this one's,
    // so that side still shows the defaults.
    let theirs = runtime.block_on(async {
        let b = other.sign_in(&bob.email, &bob.password).await.unwrap();
        other.friends(&b).await.unwrap()
    });
    assert_eq!((theirs[0].connect.as_str(), theirs[0].input.as_str(), theirs[0].keyboard, theirs[0].mouse), ("ask", "view", false, false));

    // Control: View only takes both permissions back.
    let saved = call(json!({"do": "save_friend_settings", "id": id, "input": "view"}));
    assert_eq!((saved["settings"]["input"].as_str(), saved["settings"]["keyboard"].clone(), saved["settings"]["mouse"].clone()), (Some("view"), json!(false), json!(false)));
    // A friend that is not on the list is refused rather than silently saved.
    let refused = call(json!({"do": "save_friend_settings", "id": "00000000-0000-0000-0000-000000000000", "connect": "always"}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"], "account_unavailable");
}

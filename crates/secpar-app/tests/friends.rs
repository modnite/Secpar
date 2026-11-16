//! The friends routes as the window uses them, against the real account service
//! code running locally.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test friends -- --ignored`.
use secpar_core::account::{FriendChange, Service};
use serde::Deserialize;

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

#[test]
#[ignore = "needs the local account service"]
fn a_request_is_listed_then_accepted_and_removed() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let (alice, bob) = (&accounts[0], &accounts[1]);
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let service = Service::local_test(&origin, &address).unwrap();
        let a = service.sign_in(&alice.email, &alice.password).await.unwrap();
        let b = service.sign_in(&bob.email, &bob.password).await.unwrap();
        assert!(service.friends(&a).await.unwrap().is_empty());

        // Alice asks Bob. The request names her as the one who asked.
        service.request_friend(&a, "@bob").await.unwrap();
        let sent = service.friends(&a).await.unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].handle.as_str(), sent[0].state.as_str(), sent[0].requester.as_str()), ("bob", "pending", a.user.id.as_str()));
        let incoming = service.friends(&b).await.unwrap();
        assert_eq!((incoming[0].handle.as_str(), incoming[0].requester.as_str()), ("alice", a.user.id.as_str()));

        // Bob accepts. The default permissions are view only and asks each time.
        service.accept_friend(&b, &a.user.id).await.unwrap();
        let friends = service.friends(&a).await.unwrap();
        assert_eq!(friends.len(), 1);
        assert_eq!((friends[0].state.as_str(), friends[0].connect.as_str(), friends[0].input.as_str()), ("accepted", "ask", "view"));
        assert!(!friends[0].keyboard && !friends[0].mouse);

        // Bob lets this friend connect without asking, and gives keyboard and mouse.
        let saved = service.save_friend_settings(&b, &a.user.id, &FriendChange { connect: Some("always".into()), ..Default::default() }).await.unwrap();
        assert_eq!((saved.connect.as_str(), saved.input.as_str(), saved.keyboard, saved.mouse), ("always", "view", false, false));
        let full = service.save_friend_settings(&b, &a.user.id, &FriendChange { keyboard: Some(true), mouse: Some(true), ..Default::default() }).await.unwrap();
        assert_eq!((full.input.as_str(), full.keyboard, full.mouse), ("full", true, true));
        let listed = service.friends(&b).await.unwrap();
        assert_eq!((listed[0].connect.as_str(), listed[0].input.as_str(), listed[0].keyboard, listed[0].mouse), ("always", "full", true, true));
        // View only takes both fine permissions back.
        let view = service.save_friend_settings(&b, &a.user.id, &FriendChange { input: Some("view".into()), ..Default::default() }).await.unwrap();
        assert_eq!((view.input.as_str(), view.keyboard, view.mouse), ("view", false, false));
        assert_eq!(service.friend_settings(&b, &a.user.id).await.unwrap(), view);
        // A value the service does not accept is refused before it is sent.
        assert!(service.save_friend_settings(&b, &a.user.id, &FriendChange { connect: Some("often".into()), ..Default::default() }).await.is_err());
        assert!(service.save_friend_settings(&b, &a.user.id, &FriendChange::default()).await.is_err());

        // Removing the friend ends it for both sides.
        service.remove_friend(&a, &b.user.id).await.unwrap();
        assert!(service.friends(&a).await.unwrap().is_empty());
        assert!(service.friends(&b).await.unwrap().is_empty());

        // An unknown name is refused, not quietly queued.
        assert!(service.request_friend(&a, "no_such_account").await.is_err());
    });
}

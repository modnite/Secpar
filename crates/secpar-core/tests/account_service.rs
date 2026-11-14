//! The account client against a real account service running on this computer.
//!
//! Start `node cloud/accounts/test/local-server.mjs <port> <file>` and run with
//! `SECPAR_TEST_ACCOUNTS=<file> cargo test --test account_service -- --ignored`.
use secpar_core::{
    account::{verify_access, Error, Platform, Service},
    hosting::{Access, Refreshed},
};
use secpar_tunnel::Identity;
use serde::Deserialize;

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

#[tokio::test]
#[ignore = "needs the local account service"]
async fn sign_in_enrol_and_fetch_the_access_list() {
    let file = std::env::var("SECPAR_TEST_ACCOUNTS").expect("SECPAR_TEST_ACCOUNTS");
    let Accounts { origin, address, accounts } = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    let (alice, bob) = (&accounts[0], &accounts[1]);
    let service = Service::local_test(&origin, &address).unwrap();

    // A wrong password is refused with the service's own wording.
    match service.sign_in(&alice.email, "not the password").await {
        Err(Error::Refused { status: 401, code, message: Some(_) }) => assert_eq!(code, "bad_credentials"),
        other => panic!("unexpected: {other:?}"),
    }
    let session = service.sign_in(&alice.email, &alice.password).await.unwrap();
    assert_eq!(session.user.handle, alice.handle);

    // This computer hosts. Enrolling twice keeps one device and takes the new name.
    let desktop = Identity::generate().unwrap();
    let first = service.enrol(&session, &desktop, "Desktop", Platform::Windows, true).await.unwrap();
    let again = service.enrol(&session, &desktop, "Desk", Platform::Windows, true).await.unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(first.fingerprint, secpar_core::identity::hex(&desktop.fingerprint()));
    let laptop = Identity::generate().unwrap();
    let second = service.enrol(&session, &laptop, "Laptop", Platform::Linux, false).await.unwrap();
    let listed = service.devices(&session).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed.iter().find(|d| d.id == first.id).unwrap().name, "Desk");

    // Another account cannot take the same device, and Android never hosts.
    let other = service.sign_in(&bob.email, &bob.password).await.unwrap();
    match service.enrol(&other, &desktop, "Stolen", Platform::Windows, false).await {
        Err(Error::Refused { status: 409, code, .. }) => assert_eq!(code, "device_belongs_to_another_account"),
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(
        service.enrol(&other, &Identity::generate().unwrap(), "Phone", Platform::Android, true).await,
        Err(Error::Refused { status: 400, .. })
    ));

    // The signed list for the hosting device names the laptop, and checks again offline.
    let keys = service.keys().await.unwrap();
    let (signed, list) = service.access(&session, &first, &desktop.fingerprint(), &keys).await.unwrap();
    assert_eq!(list.owner, session.user.id);
    assert_eq!(list.devices.len(), 1);
    assert_eq!(list.devices[0].fingerprint, laptop.fingerprint());
    assert!(list.guests.is_empty());
    assert_eq!(verify_access(&signed, &keys, &origin, &desktop.fingerprint(), list.expires_at - 1).unwrap(), list);
    assert!(verify_access(&signed, &keys, &origin, &laptop.fingerprint(), list.expires_at - 1).is_err());
    // A device that does not host has no list.
    assert!(matches!(
        service.access(&session, &second, &laptop.fingerprint(), &keys).await,
        Err(Error::Refused { status: 404, .. })
    ));

    // The hosting device gets the same list with only its own key, as it must
    // before anyone has logged in. A device that does not host gets nothing.
    let (_, own) = service.device_access(&desktop, &keys).await.unwrap();
    assert_eq!(own.devices, list.devices);
    assert!(matches!(service.device_access(&laptop, &keys).await, Err(Error::Refused { status: 404, .. })));

    // The hosting side keeps the list, and keeps using it while the service is away.
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("access.json");
    let access = Access::open(&origin, desktop.fingerprint(), file.clone(), at);
    assert!(access.allowed(at).is_empty());
    assert_eq!(access.refresh(&service, &desktop).await, Refreshed::Updated);
    assert_eq!(access.allowed(at), vec![laptop.fingerprint()]);
    let nowhere = Service::local_test(&origin, "http://127.0.0.1:9").unwrap();
    assert_eq!(nowhere.keys().await, Err(Error::Unreachable));
    assert_eq!(access.refresh(&nowhere, &desktop).await, Refreshed::Kept(Error::Unreachable));
    assert_eq!(access.allowed(at), vec![laptop.fingerprint()]);
    assert_eq!(Access::open(&origin, desktop.fingerprint(), file.clone(), at).allowed(at), vec![laptop.fingerprint()]);

    // Removing the laptop from the account drops it at the next refresh.
    service.remove(&session, &second.id).await.unwrap();
    assert_eq!(access.refresh(&service, &desktop).await, Refreshed::Updated);
    assert!(access.allowed(at).is_empty());

    // Removing the hosting device itself leaves it with no list at all.
    service.remove(&session, &first.id).await.unwrap();
    assert_eq!(access.refresh(&service, &desktop).await, Refreshed::Removed);
    assert!(!file.exists());
}

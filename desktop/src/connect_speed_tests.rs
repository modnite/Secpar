//! What keeps a connection from waiting: the choices that decide whether the account is asked at
//! all, jobs that run beside commands, and connections that change hands. Time is paused, so
//! "at once" means zero seconds, not "fast enough on this machine".
use super::*;
use crate::previews::Handoffs;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout, Instant as Clock};

const OWNER: &str = "00000000-0000-4000-8000-0000000000a1";
const ID: &str = "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER: &str = "11111111-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

fn fp_hex(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}
fn computer(id: &str, fp: Option<u8>, linked: u8, online: u8) -> OwnedComputer {
    OwnedComputer { id: id.into(), name: format!("PC {}", &id[9..13]), platform: "linux".into(), certificate_fp: fp.map(fp_hex), linked, online }
}
fn credential(fp: u8) -> Computer {
    Computer { id: "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(), name: "PC".into(), hosts: vec!["127.0.0.1".into()], port: 1, fingerprint: [fp; 32], token: "dG9rZW4".into(), relay: None }
}
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn a_saved_credential_is_used_unless_the_account_has_ruled_it_out() {
    let saved = [0xab; 32];
    assert_eq!(plan_open(&[], ID, Some(&saved)), Plan::UseSaved, "before the first sync nothing contradicts it");
    assert_eq!(plan_open(&[computer(ID, Some(0xab), 1, 1)], ID, Some(&saved)), Plan::UseSaved);
    assert_eq!(plan_open(&[computer(OTHER, Some(0xcd), 1, 1)], ID, Some(&saved)), Plan::UseSaved, "another computer says nothing about this one");
    assert_eq!(plan_open(&[computer(ID, Some(0xcd), 1, 1)], ID, Some(&saved)), Plan::AskAccount, "the computer has another identity now");
    assert_eq!(plan_open(&[computer(ID, Some(0xab), 0, 1)], ID, Some(&saved)), Plan::AskAccount, "no longer linked");
    assert_eq!(plan_open(&[computer(ID, Some(0xab), 1, 1)], ID, None), Plan::AskAccount, "nothing saved");
}

/// An account that accepts a connection and never answers: any request to it would hang.
async fn silent_account() -> Api {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = vec![];
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 64];
            let _ = socket.read(&mut buf).await;
            held.push(socket);
        }
    });
    Api::for_test(base)
}

#[tokio::test(start_paused = true)]
async fn opening_with_a_saved_credential_does_not_wait_for_the_account() {
    let api = silent_account().await;
    for known in [vec![], vec![computer(ID, Some(0xab), 1, 1)]] {
        let (events, mut out) = mpsc::channel(4);
        let mut pending = None;
        let started = Clock::now();
        let done = timeout(ms(1), connect_owned_for_test(&api, OWNER, ID, &known, Some(credential(0xab)), &mut pending, &events)).await;
        assert!(done.expect("answered without a round trip").is_ok());
        assert_eq!(started.elapsed(), Duration::ZERO, "no virtual time passed");
        assert!(matches!(out.recv().await, Some(Event::OwnedConnection(OwnedConnection::Cached { .. }))));
    }
}

#[tokio::test(start_paused = true)]
async fn a_credential_the_account_contradicts_does_go_to_the_account() {
    let api = silent_account().await;
    let (events, _out) = mpsc::channel(4);
    let mut pending = None;
    let known = vec![computer(ID, Some(0xcd), 1, 1)];
    let done = timeout(Duration::from_secs(3), connect_owned_for_test(&api, OWNER, ID, &known, Some(credential(0xab)), &mut pending, &events)).await;
    assert!(done.is_err(), "it waits for the account, which never answers");
}

fn idle_sync() -> Background<SyncOut> {
    Some(Box::pin(async {
        sleep(Duration::from_secs(20)).await;
        SyncOut { host: Err("done".into()), computers: Ok(vec![]) }
    }))
}

#[tokio::test(start_paused = true)]
async fn a_command_is_served_while_a_sync_is_still_running() {
    let (tx, mut rx) = mpsc::channel(4);
    tx.send(Command::Refresh).await.unwrap();
    let (mut sync, mut auto): (Background<SyncOut>, Background<AutoOut>) = (idle_sync(), None);
    let started = Clock::now();
    let wake = next_wake(&mut rx, Duration::from_secs(1), &mut sync, &mut auto).await;
    assert!(matches!(wake, Some(Wake::Command(Command::Refresh))));
    assert_eq!(started.elapsed(), Duration::ZERO, "the 20 s sync did not hold the command back");
    assert!(sync.is_some(), "and it carries on");
    // Without a command the clock ticks, and the sync finishes at its own time.
    let wake = next_wake(&mut rx, Duration::from_secs(1), &mut sync, &mut auto).await;
    assert!(matches!(wake, Some(Wake::Tick)));
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    let wake = next_wake(&mut rx, Duration::from_secs(60), &mut sync, &mut auto).await;
    assert!(matches!(wake, Some(Wake::Sync(_))));
    assert_eq!(started.elapsed(), Duration::from_secs(20));
    drop(tx);
    assert!(next_wake(&mut rx, Duration::from_secs(1), &mut None, &mut None).await.is_none(), "closing ends the loop");
}

#[tokio::test(start_paused = true)]
async fn the_host_check_and_the_computer_list_overlap() {
    let started = Clock::now();
    let out = gather(
        async {
            sleep(Duration::from_secs(2)).await;
            Err("slow local service".to_string())
        },
        async {
            sleep(ms(300)).await;
            Ok(vec![computer(ID, Some(0xab), 1, 1)])
        },
    )
    .await;
    assert_eq!(started.elapsed(), Duration::from_secs(2), "the longer of the two, not their sum");
    assert_eq!(out.computers.unwrap().len(), 1);
}

#[test]
fn a_pairing_request_is_asked_about_quickly_at_first() {
    assert_eq!(pair_poll_delay(Duration::ZERO), ms(300));
    assert_eq!(pair_poll_delay(Duration::from_secs(5)), ms(300));
    assert_eq!(pair_poll_delay(Duration::from_secs(10)), ms(800));
    assert_eq!(pair_poll_delay(Duration::from_secs(60)), Duration::from_secs(2));
}

#[test]
fn computers_without_a_credential_are_paired_one_at_a_time_with_a_backoff() {
    let now = Instant::now();
    let a = "11111111-aaaa-4aaa-8aaa-000000000001";
    let b = "11111111-aaaa-4aaa-8aaa-000000000002";
    let list = vec![
        computer(a, Some(1), 1, 0),                                  // offline
        computer("11111111-aaaa-4aaa-8aaa-000000000003", Some(3), 0, 1), // not linked
        computer("11111111-aaaa-4aaa-8aaa-000000000004", None, 1, 1),    // no identity on record
        computer("11111111-aaaa-4aaa-8aaa-000000000005", Some(5), 1, 1), // this computer
        computer(b, Some(2), 1, 1),
    ];
    let local = Some("11111111-aaaa-4aaa-8aaa-000000000005");
    let none = |_: &str| None;
    let mut tried = HashMap::new();
    assert_eq!(plan_auto_pair(&list, local, &none, &tried, now).map(|c| c.id.as_str()), Some(b));
    let has = |id: &str| (id == b).then_some([2u8; 32]);
    assert!(plan_auto_pair(&list, local, &has, &tried, now).is_none(), "a matching credential is enough");
    let stale = |id: &str| (id == b).then_some([9u8; 32]);
    assert_eq!(plan_auto_pair(&list, local, &stale, &tried, now).map(|c| c.id.as_str()), Some(b), "a credential for another identity is not");
    tried.insert(b.to_string(), now);
    assert!(plan_auto_pair(&list, local, &none, &tried, now + Duration::from_secs(299)).is_none());
    assert!(plan_auto_pair(&list, local, &none, &tried, now + Duration::from_secs(300)).is_some());
}

#[tokio::test(start_paused = true)]
async fn the_files_page_gets_a_previews_connection_at_once_or_goes_on_without_it() {
    let shelf: Handoffs<u32> = Handoffs::default();
    let started = Clock::now();
    assert_eq!(shelf.take("pc", Duration::from_millis(1500)).await, None, "nothing on offer: no waiting");
    assert_eq!(started.elapsed(), Duration::ZERO);

    let mut offer = shelf.offer("pc");
    let answer = async {
        let reply = offer.requests.recv().await.unwrap();
        let _ = reply.send(7);
    };
    let (taken, _) = tokio::join!(shelf.take("pc", Duration::from_millis(1500)), answer);
    assert_eq!(taken, Some(7));
    assert_eq!(started.elapsed(), Duration::ZERO);

    // An owner that does not answer costs at most the wait.
    let mut offer = shelf.offer("pc");
    let taken = shelf.take("pc", Duration::from_millis(1500)).await;
    assert_eq!(taken, None);
    assert_eq!(started.elapsed(), Duration::from_millis(1500));
    assert!(offer.requests.try_recv().is_ok(), "the request was delivered, only the answer was missing");

    drop(offer);
    assert_eq!(shelf.take("pc", Duration::from_millis(1500)).await, None, "a withdrawn offer is gone");
    assert_eq!(started.elapsed(), Duration::from_millis(1500));
}

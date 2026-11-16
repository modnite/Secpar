//! Asks the account service for relay access with this computer's device key
//! and tries to get a relay address. Prints which step fails; never the
//! credentials or the addresses.
use secpar_core::{account, identity, store::SystemStore};

#[tokio::main]
async fn main() {
    let state = std::path::PathBuf::from(std::env::args().nth(1).expect("state folder"));
    let identity = identity::device(&SystemStore, &state.join("device.txt")).expect("device identity");
    let service = account::Service::new(account::ORIGIN).expect("service");
    {
        let opened = secpar_tunnel::open(std::net::UdpSocket::bind("0.0.0.0:0").expect("socket")).expect("open");
        let relay = opened.relay();
        let stun = tokio::net::lookup_host("stun.cloudflare.com:3478").await.ok().and_then(|mut found| found.find(std::net::SocketAddr::is_ipv4)).expect("stun name");
        let seen = tokio::task::spawn_blocking(move || relay.seen_from_outside(stun)).await;
        println!("seen from outside: {}", matches!(seen, Ok(Some(_))));
    }
    let access = match service.device_relay(&identity).await {
        Ok(access) => access,
        Err(error) => return println!("relay access refused: {error:?}"),
    };
    println!("relay access given, server {}", access.server);
    let Some(server) = tokio::net::lookup_host(&access.server).await.ok().and_then(|mut found| found.find(std::net::SocketAddr::is_ipv4)) else {
        return println!("server name not found");
    };
    let opened = secpar_tunnel::open(std::net::UdpSocket::bind("0.0.0.0:0").expect("socket")).expect("open");
    let relay = opened.relay();
    let got = tokio::task::spawn_blocking(move || relay.start(server, access.username, access.credential)).await;
    println!("relay address given: {}", matches!(got, Ok(Some(_))));
}

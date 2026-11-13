use super::*;
use tokio::io::AsyncReadExt;

const LISTEN: &str = "127.0.0.2";

/// A stand-in engine: one TCP echo port and one UDP echo port on loopback.
async fn engine() -> (u16, u16) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = tcp.accept().await {
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buffer).await {
                    if n == 0 || stream.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let udp_port = udp.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        while let Ok((n, from)) = udp.recv_from(&mut buffer).await {
            let _ = udp.send_to(&buffer[..n], from).await;
        }
    });
    (tcp_port, udp_port)
}

async fn host(identity: &Identity, allowed: Arc<Allowed>, ports: Ports) -> (Host, SocketAddr) {
    let host = serve(
        "127.0.0.1:0".parse().unwrap(),
        identity,
        allowed,
        "127.0.0.1".parse().unwrap(),
        ports,
    )
    .await
    .unwrap();
    let at = host.local_addr().unwrap();
    (host, at)
}

async fn within<T>(what: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), what)
        .await
        .expect("finished in time")
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_and_udp_reach_the_engine_and_return() {
    let (tcp_port, udp_port) = engine().await;
    let ports = Ports { tcp: vec![tcp_port], udp: vec![udp_port] };
    let (host_id, device) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let (_host, at) = host(&host_id, Allowed::new([device.fingerprint()]), ports.clone()).await;
    let _client = within(connect(at, &device, host_id.fingerprint(), LISTEN.parse().unwrap(), ports))
        .await
        .unwrap();

    let mut stream = within(TcpStream::connect((LISTEN, tcp_port))).await.unwrap();
    stream.write_all(b"over tcp").await.unwrap();
    let mut answer = [0u8; 8];
    within(stream.read_exact(&mut answer)).await.unwrap();
    assert_eq!(&answer, b"over tcp");

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let payload = vec![7u8; 1100];
    socket.send_to(&payload, (LISTEN, udp_port)).await.unwrap();
    let mut answer = vec![0u8; 2000];
    let (n, _) = within(socket.recv_from(&mut answer)).await.unwrap();
    assert_eq!(&answer[..n], &payload[..]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_is_not_allowed_gets_no_tunnel() {
    let (tcp_port, udp_port) = engine().await;
    let ports = Ports { tcp: vec![tcp_port], udp: vec![udp_port] };
    let (host_id, device) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let other = Identity::generate().unwrap();
    let (_host, at) = host(&host_id, Allowed::new([other.fingerprint()]), ports.clone()).await;
    let refused = within(connect(at, &device, host_id.fingerprint(), LISTEN.parse().unwrap(), ports)).await;
    assert!(refused.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_with_another_certificate_is_refused() {
    let (tcp_port, udp_port) = engine().await;
    let ports = Ports { tcp: vec![tcp_port], udp: vec![udp_port] };
    let (host_id, device) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let expected = Identity::generate().unwrap();
    let (_host, at) = host(&host_id, Allowed::new([device.fingerprint()]), ports.clone()).await;
    let refused = within(connect(at, &device, expected.fingerprint(), LISTEN.parse().unwrap(), ports)).await;
    assert!(refused.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_port_the_host_does_not_offer_is_not_forwarded() {
    let (tcp_port, udp_port) = engine().await;
    let (other_tcp, _) = engine().await;
    let offered = Ports { tcp: vec![tcp_port], udp: vec![udp_port] };
    let asked = Ports { tcp: vec![tcp_port, other_tcp], udp: vec![udp_port] };
    let (host_id, device) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let (_host, at) = host(&host_id, Allowed::new([device.fingerprint()]), offered).await;
    let _client = within(connect(at, &device, host_id.fingerprint(), LISTEN.parse().unwrap(), asked))
        .await
        .unwrap();
    let mut stream = within(TcpStream::connect((LISTEN, other_tcp))).await.unwrap();
    let _ = stream.write_all(b"hello").await;
    let mut answer = [0u8; 5];
    // The host drops the stream, so nothing is echoed back.
    assert!(within(stream.read_exact(&mut answer)).await.is_err());
}

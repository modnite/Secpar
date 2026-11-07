use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    rustls::{self, pki_types::PrivatePkcs8KeyDer},
    TlsAcceptor,
};

struct Https {
    port: u16,
    pem: String,
    saw_authorization: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Https {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn https(body: Vec<u8>) -> Https {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certified = rcgen::generate_simple_self_signed(vec!["fixture.invalid".into()]).unwrap();
    let pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        STANDARD.encode(certified.cert.der())
    );
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certified.cert.der().clone()],
            PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()).into(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let saw_authorization = Arc::new(AtomicBool::new(false));
    let seen = saw_authorization.clone();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let Ok(mut tls) = acceptor.accept(socket).await else {
            return;
        };
        let mut request = Vec::new();
        loop {
            let mut bytes = [0u8; 1024];
            let n = tls.read(&mut bytes).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            request.extend_from_slice(&bytes[..n]);
            if request.windows(4).any(|b| b == b"\r\n\r\n") {
                break;
            }
            assert!(request.len() < 8192);
        }
        seen.store(
            String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("authorization: basic "),
            Ordering::SeqCst,
        );
        let header=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
        if tls.write_all(header.as_bytes()).await.is_ok() {
            let _ = tls.write_all(&body).await;
        }
        let _ = tls.shutdown().await;
    });
    Https {
        port,
        pem,
        saw_authorization,
        task,
    }
}

#[tokio::test]
async fn only_the_private_certificate_receives_management_credentials() {
    let server = https(br#"{"status":true}"#.to_vec()).await;
    let api = Api::new(server.port, server.pem.as_bytes(), "fixture-password").unwrap();
    assert_eq!(
        api.request(reqwest::Method::GET, "config", None)
            .await
            .unwrap()["status"],
        true
    );
    assert!(server.saw_authorization.load(Ordering::SeqCst));
    let other = https(br#"{"status":true}"#.to_vec()).await;
    let wrong = Api::new(other.port, server.pem.as_bytes(), "must-not-leak").unwrap();
    assert!(wrong
        .request(reqwest::Method::GET, "config", None)
        .await
        .is_err());
    assert!(
        !other.saw_authorization.load(Ordering::SeqCst),
        "Untrusted endpoint received credentials"
    );
}

#[tokio::test]
async fn oversized_management_responses_are_rejected() {
    let server = https(vec![b' '; 1024 * 1024 + 1]).await;
    let api = Api::new(server.port, server.pem.as_bytes(), "fixture-password").unwrap();
    assert_eq!(
        api.request(reqwest::Method::GET, "config", None)
            .await
            .unwrap_err(),
        Error("Private host response exceeds its limit")
    );
}

#[test]
fn private_state_survives_reuse_without_exposing_or_replacing_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("engine");
    private::directory(&state).unwrap();
    let first = password(&state).unwrap();
    let second = password(&state).unwrap();
    assert_eq!(first.0, second.0);
    assert_eq!(format!("{first:?}"), "[redacted]");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let file = state.join("management.secret");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(password(&state).is_err());
    }
}

#[test]
fn a_config_write_never_follows_a_destination_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("unrelated");
    std::fs::write(&outside, b"preserve me").unwrap();
    let state = temp.path().join("engine");
    private::directory(&state).unwrap();
    let config = state.join("sunshine.conf");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &config).unwrap();
    #[cfg(windows)]
    std::fs::write(&config, b"old config").unwrap();
    replace_private(&config, b"new config").unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"preserve me");
    assert_eq!(std::fs::read(config).unwrap(), b"new config");
}

#[tokio::test]
async fn failed_credential_helper_is_reaped_and_private_state_can_be_retried() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = std::env::current_exe().unwrap(); // libtest rejects Sunshine arguments.
    let config = || Config {
        executable: fixture.clone(),
        state: temp.path().join("engine"),
        base_port: 48989,
        capture: Capture::Automatic,
        encoder: Encoder::Automatic,
        audio_sink: None,
    };
    for _ in 0..2 {
        assert!(matches!(
            Host::start(config()).await,
            Err(Error("Bundled host credential initialization failed"))
        ));
    }
}

#[cfg(unix)]
#[test]
fn upstream_writes_cannot_follow_an_existing_state_hardlink() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("unrelated");
    std::fs::write(&outside, b"preserve me").unwrap();
    let state = temp.path().join("engine");
    private::directory(&state).unwrap();
    std::fs::hard_link(&outside, state.join("management.json")).unwrap();
    #[cfg(unix)]
    assert!(private::ordinary_or_missing(&state.join("management.json")).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"preserve me");
}

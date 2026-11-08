//! The host's TURN credentials from the rendezvous Worker (`POST /v1/turn?role=host`,
//! docs/remote-protocol.md), the same for every desktop host.

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// A UDP TURN server with the long-term credentials to use on it.
#[derive(Clone, PartialEq)]
pub struct TurnServer {
    pub addr: SocketAddr,
    pub user: String,
    pub pass: String,
}

impl std::fmt::Debug for TurnServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TurnServer({})", self.addr) // never the credentials
    }
}

/// `https://...` base of a `wss://`/`ws://` Worker URL.
pub fn http_base(url: &str) -> String {
    url.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1)
}

/// `<base>/v1/turn?role=host&pc=..&ts=..&sig=..`; `sig_hex` is the host signature for `pc`/`ts`
/// (`secpar_network::relay_auth::host_signature`, hex).
pub fn turn_url(base: &str, pc: &str, ts: u64, sig_hex: &str) -> String {
    format!("{}/v1/turn?role=host&pc={pc}&ts={ts}&sig={sig_hex}", http_base(base))
}

/// The first UDP `turn:` server in the Worker's `iceServers` reply.
pub fn parse_turn_reply(body: &serde_json::Value) -> Option<TurnServer> {
    let ice = &body["iceServers"];
    let servers: Vec<&serde_json::Value> = ice.as_array().map(|a| a.iter().collect()).unwrap_or_else(|| vec![ice]);
    for s in servers {
        let (Some(user), Some(pass)) = (s["username"].as_str(), s["credential"].as_str()) else { continue };
        let urls: Vec<&str> = match &s["urls"] {
            serde_json::Value::Array(a) => a.iter().filter_map(|u| u.as_str()).collect(),
            u => u.as_str().into_iter().collect(),
        };
        for u in urls {
            if !u.starts_with("turn:") || (u.contains("transport=") && !u.contains("transport=udp")) {
                continue;
            }
            let hp = u.trim_start_matches("turn:").split('?').next()?;
            let hp = if hp.contains(':') { hp.to_string() } else { format!("{hp}:3478") };
            if let Some(addr) = hp.to_socket_addrs().ok()?.find(|a| a.is_ipv4()) {
                return Some(TurnServer { addr, user: user.to_string(), pass: pass.to_string() });
            }
        }
    }
    None
}

/// Fetches the credentials. Err is a message safe to log (no credentials, no URL).
pub async fn fetch_turn(url: &str) -> Result<TurnServer, String> {
    let r = reqwest::Client::new().post(url).timeout(Duration::from_secs(5)).send().await.map_err(|e| format!("credentials request failed: {}", e.without_url()))?;
    if !r.status().is_success() {
        return Err(format!("credentials refused ({})", r.status()));
    }
    let body: serde_json::Value = r.json().await.map_err(|e| format!("bad credentials reply: {}", e.without_url()))?;
    parse_turn_reply(&body).ok_or_else(|| "no UDP TURN server in the reply".to_string())
}

/// [`fetch_turn`] for callers on a plain thread (never on an async worker thread).
pub fn fetch_turn_blocking(url: &str) -> Result<TurnServer, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(fetch_turn(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn url_matches_the_worker_contract() {
        assert_eq!(turn_url("wss://w.example", "pc1", 7, "ab"), "https://w.example/v1/turn?role=host&pc=pc1&ts=7&sig=ab");
        assert_eq!(http_base("ws://127.0.0.1:1"), "http://127.0.0.1:1");
    }

    #[test]
    fn picks_the_first_udp_turn_url() {
        let reply = json!({"iceServers":[{"urls":["stun:1.2.3.4:3478","turn:127.0.0.1:3478?transport=tcp","turn:127.0.0.1:53?transport=udp","turn:127.0.0.1:3478"],"username":"u","credential":"c"}]});
        let s = parse_turn_reply(&reply).unwrap();
        assert_eq!((s.addr, s.user.as_str(), s.pass.as_str()), ("127.0.0.1:53".parse().unwrap(), "u", "c"));
        assert_eq!(format!("{s:?}"), "TurnServer(127.0.0.1:53)", "Debug never shows credentials");
        // a single object instead of a list, and the default port
        let single = json!({"iceServers":{"urls":"turn:127.0.0.1","username":"u","credential":"c"}});
        assert_eq!(parse_turn_reply(&single).unwrap().addr, "127.0.0.1:3478".parse().unwrap());
        assert!(parse_turn_reply(&json!({"iceServers":[{"urls":["stun:x:1"],"username":"u","credential":"c"}]})).is_none());
        assert!(parse_turn_reply(&json!({"iceServers":[{"urls":["turn:127.0.0.1"]}]})).is_none(), "no credentials");
        assert!(parse_turn_reply(&json!({})).is_none());
    }

    #[tokio::test]
    async fn fetches_from_a_fake_worker() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/turn")).and(query_param("role", "host")).and(query_param("pc", "pc1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"iceServers":[{"urls":["turn:127.0.0.1:3478?transport=udp"],"username":"u","credential":"c"}]})))
            .mount(&server).await;
        let url = turn_url(&server.uri(), "pc1", 1, "sig");
        assert_eq!(fetch_turn(&url).await.unwrap().addr, "127.0.0.1:3478".parse().unwrap());
        let bad = turn_url(&server.uri(), "other", 1, "sig");
        assert!(fetch_turn(&bad).await.unwrap_err().contains("refused"));
    }
}

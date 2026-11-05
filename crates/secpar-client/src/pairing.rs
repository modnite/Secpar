use std::collections::HashMap;
use url::Url;

// Do not derive Debug: a QR contains a live one-time credential.
#[derive(Clone)]
pub struct Pairing {
    pub name: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub fingerprint: [u8; 32],
    pub otp: String,
    pub relay_url: Option<String>,
    pub pc_id: Option<String>,
}

pub fn fingerprint(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid host fingerprint".into());
    }
    let mut out = [0; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "Invalid fingerprint")?;
    }
    Ok(out)
}

impl Pairing {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 16_384 {
            return Err("Pairing link is too long".into());
        }
        let u = Url::parse(text.trim()).map_err(|_| "Paste a complete Secpar pairing link")?;
        if !matches!(u.scheme(), "secpar" | "pcremote")
            || u.host_str() != Some("pair")
            || !u.path().is_empty()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
        {
            return Err("Not a Secpar pairing link".into());
        }
        let mut q = HashMap::new();
        for (k, v) in u.query_pairs() {
            if q.insert(k.to_string(), v.to_string()).is_some() {
                return Err("Ambiguous pairing link".into());
            }
        }
        let get = |key: &str| q.get(key).cloned().unwrap_or_default();
        let port = get("p")
            .parse::<u16>()
            .ok()
            .filter(|v| *v != 0)
            .ok_or("Invalid host port")?;
        let fingerprint = fingerprint(&get("fp"))?;
        let otp = get("t");
        if otp.is_empty() || otp.len() > 256 {
            return Err("Missing or invalid pairing code".into());
        }
        let hosts: Vec<String> = get("h")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if hosts.len() > 16 || hosts.iter().any(|h| h.parse::<std::net::IpAddr>().is_err()) {
            return Err("Invalid host addresses".into());
        }
        let relay_url = q.get("r").cloned();
        let pc_id = q.get("pc").cloned().filter(|s| !s.is_empty());
        if let Some(relay) = &relay_url {
            let r = Url::parse(relay).map_err(|_| "Invalid relay address")?;
            if r.scheme() != "wss"
                || r.host_str().is_none()
                || r.path() != "/v1/phone"
                || !r.username().is_empty()
                || r.password().is_some()
                || r.fragment().is_some()
                || pc_id.is_none()
            {
                return Err("Invalid secure relay address".into());
            }
            if r.query_pairs()
                .find(|(k, _)| k == "pc")
                .map(|(_, v)| v.to_string())
                != pc_id
            {
                return Err("Relay host identity mismatch".into());
            }
        }
        if hosts.is_empty() && relay_url.is_none() {
            return Err("Pairing link has no connection address".into());
        }
        Ok(Self {
            name: q
                .get("n")
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or("PC".into()),
            hosts,
            port,
            fingerprint,
            otp,
            relay_url,
            pc_id,
        })
    }

    pub fn direct_urls(&self) -> Vec<String> {
        self.hosts
            .iter()
            .map(|h| {
                if h.contains(':') {
                    format!("wss://[{h}]:{}/v1", self.port)
                } else {
                    format!("wss://{h}:{}/v1", self.port)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn link() -> String {
        format!(
            "secpar://pair?n=Desktop&h=192.168.1.2,::1&p=48999&fp={}&t=one-use",
            "42".repeat(32)
        )
    }
    #[test]
    fn supports_both_apps_and_ipv6() {
        let p = Pairing::parse(&link()).unwrap();
        assert_eq!(p.direct_urls()[1], "wss://[::1]:48999/v1");
        assert!(Pairing::parse(&link().replace("secpar:", "pcremote:")).is_ok());
    }
    #[test]
    fn rejects_ambiguous_or_insecure_codes() {
        for bad in [
            link() + "&t=other",
            link().replace("48999", "0"),
            link().replace("192.168.1.2", "user@evil.test"),
            link() + "#fragment",
            link() + "&r=ws%3A%2F%2Fevil.test%2Fv1%2Fphone&pc=pc1",
        ] {
            assert!(Pairing::parse(&bad).is_err());
        }
    }
    #[test]
    fn binds_relay_to_expected_host() {
        let relay = "wss://relay.example/v1/phone?pc=pc1&ts=1&sig=abc";
        let mut u = Url::parse(&link()).unwrap();
        u.query_pairs_mut()
            .append_pair("r", relay)
            .append_pair("pc", "pc1");
        assert!(Pairing::parse(u.as_str()).is_ok());
        assert!(Pairing::parse(&u.to_string().replace("&pc=pc1", "&pc=pc2")).is_err());
    }
}

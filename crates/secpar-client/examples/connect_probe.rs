//! Times connection establishment against a real host. A development check, not part of the product.
//!
//!   connect_probe [--hosts a,b,REAL] [--raw] <file holding a pairing link>
//!
//! Pairs (spends the one-time code), then reconnects twice with the saved credential. Prints the
//! time of each step. `--hosts` replaces the address list of the link; `REAL` stands for the first
//! address in the link. `--raw` also times a plain TCP connect to every address (25 s limit),
//! which shows what the operating system does with a dead one. The link is read from a file so it
//! never shows up in a process list or a log. Nothing here prints the link or any credential.
use secpar_client::control::Connected;
use secpar_client::pairing::Pairing;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let take = |args: &mut Vec<String>, flag: &str| -> Option<String> {
        let at = args.iter().position(|a| a == flag)?;
        args.remove(at);
        (at < args.len()).then(|| args.remove(at))
    };
    let hosts = take(&mut args, "--hosts");
    let save = take(&mut args, "--save");
    let saved = take(&mut args, "--computer");
    let raw = args.iter().position(|a| a == "--raw").map(|at| args.remove(at)).is_some();
    if let Some(file) = saved {
        // Reconnect from a credential kept by an earlier run, in a process that has no memory of routes.
        let computer: secpar_client::control::Computer = serde_json::from_str(&std::fs::read_to_string(file).expect("computer file")).expect("computer");
        for n in 1..=2 {
            let start = Instant::now();
            let conn = Connected::reconnect(computer.clone(), "connect-probe").await.expect("reconnect");
            println!("cold reconnect {n}: {} ms (via {:?})", start.elapsed().as_millis(), conn.direct_host);
            for t in &conn.route_log { println!("  route {} start {} ms end {} ms {}", t.label, t.started_ms, t.ended_ms, t.outcome); } //ROUTELOG
            drop(conn);
        }
        return;
    }
    let link = std::fs::read_to_string(&args[0]).expect("link file");
    let link = link.trim().to_string();
    // The address list can be rewritten before parsing, because a long list is refused by the parser.
    let link = match hosts {
        Some(spec) => {
            let mut url = url::Url::parse(&link).expect("link");
            let pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let real = pairs.iter().find(|(k, _)| k == "h").and_then(|(_, v)| v.split(',').next()).unwrap_or("").to_string();
            let list: Vec<String> = spec.split(',').map(|h| if h == "REAL" { real.clone() } else { h.to_string() }).collect();
            url.query_pairs_mut().clear().extend_pairs(pairs.iter().map(|(k, v)| if k == "h" { (k.clone(), list.join(",")) } else { (k.clone(), v.clone()) }));
            url.to_string()
        }
        None => link,
    };
    let mut pairing = Pairing::parse(&link).expect("pairing link");
    pairing.relay_url = None;
    println!("addresses: {} port {}", pairing.hosts.len(), pairing.port);
    if raw {
        let port = pairing.port;
        let tasks: Vec<_> = pairing
            .hosts
            .iter()
            .cloned()
            .map(|h| {
                tokio::spawn(async move {
                    let start = Instant::now();
                    let r = tokio::time::timeout(Duration::from_secs(25), tokio::net::TcpStream::connect((h.as_str(), port))).await;
                    let how = match r {
                        Ok(Ok(_)) => "connected".to_string(),
                        Ok(Err(e)) => format!("error {:?}", e.kind()),
                        Err(_) => "no answer in 25 s".to_string(),
                    };
                    (h, how, start.elapsed())
                })
            })
            .collect();
        for t in tasks {
            let (h, how, took) = t.await.unwrap();
            println!("raw tcp {h}: {how} after {} ms", took.as_millis());
        }
    }
    let start = Instant::now();
    let conn = Connected::pair(pairing, "connect-probe").await.expect("pairing");
    println!("pair: {} ms (via {:?})", start.elapsed().as_millis(), conn.direct_host);
    for t in &conn.route_log { println!("  route {} start {} ms end {} ms {}", t.label, t.started_ms, t.ended_ms, t.outcome); } //ROUTELOG
    let computer = conn.computer.clone();
    if let Some(file) = save {
        std::fs::write(file, serde_json::to_string(&computer).unwrap()).unwrap();
    }
    drop(conn);
    for n in 1..=2 {
        let start = Instant::now();
        let conn = Connected::reconnect(computer.clone(), "connect-probe").await.expect("reconnect");
        println!("reconnect {n}: {} ms (via {:?})", start.elapsed().as_millis(), conn.direct_host);
        for t in &conn.route_log { println!("  route {} start {} ms end {} ms {}", t.label, t.started_ms, t.ended_ms, t.outcome); } //ROUTELOG
        drop(conn);
    }
}

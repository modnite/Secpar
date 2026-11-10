//! Racing connection routes. A computer is reachable by several LAN addresses and by the relay.
//! Some of them are dead (container bridges, VPNs, stale DHCP leases). Windows waits about 21 s
//! for a SYN to a dead address, so trying routes one after another can take minutes.
//! Here the routes start staggered and run side by side. The first one that finishes wins and the
//! rest are dropped, which closes their sockets.
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{future::Future, time::Duration};
use tokio::time::Instant;

/// Delays and limits for one connection. `Default` is what the apps use.
#[derive(Clone, Debug)]
pub struct Timing {
    /// A LAN address starts this long after the one before it, or at once if that one failed.
    /// See `start_offset`.
    pub stagger: Duration,
    /// The relay joins the race this long after the start, so it stays unused while a LAN
    /// address answers within this time.
    pub relay_delay: Duration,
    /// Head start the relay gets when the last connection to this computer went through it.
    pub direct_after_relay: Duration,
    /// TCP connect to a LAN address. The operating system's own limit is 21 s on Windows.
    pub direct_tcp: Duration,
    /// TCP, TLS and WebSocket upgrade together, for a LAN address.
    pub direct_open: Duration,
    /// The same for the relay (internet).
    pub relay_open: Duration,
    /// How long the host may take to answer hello or pair.
    pub answer: Duration,
    /// After `paired` on a LAN address: how long to wait for the host's remote-access credentials.
    /// A host sends them in the same breath, and one without remote access never sends them.
    pub remote_wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            stagger: Duration::from_millis(200),
            relay_delay: Duration::from_millis(1000),
            direct_after_relay: Duration::from_millis(300),
            direct_tcp: Duration::from_millis(2500),
            direct_open: Duration::from_secs(5),
            relay_open: Duration::from_secs(8),
            answer: Duration::from_secs(8),
            remote_wait: Duration::from_millis(750),
        }
    }
}

impl Timing {
    /// When the `k`-th LAN address starts, counted from the first. The first few get the full
    /// stagger, because the likely candidates sit at the front. After that the pace doubles, so a
    /// long list of old addresses does not hold the last one back for seconds.
    pub fn start_offset(&self, k: usize) -> Duration {
        let full = k.min(3) as u32;
        let quick = k.saturating_sub(3) as u32;
        self.stagger * full + self.stagger / 2 * quick
    }
}

/// What one route attempt came to.
pub enum Attempt<T> {
    /// Connected. The race ends and the other routes are dropped.
    Done(T),
    /// This route is unusable. Start the next one now.
    Retry(String),
    /// Stop everything. Used when the host answered and said no.
    Fatal(String),
}

/// One route as it ran. Addresses only, never a URL with a signature.
#[derive(Clone, Debug)]
pub struct Trace {
    pub label: String,
    pub started_ms: u64,
    pub ended_ms: u64,
    pub outcome: String,
}

/// Runs `attempt(i)` for every candidate `i`. Candidate `i` starts at `starts[i]` after the call,
/// or at once when an earlier start has failed. Returns the winner's index and value, or the
/// error, together with a trace of each route that ran.
pub async fn race<T, F, Fut>(
    labels: Vec<String>,
    starts: Vec<Duration>,
    mut attempt: F,
) -> (Result<(usize, T), String>, Vec<Trace>)
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let begin = Instant::now();
    let ms = |t: Instant| t.saturating_duration_since(begin).as_millis() as u64;
    let mut order: Vec<usize> = (0..labels.len()).collect();
    order.sort_by_key(|i| starts[*i]);
    let mut traces: Vec<Option<Trace>> = vec![None; labels.len()];
    let mut running = FuturesUnordered::new();
    let (mut next, mut pull) = (0, false);
    let mut last_error = "No connection address".to_string();
    let result = loop {
        while next < order.len() && (pull || begin + starts[order[next]] <= Instant::now()) {
            let i = order[next];
            traces[i] = Some(Trace { label: labels[i].clone(), started_ms: ms(Instant::now()), ended_ms: 0, outcome: "running".into() });
            let fut = attempt(i);
            running.push(async move { (i, fut.await) });
            next += 1;
            pull = false;
        }
        if running.is_empty() {
            if next >= order.len() {
                break Err(last_error);
            }
            pull = true;
            continue;
        }
        let wake = order.get(next).map(|i| begin + starts[*i]);
        tokio::select! {
            Some((i, outcome)) = running.next() => {
                let t = traces[i].as_mut().expect("started");
                t.ended_ms = ms(Instant::now());
                match outcome {
                    Attempt::Done(v) => { t.outcome = "connected".into(); break Ok((i, v)); }
                    Attempt::Fatal(e) => { t.outcome = format!("refused: {e}"); break Err(e); }
                    Attempt::Retry(e) => { t.outcome = format!("failed: {e}"); last_error = e; pull = true; }
                }
            }
            _ = async { match wake { Some(w) => tokio::time::sleep_until(w).await, None => std::future::pending().await } } => {}
        }
    };
    let end = ms(Instant::now());
    let traces = traces
        .into_iter()
        .flatten()
        .map(|mut t| {
            if t.outcome == "running" {
                t.ended_ms = end;
                t.outcome = "cancelled".into();
            }
            t
        })
        .collect();
    (result, traces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("r{i}")).collect()
    }
    /// A route that finishes `after` from its own start, one way or the other.
    async fn route(after: Duration, ok: bool) -> Attempt<()> {
        tokio::time::sleep(after).await;
        if ok { Attempt::Done(()) } else { Attempt::Retry("dead".into()) }
    }

    #[tokio::test(start_paused = true)]
    async fn dead_routes_do_not_add_up() {
        // Four routes that each take 21 s to fail, then one that answers in 50 ms.
        let t0 = Instant::now();
        let starts = (0..5).map(|i| ms(200 * i)).collect();
        let (r, trace) = race(names(5), starts, |i| route(if i < 4 { ms(21_000) } else { ms(50) }, i == 4)).await;
        assert_eq!(r.unwrap().0, 4);
        assert_eq!(t0.elapsed(), ms(850), "start 800 ms in, 50 ms to answer");
        assert_eq!(trace.iter().filter(|t| t.outcome == "cancelled").count(), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn first_success_cancels_the_rest() {
        let finished = Arc::new(AtomicUsize::new(0));
        let f = finished.clone();
        let (r, trace) = race(names(3), vec![ms(0), ms(0), ms(0)], |i| {
            let f = f.clone();
            async move {
                tokio::time::sleep(ms([300, 100, 200][i])).await;
                f.fetch_add(1, Ordering::SeqCst);
                Attempt::Done(i)
            }
        })
        .await;
        assert_eq!(r.unwrap(), (1, 1));
        tokio::time::sleep(ms(1000)).await;
        assert_eq!(finished.load(Ordering::SeqCst), 1, "the losers were dropped, not left running");
        assert_eq!(trace[1].outcome, "connected");
        assert_eq!(trace[0].outcome, "cancelled");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_starts_the_next_route_at_once() {
        let t0 = Instant::now();
        // Second route is due at 10 s, but the first fails after 30 ms.
        let (r, trace) = race(names(2), vec![ms(0), ms(10_000)], |i| route(if i == 0 { ms(30) } else { ms(20) }, i == 1)).await;
        assert_eq!(r.unwrap().0, 1);
        assert_eq!(t0.elapsed(), ms(50));
        assert_eq!(trace[1].started_ms, 30);
    }

    #[tokio::test(start_paused = true)]
    async fn every_route_failing_ends_with_the_last_error() {
        let t0 = Instant::now();
        let (r, _) = race(names(3), vec![ms(0), ms(200), ms(400)], |i| async move {
            tokio::time::sleep(ms(2500)).await;
            Attempt::<()>::Retry(format!("e{i}"))
        })
        .await;
        assert_eq!(r.err().unwrap(), "e2");
        assert_eq!(t0.elapsed(), ms(2900), "bounded by the last start plus one timeout, not by the sum");
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_stops_the_race() {
        let (r, _) = race(names(2), vec![ms(0), ms(0)], |i| async move {
            if i == 0 {
                tokio::time::sleep(ms(10)).await;
                Attempt::<()>::Fatal("revoked".into())
            } else {
                tokio::time::sleep(ms(500)).await;
                Attempt::Done(())
            }
        })
        .await;
        assert_eq!(r.err().unwrap(), "revoked");
    }

    #[tokio::test(start_paused = true)]
    async fn relay_joins_late_and_only_when_needed() {
        // r0..r2 are LAN addresses, r3 is the relay due after 1 s.
        let relay_started = Arc::new(AtomicUsize::new(0));
        let starts = vec![ms(0), ms(200), ms(400), ms(1000)];
        let flag = relay_started.clone();
        let t0 = Instant::now();
        let (r, _) = race(names(4), starts.clone(), |i| {
            let flag = flag.clone();
            async move {
                if i == 3 {
                    flag.fetch_add(1, Ordering::SeqCst);
                    return route(ms(150), true).await;
                }
                route(ms(2500), false).await
            }
        })
        .await;
        assert_eq!(r.unwrap().0, 3);
        assert_eq!(t0.elapsed(), ms(1150), "the relay answers 150 ms after its start at 1 s");

        // A LAN address that answers in 300 ms: the relay is never asked.
        relay_started.store(0, Ordering::SeqCst);
        let flag = relay_started.clone();
        let (r, _) = race(names(4), starts, |i| {
            let flag = flag.clone();
            async move {
                if i == 3 {
                    flag.fetch_add(1, Ordering::SeqCst);
                }
                route(ms(300), i == 1).await
            }
        })
        .await;
        assert_eq!(r.unwrap().0, 1);
        assert_eq!(relay_started.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn the_pace_doubles_after_the_third_address() {
        let t = Timing::default();
        let at: Vec<u64> = (0..6).map(|k| t.start_offset(k).as_millis() as u64).collect();
        assert_eq!(at, [0, 200, 400, 600, 700, 800]);
        assert!(t.start_offset(15) < Duration::from_millis(2000), "sixteen addresses are all under way within two seconds");
    }
}

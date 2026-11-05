//! Platform-independent STUN discovery and TURN relay, shared by hosts and clients.
pub mod stun;
pub mod turn;
fn random<const N: usize>() -> [u8; N] {
    use ring::rand::SecureRandom;
    let mut out = [0; N];
    ring::rand::SystemRandom::new()
        .fill(&mut out)
        .expect("OS random source");
    out
}
#[cfg(test)]
fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

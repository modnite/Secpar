//! Reed-Solomon FEC over GF(2^8), compatible with Backblaze JavaReedSolomon.
//! Data shards are grouped into blocks of at most `BLOCK` shards, each with its own parity.

use super::SHARD;
use reed_solomon_erasure::galois_8::ReedSolomon;
use std::{collections::VecDeque, sync::{Arc, Mutex, OnceLock}};

// Building the systematic coding matrix performs a matrix inversion. Repeating
// that for every encoded frame blocks both video delivery and cursor polling.
// Keep a bounded LRU shared by streams; ReedSolomon is internally synchronized.
fn coder(data: usize, parity: usize) -> Option<Arc<ReedSolomon>> {
    type Cache = VecDeque<((usize, usize), Arc<ReedSolomon>)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    let key = (data, parity);
    {
        let mut entries = cache.lock().unwrap();
        if let Some(index) = entries.iter().position(|(k, _)| *k == key) {
            let entry = entries.remove(index).unwrap();
            let result = entry.1.clone();
            entries.push_back(entry);
            return Some(result);
        }
    }
    let result = Arc::new(ReedSolomon::new(data, parity).ok()?);
    let mut entries = cache.lock().unwrap();
    // Another stream may have populated it while this matrix was constructed.
    if let Some((_, existing)) = entries.iter().find(|(k, _)| *k == key) { return Some(existing.clone()); }
    if entries.len() >= 128 { entries.pop_front(); }
    entries.push_back((key, result.clone()));
    Some(result)
}

pub const BLOCK: usize = 64;

/// `(k_b, m_b)` for each block of a frame with `k` data shards at FEC percentage `p`.
pub fn layout(k: usize, p: usize) -> Vec<(usize, usize)> {
    (0..k.max(1))
        .step_by(BLOCK)
        .map(|start| {
            let kb = (k.max(1) - start).min(BLOCK);
            (kb, (kb * p).div_ceil(100).max(1))
        })
        .collect()
}

/// Splits `payload` into `k` zero-padded data shards plus per-block parity, in layout order.
pub fn shards(payload: &[u8], p: usize) -> (usize, Vec<[u8; SHARD]>) {
    let k = payload.len().div_ceil(SHARD).max(1);
    let mut chunks = payload.chunks(SHARD);
    let mut out = Vec::new();
    for (kb, mb) in layout(k, p) {
        let mut block = vec![[0u8; SHARD]; kb + mb];
        for s in block.iter_mut().take(kb) {
            if let Some(c) = chunks.next() {
                s[..c.len()].copy_from_slice(c);
            }
        }
        coder(kb, mb).expect("kb ≤ 64, mb ≤ 64").encode(&mut block).expect("encode");
        out.extend(block);
    }
    (k, out)
}

/// Rebuilds the `k` data shards (concatenated, still padded) from shards in layout order.
pub fn rebuild(k: usize, p: usize, mut shards: Vec<Option<Vec<u8>>>) -> Option<Vec<u8>> {
    let blocks = layout(k, p);
    if shards.len() != blocks.iter().map(|(kb, mb)| kb + mb).sum::<usize>()
        || shards.iter().flatten().any(|s| s.len() != SHARD)
    {
        return None;
    }
    let mut out = Vec::with_capacity(k * SHARD);
    let mut at = 0;
    for (kb, mb) in blocks {
        let block = &mut shards[at..at + kb + mb];
        if block.iter().take(kb).any(Option::is_none) {
            if block.iter().flatten().count() < kb {
                return None;
            }
            coder(kb, mb)?.reconstruct_data(block).ok()?;
        }
        for s in block.iter().take(kb) {
            out.extend_from_slice(s.as_ref()?);
        }
        at += kb + mb;
    }
    Some(out)
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn cached_encoder_preserves_uncached_wire_bytes() {
        for k in [1, 8, 32, 64, 65, 170] {
            let data=vec![42u8; k*SHARD-7];
            for p in [5, 15, 50] {
                let (_, actual)=shards(&data,p);
                let mut expected=Vec::new(); let mut input=data.chunks(SHARD);
                for (kb,mb) in layout(k,p) {
                    let mut block=vec![[0u8;SHARD];kb+mb];
                    for shard in block.iter_mut().take(kb) { if let Some(bytes)=input.next() { shard[..bytes.len()].copy_from_slice(bytes); } }
                    ReedSolomon::new(kb,mb).unwrap().encode(&mut block).unwrap(); expected.extend(block);
                }
                assert_eq!(actual,expected,"k={k},p={p}");
            }
        }
    }
    #[test]
    fn streams_can_encode_and_recover_concurrently() {
        let handles:Vec<_>=(0..4).map(|n|std::thread::spawn(move|| {
            for k in 1..=64 {
                let data=vec![n as u8;k*SHARD-1]; let (count,parts)=shards(&data,15);
                let mut received:Vec<_>=parts.into_iter().map(|p|Some(p.to_vec())).collect(); received[0]=None;
                let rebuilt=rebuild(count,15,received).unwrap(); assert_eq!(&rebuilt[..data.len()],&data);
            }
        })).collect();
        for handle in handles {handle.join().unwrap();}
    }
}

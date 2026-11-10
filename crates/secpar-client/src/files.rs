//! Browsing a computer's files and moving files in both directions: `fs.list`, `fs.get` and the
//! `file.*` messages from docs/protocol.md. `Engine` holds all the protocol decisions and does no
//! network I/O, so it is tested directly. `run` connects it to a paired computer.
//!
//! Downloads: `fs.get` makes the host send `file.offer`s. Every offer is accepted, one at a time,
//! into the save folder. The host sends chunks (`[1][uuid][u64 offset][data]`) and then `file.end`
//! with the SHA-256 of the whole file. The file only gets its final name when the hash matches.
//!
//! Uploads: `file.begin`, wait for `file.ready`, send chunks, `file.end` with the SHA-256, wait for
//! `file.done`. The host decides where an upload lands (its received-files folder).
use crate::control::Connected;
use futures_util::{SinkExt, StreamExt};
use ring::digest::{Context, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Chunk payload size, per protocol (at most 256 KiB).
pub const CHUNK: usize = 256 * 1024;
/// The host forgets a silent client after 60 seconds.
const PING: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Full path to give back to `fs.list` and `fs.get`.
    pub path: String,
    pub dir: bool,
    pub size: u64,
    /// Unix seconds.
    pub mtime: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Queued,
    Active,
    Done,
    Failed,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    pub id: String,
    /// File name, or `folder/file` for a folder download.
    pub name: String,
    pub upload: bool,
    pub size: u64,
    pub done: u64,
    pub phase: Phase,
    /// Where a finished file ended up.
    pub path: String,
    /// Why a transfer failed, in plain words.
    pub error: String,
}
impl Transfer {
    fn new(id: &str, name: &str, upload: bool, size: u64) -> Self {
        Self { id: id.into(), name: name.into(), upload, size, done: 0, phase: Phase::Queued, path: String::new(), error: String::new() }
    }
    pub fn finished(&self) -> bool {
        matches!(self.phase, Phase::Done | Phase::Failed)
    }
}

pub enum Request {
    List(String),
    Get(String),
    /// Upload this file under its own name.
    Send(PathBuf),
    /// Upload this file under `name`, which may be `folder/file` to keep a folder layout.
    SendAs(PathBuf, String),
    Cancel(String),
    /// Where later downloads go.
    SaveTo(PathBuf),
    Stop,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// What the host offers. Both are true when the host did not say.
    Ready { browse: bool, upload: bool },
    Listed { path: String, entries: Vec<Entry> },
    /// A list or download request the host refused.
    Problem(String),
    Transfer(Transfer),
    Closed(String),
}
#[derive(Debug, PartialEq)]
pub enum Out {
    Text(Value),
    Bin(Vec<u8>),
    Note(Notice),
}

/// A file name that is safe on every system: base name only, no reserved Windows device names or
/// characters Windows forbids. Matches what the hosts do.
pub fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base.chars().map(|c| if c < ' ' || "<>:\"/\\|?*".contains(c) { '_' } else { c }).take(200).collect();
    s = s.trim_end_matches(['.', ' ']).trim_start().to_string();
    if s.is_empty() {
        return "file".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0');
    if reserved {
        s.insert(0, '_');
    }
    s
}

/// A host-supplied relative path ("Photos/2024/a.jpg") made safe: every part sanitised and `.`,
/// `..`, empty parts, drive and UNC prefixes dropped, so it can never leave the save folder.
pub fn safe_rel(name: &str) -> PathBuf {
    let parts: Vec<String> = name.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..").map(sanitize).collect();
    let keep = parts.len().saturating_sub(16);
    let p: PathBuf = parts[keep..].iter().collect();
    if p.as_os_str().is_empty() {
        PathBuf::from("file")
    } else {
        p
    }
}

/// `dir/name`, or `name (1)`, `name (2)` and so on if taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut p = dir.join(name);
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    p
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
pub fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let h: String = s.chars().filter(|c| *c != '-').collect();
    if h.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(h.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}
fn format_uuid(b: &[u8; 16]) -> String {
    let h = hex(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}
fn new_id() -> ([u8; 16], String) {
    let mut b = [0u8; 16];
    SystemRandom::new().fill(&mut b).expect("system RNG");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    (b, format_uuid(&b))
}
fn chunk_frame(id: &[u8; 16], offset: u64, data: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(25 + data.len());
    f.push(1);
    f.extend_from_slice(id);
    f.extend_from_slice(&offset.to_be_bytes());
    f.extend_from_slice(data);
    f
}
/// `(id, offset, data)` of a file chunk frame.
pub fn parse_chunk(b: &[u8]) -> Option<([u8; 16], u64, &[u8])> {
    if b.len() < 25 || b[0] != 1 {
        return None;
    }
    Some((b[1..17].try_into().ok()?, u64::from_be_bytes(b[17..25].try_into().ok()?), &b[25..]))
}

/// Plain words for a `file.error` code.
pub fn reason(code: &str) -> &'static str {
    match code {
        "denied" => "The computer refused the file. File transfer may be turned off there.",
        "disk_full" => "The computer is out of disk space.",
        "too_large" => "The file is larger than the computer expected.",
        "bad_hash" => "The file arrived damaged. Try again.",
        "unsupported" => "File transfer is turned off on this computer.",
        _ => "The computer could not finish the transfer.",
    }
}
/// What the host's "file transfer is off" refusal turns into.
pub const OFF: &str = "File transfer is turned off on this computer.";
/// Plain words for an `error` reply to `fs.list` or `fs.get`.
fn problem(code: &str, of: &str) -> String {
    match (code, of) {
        ("unsupported", _) => OFF.into(),
        (_, "fs.get") => "That could not be downloaded.".into(),
        _ => "That folder could not be opened.".into(),
    }
}

fn entry(v: &Value) -> Option<Entry> {
    Some(Entry {
        name: v["name"].as_str()?.into(),
        path: v["path"].as_str()?.into(),
        dir: v["dir"].as_bool().unwrap_or(false),
        size: v["size"].as_u64().unwrap_or(0),
        mtime: v["mtime"].as_u64().unwrap_or(0),
    })
}

struct Offer {
    id: String,
    name: String,
    size: u64,
}
struct Download {
    key: [u8; 16],
    dir: PathBuf,
    name: String,
    part: PathBuf,
    file: File,
    got: u64,
    hash: Context,
    /// The host's `file.end` reached us before the last chunk. Finish when the data is in.
    end: Option<String>,
    t: Transfer,
}
#[derive(PartialEq)]
enum Step {
    /// Not announced to the host yet.
    Queued,
    /// `file.begin` sent, waiting for `file.ready`.
    Waiting,
    Sending,
    /// `file.end` sent, waiting for `file.done`.
    Finishing,
}
struct Upload {
    key: [u8; 16],
    file: File,
    sent: u64,
    hash: Context,
    step: Step,
    t: Transfer,
}

pub struct Engine {
    save_dir: PathBuf,
    offers: VecDeque<Offer>,
    seen: std::collections::HashSet<String>,
    download: Option<Download>,
    uploads: VecDeque<Upload>,
}

impl Engine {
    pub fn new(save_dir: PathBuf) -> Self {
        Self { save_dir, offers: VecDeque::new(), seen: Default::default(), download: None, uploads: VecDeque::new() }
    }

    fn note(t: &Transfer) -> Out {
        Out::Note(Notice::Transfer(t.clone()))
    }

    /// True while upload chunks are waiting to be sent: call `pump` until it returns None.
    pub fn sending(&self) -> bool {
        self.uploads.front().is_some_and(|u| u.step == Step::Sending)
    }
    /// True while any transfer is queued or moving.
    pub fn busy(&self) -> bool {
        self.download.is_some() || !self.offers.is_empty() || !self.uploads.is_empty()
    }

    pub fn request(&mut self, r: Request) -> Vec<Out> {
        match r {
            Request::List(path) => vec![Out::Text(json!({"t":"fs.list","path":path}))],
            Request::Get(path) => vec![Out::Text(json!({"t":"fs.get","path":path}))],
            Request::SaveTo(dir) => {
                self.save_dir = dir;
                vec![]
            }
            Request::Send(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
                self.queue_upload(path, name)
            }
            Request::SendAs(path, name) => self.queue_upload(path, name),
            Request::Cancel(id) => self.cancel(&id),
            Request::Stop => {
                let ids: Vec<String> = self.offers.iter().map(|o| o.id.clone()).chain(self.download.iter().map(|d| d.t.id.clone())).chain(self.uploads.iter().map(|u| u.t.id.clone())).collect();
                ids.iter().flat_map(|id| self.cancel(id)).collect()
            }
        }
    }

    fn queue_upload(&mut self, path: PathBuf, name: String) -> Vec<Out> {
        let (key, id) = new_id();
        let opened = std::fs::metadata(&path).and_then(|m| {
            if m.is_dir() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "folder"));
            }
            Ok((File::open(&path)?, m.len()))
        });
        let (file, size) = match opened {
            Ok(v) => v,
            Err(e) => {
                let mut t = Transfer::new(&id, &name, true, 0);
                t.phase = Phase::Failed;
                t.error = if e.kind() == std::io::ErrorKind::InvalidInput { "Folders cannot be uploaded. Pick files.".into() } else { "That file could not be read.".into() };
                return vec![Self::note(&t)];
            }
        };
        let t = Transfer::new(&id, &name, true, size);
        let mut out = vec![Self::note(&t)];
        self.uploads.push_back(Upload { key, file, sent: 0, hash: Context::new(&SHA256), step: Step::Queued, t });
        out.extend(self.start_upload());
        out
    }

    /// Announces the next upload when none is in flight.
    fn start_upload(&mut self) -> Vec<Out> {
        let Some(u) = self.uploads.front_mut() else { return vec![] };
        if u.step != Step::Queued {
            return vec![];
        }
        u.step = Step::Waiting;
        u.t.phase = Phase::Active;
        vec![Out::Text(json!({"t":"file.begin","id":u.t.id,"name":u.t.name,"size":u.t.size})), Self::note(&u.t)]
    }

    fn cancel(&mut self, id: &str) -> Vec<Out> {
        let cancel = Out::Text(json!({"t":"file.cancel","id":id}));
        if let Some(i) = self.offers.iter().position(|o| o.id == id) {
            let o = self.offers.remove(i).unwrap();
            let mut t = Transfer::new(&o.id, &safe_rel(&o.name).to_string_lossy().replace('\\', "/"), false, o.size);
            t.phase = Phase::Failed;
            t.error = "Cancelled".into();
            return vec![Out::Text(json!({"t":"file.reject","id":id})), Self::note(&t)];
        }
        if self.download.as_ref().is_some_and(|d| d.t.id == id) {
            let d = self.download.take().unwrap();
            let mut t = d.t.clone();
            Self::drop_download(d);
            t.phase = Phase::Failed;
            t.error = "Cancelled".into();
            let mut out = vec![cancel, Self::note(&t)];
            out.extend(self.next_offer());
            return out;
        }
        if let Some(i) = self.uploads.iter().position(|u| u.t.id == id) {
            let mut u = self.uploads.remove(i).unwrap();
            let announced = u.step != Step::Queued;
            u.t.phase = Phase::Failed;
            u.t.error = "Cancelled".into();
            let mut out = vec![];
            if announced {
                out.push(cancel);
            }
            out.push(Self::note(&u.t));
            out.extend(self.start_upload());
            return out;
        }
        vec![]
    }
    fn drop_download(d: Download) {
        drop(d.file);
        let _ = std::fs::remove_file(&d.part);
    }

    /// The connection is gone: fail whatever was moving and remove partial files.
    pub fn abort(&mut self, why: &str) -> Vec<Out> {
        let mut out = vec![];
        if let Some(d) = self.download.take() {
            let mut t = d.t.clone();
            Self::drop_download(d);
            t.phase = Phase::Failed;
            t.error = why.into();
            out.push(Self::note(&t));
        }
        for o in self.offers.drain(..) {
            let mut t = Transfer::new(&o.id, &o.name, false, o.size);
            t.phase = Phase::Failed;
            t.error = why.into();
            out.push(Self::note(&t));
        }
        for mut u in self.uploads.drain(..) {
            u.t.phase = Phase::Failed;
            u.t.error = why.into();
            out.push(Self::note(&u.t));
        }
        out
    }

    pub fn on_text(&mut self, v: &Value) -> Vec<Out> {
        let id = v["id"].as_str().unwrap_or("");
        match v["t"].as_str().unwrap_or("") {
            "fs.list" => {
                let entries = v["entries"].as_array().map(|a| a.iter().filter_map(entry).collect()).unwrap_or_default();
                vec![Out::Note(Notice::Listed { path: v["path"].as_str().unwrap_or("").into(), entries })]
            }
            "error" => match v["of"].as_str() {
                Some(of @ ("fs.list" | "fs.get")) => vec![Out::Note(Notice::Problem(problem(v["code"].as_str().unwrap_or(""), of)))],
                _ => vec![],
            },
            "file.offer" => {
                let (Some(name), Some(size)) = (v["name"].as_str(), v["size"].as_u64()) else { return vec![] };
                if parse_uuid(id).is_none() || !self.seen.insert(id.to_string()) {
                    return vec![];
                }
                let shown = safe_rel(name).to_string_lossy().replace('\\', "/");
                let t = Transfer::new(id, &shown, false, size);
                self.offers.push_back(Offer { id: id.into(), name: name.into(), size });
                let mut out = vec![Self::note(&t)];
                out.extend(self.next_offer());
                out
            }
            "file.end" => self.download_end(id, v["sha256"].as_str().unwrap_or("")),
            "file.ready" => self.upload_ready(id, v["offset"].as_u64().unwrap_or(0)),
            "file.done" => {
                let Some(i) = self.uploads.iter().position(|u| u.t.id == id) else { return vec![] };
                let mut u = self.uploads.remove(i).unwrap();
                u.t.phase = Phase::Done;
                u.t.done = u.t.size;
                u.t.path = v["path"].as_str().unwrap_or("").into();
                let mut out = vec![Self::note(&u.t)];
                out.extend(self.start_upload());
                out
            }
            "file.error" => self.file_error(id, v["code"].as_str().unwrap_or("")),
            _ => vec![],
        }
    }

    fn file_error(&mut self, id: &str, code: &str) -> Vec<Out> {
        let why = reason(code);
        if self.download.as_ref().is_some_and(|d| d.t.id == id) {
            let d = self.download.take().unwrap();
            let mut t = d.t.clone();
            Self::drop_download(d);
            t.phase = Phase::Failed;
            t.error = why.into();
            let mut out = vec![Self::note(&t)];
            out.extend(self.next_offer());
            return out;
        }
        if let Some(i) = self.offers.iter().position(|o| o.id == id) {
            let o = self.offers.remove(i).unwrap();
            let mut t = Transfer::new(&o.id, &o.name, false, o.size);
            t.phase = Phase::Failed;
            t.error = why.into();
            return vec![Self::note(&t)];
        }
        if let Some(i) = self.uploads.iter().position(|u| u.t.id == id) {
            let mut u = self.uploads.remove(i).unwrap();
            u.t.phase = Phase::Failed;
            u.t.error = why.into();
            let mut out = vec![Self::note(&u.t)];
            out.extend(self.start_upload());
            return out;
        }
        vec![]
    }

    // ---- downloads ----

    /// Accepts the next queued offer when no download is running.
    fn next_offer(&mut self) -> Vec<Out> {
        let mut out = vec![];
        while self.download.is_none() {
            let Some(o) = self.offers.pop_front() else { break };
            let rel = safe_rel(&o.name);
            let dir = rel.parent().filter(|p| !p.as_os_str().is_empty()).map_or(self.save_dir.clone(), |p| self.save_dir.join(p));
            let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let key = parse_uuid(&o.id).unwrap_or_default();
            let part = dir.join(format!("{name}.{}.part", &hex(&key)[..12]));
            let shown = rel.to_string_lossy().replace('\\', "/");
            let mut t = Transfer::new(&o.id, &shown, false, o.size);
            let opened = std::fs::create_dir_all(&dir).and_then(|_| OpenOptions::new().create(true).truncate(true).write(true).open(&part));
            match opened {
                Ok(file) => {
                    t.phase = Phase::Active;
                    out.push(Out::Text(json!({"t":"file.accept","id":o.id,"offset":0})));
                    out.push(Self::note(&t));
                    self.download = Some(Download { key, dir, name, part, file, got: 0, hash: Context::new(&SHA256), end: None, t });
                }
                Err(_) => {
                    t.phase = Phase::Failed;
                    t.error = "The save folder could not be written to.".into();
                    out.push(Out::Text(json!({"t":"file.reject","id":o.id})));
                    out.push(Self::note(&t));
                }
            }
        }
        out
    }

    pub fn on_binary(&mut self, frame: &[u8]) -> Vec<Out> {
        let Some((key, offset, data)) = parse_chunk(frame) else { return vec![] };
        let Some(d) = self.download.as_mut().filter(|d| d.key == key) else { return vec![] };
        // A chunk that is not the next one is stale. The host sends them in order.
        if offset != d.got {
            return vec![];
        }
        if d.got + data.len() as u64 > d.t.size {
            let d = self.download.take().unwrap();
            let id = d.t.id.clone();
            let mut t = d.t.clone();
            Self::drop_download(d);
            t.phase = Phase::Failed;
            t.error = reason("too_large").into();
            let mut out = vec![Out::Text(json!({"t":"file.cancel","id":id})), Self::note(&t)];
            out.extend(self.next_offer());
            return out;
        }
        if let Err(e) = d.file.write_all(data) {
            let d = self.download.take().unwrap();
            let id = d.t.id.clone();
            let mut t = d.t.clone();
            Self::drop_download(d);
            t.phase = Phase::Failed;
            t.error = if e.kind() == std::io::ErrorKind::StorageFull { "This computer is out of disk space.".into() } else { "The file could not be saved.".into() };
            let mut out = vec![Out::Text(json!({"t":"file.cancel","id":id})), Self::note(&t)];
            out.extend(self.next_offer());
            return out;
        }
        d.hash.update(data);
        d.got += data.len() as u64;
        d.t.done = d.got;
        let mut out = vec![Self::note(&d.t)];
        if d.got == d.t.size {
            if let Some(sha) = d.end.take() {
                let id = d.t.id.clone();
                out.extend(self.download_end(&id, &sha));
            }
        }
        out
    }

    fn download_end(&mut self, id: &str, sha256: &str) -> Vec<Out> {
        let Some(d) = self.download.as_mut().filter(|d| d.t.id == id) else { return vec![] };
        if d.got < d.t.size {
            // The end marker overtook the last chunks. Wait for them.
            d.end = Some(sha256.to_string());
            return vec![];
        }
        let d = self.download.take().unwrap();
        let mut t = d.t.clone();
        let ok = hex(d.hash.clone().finish().as_ref()).eq_ignore_ascii_case(sha256) && d.got == d.t.size;
        let mut file = d.file;
        let flushed = file.flush().is_ok();
        drop(file);
        if ok && flushed {
            let dest = unique_path(&d.dir, &d.name);
            match std::fs::rename(&d.part, &dest) {
                Ok(()) => {
                    t.phase = Phase::Done;
                    t.done = t.size;
                    t.path = dest.to_string_lossy().into();
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&d.part);
                    t.phase = Phase::Failed;
                    t.error = "The file could not be saved.".into();
                }
            }
        } else {
            let _ = std::fs::remove_file(&d.part);
            t.phase = Phase::Failed;
            t.error = reason("bad_hash").into();
        }
        let mut out = vec![Self::note(&t)];
        out.extend(self.next_offer());
        out
    }

    // ---- uploads ----

    fn upload_ready(&mut self, id: &str, offset: u64) -> Vec<Out> {
        let Some(u) = self.uploads.front_mut().filter(|u| u.t.id == id && u.step == Step::Waiting) else { return vec![] };
        // The final hash covers the whole file, so fold in what the host already has.
        let mut left = offset.min(u.t.size);
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            match u.file.read(&mut buf[..CHUNK.min(left as usize)]) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    u.hash.update(&buf[..n]);
                    left -= n as u64;
                    u.sent += n as u64;
                }
            }
        }
        u.step = Step::Sending;
        u.t.done = u.sent;
        vec![Self::note(&u.t)]
    }

    /// The next piece of the running upload: a chunk frame, and the `file.end` after the last one.
    pub fn pump(&mut self) -> Vec<Out> {
        let Some(u) = self.uploads.front_mut().filter(|u| u.step == Step::Sending) else { return vec![] };
        let mut buf = vec![0u8; CHUNK.min((u.t.size - u.sent) as usize).max(1)];
        let n = if u.sent >= u.t.size { 0 } else { u.file.read(&mut buf).unwrap_or(0) };
        if n > 0 {
            u.hash.update(&buf[..n]);
            let frame = chunk_frame(&u.key, u.sent, &buf[..n]);
            u.sent += n as u64;
            u.t.done = u.sent;
            if u.sent < u.t.size {
                return vec![Out::Bin(frame), Self::note(&u.t)];
            }
            let sha = hex(u.hash.clone().finish().as_ref());
            u.step = Step::Finishing;
            return vec![Out::Bin(frame), Out::Text(json!({"t":"file.end","id":u.t.id,"sha256":sha})), Self::note(&u.t)];
        }
        if u.sent == u.t.size {
            let sha = hex(u.hash.clone().finish().as_ref());
            u.step = Step::Finishing;
            return vec![Out::Text(json!({"t":"file.end","id":u.t.id,"sha256":sha})), Self::note(&u.t)];
        }
        // The file shrank or could not be read while it was being sent.
        let mut u = self.uploads.pop_front().unwrap();
        u.t.phase = Phase::Failed;
        u.t.error = "The file changed while it was being sent.".into();
        let mut out = vec![Out::Text(json!({"t":"file.cancel","id":u.t.id})), Self::note(&u.t)];
        out.extend(self.start_upload());
        out
    }
}

/// Most files one folder upload queues.
pub const TREE_MAX: usize = 5000;

/// Turns dropped or picked paths into uploads: a file keeps its name, a folder becomes one upload
/// per file inside, named `folder/sub/file` so the layout survives. Symlinks are skipped.
pub fn expand(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => walk(path, &name, &mut out),
            Ok(m) if m.is_file() => out.push((path.clone(), name)),
            _ => {}
        }
        if out.len() >= TREE_MAX {
            out.truncate(TREE_MAX);
            break;
        }
    }
    out
}
fn walk(dir: &Path, prefix: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut items: Vec<_> = rd.flatten().collect();
    items.sort_by_key(|e| e.file_name());
    for e in items {
        if out.len() >= TREE_MAX {
            return;
        }
        let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path(), &name, out),
            Ok(t) if t.is_file() => out.push((e.path(), name)),
            _ => {}
        }
    }
}

/// Runs one connection until `Request::Stop`, the request channel closing or the socket dropping.
/// Everything the UI needs arrives as `Notice`s; `wake` is called after each batch.
pub async fn run(conn: Connected, save_dir: PathBuf, mut requests: mpsc::UnboundedReceiver<Request>, notices: mpsc::UnboundedSender<Notice>, wake: Arc<dyn Fn() + Send + Sync>) {
    let Connected { socket, caps, computer, .. } = conn;
    let log = crate::connlog::Conn::new(&computer.id);
    // The first folder listing is what the person waits for: time from asking to the reply.
    let mut first_asked: Option<std::time::Instant> = None;
    let mut listed_once = false;
    let (mut tx, mut rx) = socket.split();
    let have = |c: &str| caps.is_empty() || caps.iter().any(|x| x == c);
    let _ = notices.send(Notice::Ready { browse: have("fsbrowse"), upload: have("files") });
    wake();
    let mut engine = Engine::new(save_dir);
    let mut ping = tokio::time::interval(PING);
    ping.tick().await;
    let end = 'outer: loop {
        let mut outs: Vec<Out> = vec![];
        let mut stop = false;
        tokio::select! {
            biased;
            request = requests.recv() => match request {
                None | Some(Request::Stop) => { stop = true; outs = engine.request(Request::Stop); }
                Some(r) => {
                    if first_asked.is_none() && !listed_once && matches!(r, Request::List(_)) {
                        first_asked = Some(std::time::Instant::now());
                        listed_once = true;
                    }
                    outs = engine.request(r)
                }
            },
            message = rx.next() => match message {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<Value>(&t) {
                    Ok(v) => {
                        if let Some(asked) = first_asked.take_if(|_| matches!(v["t"].as_str(), Some("fs.list" | "error"))) {
                            log.step("first_reply", v["t"].as_str().unwrap_or(""), asked);
                        }
                        outs = engine.on_text(&v)
                    }
                    Err(_) => {}
                },
                Some(Ok(Message::Binary(b))) => outs = engine.on_binary(&b),
                Some(Ok(Message::Ping(p))) => { let _ = tx.send(Message::Pong(p)).await; }
                Some(Ok(_)) => {}
                _ => break 'outer "Connection closed".to_string(),
            },
            _ = ping.tick() => outs = vec![Out::Text(json!({"t":"ping"}))],
            _ = std::future::ready(()), if engine.sending() => outs = engine.pump(),
        }
        for out in outs {
            match out {
                Out::Text(v) => {
                    if tx.send(Message::text(v.to_string())).await.is_err() {
                        break 'outer "Connection closed".to_string();
                    }
                }
                Out::Bin(b) => {
                    if tx.send(Message::binary(b)).await.is_err() {
                        break 'outer "Connection closed".to_string();
                    }
                }
                Out::Note(n) => {
                    let _ = notices.send(n);
                }
            }
        }
        wake();
        if stop {
            let _ = tokio::time::timeout(Duration::from_secs(1), tx.close()).await;
            break "Disconnected".into();
        }
    };
    for out in engine.abort("The connection to the computer was lost.") {
        if let Out::Note(n) = out {
            let _ = notices.send(n);
        }
    }
    let _ = notices.send(Notice::Closed(end));
    wake();
}

#[cfg(test)]
mod tests;

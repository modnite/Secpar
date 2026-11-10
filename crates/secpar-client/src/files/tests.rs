use super::*;
use ring::digest::digest;

const ID: &str = "11111111-2222-4333-8444-555555555555";
const ID2: &str = "66666666-7777-4888-8999-aaaaaaaaaaaa";

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("secpar-client-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn sha(data: &[u8]) -> String {
    hex(ring::digest::digest(&SHA256, data).as_ref())
}
fn offer(e: &mut Engine, id: &str, name: &str, size: u64) -> Vec<Out> {
    e.on_text(&json!({"t":"file.offer","id":id,"name":name,"size":size}))
}
fn chunk(id: &str, offset: u64, data: &[u8]) -> Vec<u8> {
    chunk_frame(&parse_uuid(id).unwrap(), offset, data)
}
fn transfers(out: &[Out]) -> Vec<Transfer> {
    out.iter().filter_map(|o| if let Out::Note(Notice::Transfer(t)) = o { Some(t.clone()) } else { None }).collect()
}
fn texts(out: &[Out]) -> Vec<Value> {
    out.iter().filter_map(|o| if let Out::Text(v) = o { Some(v.clone()) } else { None }).collect()
}
fn last(out: &[Out]) -> Transfer {
    transfers(out).pop().expect("a transfer update")
}
fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}
/// Runs the whole upload and returns every frame and message it produced.
fn drain(e: &mut Engine) -> Vec<Out> {
    let mut out = vec![];
    while e.sending() {
        out.extend(e.pump());
    }
    out
}

#[test]
fn names_cannot_leave_the_save_folder() {
    assert_eq!(safe_rel("../../etc/passwd"), PathBuf::from("etc/passwd"));
    assert_eq!(safe_rel("C:\\Windows\\win.ini"), PathBuf::from("C_/Windows/win.ini"));
    assert_eq!(safe_rel("a/./b//c.txt"), PathBuf::from("a/b/c.txt"));
    assert_eq!(safe_rel(""), PathBuf::from("file"));
    assert_eq!(safe_rel(".."), PathBuf::from("file"));
    assert_eq!(sanitize("nul.txt"), "_nul.txt");
    assert_eq!(sanitize("a:b?.txt"), "a_b_.txt");
    assert_eq!(sanitize("trailing. "), "trailing");
}
#[test]
fn uuid_and_chunk_framing_round_trip() {
    let key = parse_uuid(ID).unwrap();
    assert_eq!(format_uuid(&key), ID);
    assert_eq!(parse_uuid("nope"), None);
    let f = chunk_frame(&key, 0x0102030405, b"abc");
    assert_eq!(f[0], 1);
    assert_eq!(parse_chunk(&f), Some((key, 0x0102030405, &b"abc"[..])));
    assert_eq!(parse_chunk(&f[..24]), None);
    assert_eq!(parse_chunk(&[2, 0xff, 0xd8]), None);
}
#[test]
fn a_listing_becomes_entries_and_errors_become_plain_words() {
    let mut e = Engine::new(tmp("list"));
    let out = e.on_text(&json!({"t":"fs.list","path":"C:\\","entries":[
        {"name":"Users","path":"C:\\Users","dir":true,"size":0,"mtime":5},
        {"name":"a.txt","path":"C:\\a.txt","dir":false,"size":12,"mtime":7},
        {"name":"broken"}]}));
    let [Out::Note(Notice::Listed { path, entries })] = &out[..] else { panic!("one listing") };
    assert_eq!(path, "C:\\");
    assert_eq!(entries.len(), 2);
    assert!(entries[0].dir && entries[1].size == 12);
    let off = e.on_text(&json!({"t":"error","code":"unsupported","msg":"file transfer is off on this PC","of":"fs.list"}));
    assert_eq!(off, vec![Out::Note(Notice::Problem("File transfer is turned off on this computer.".into()))]);
    let denied = e.on_text(&json!({"t":"error","code":"denied","msg":"os error 5","of":"fs.list"}));
    assert_eq!(denied, vec![Out::Note(Notice::Problem("That folder could not be opened.".into()))]);
    assert!(e.on_text(&json!({"t":"error","code":"denied","msg":"x","of":"mouse.move"})).is_empty());
}
#[test]
fn requests_become_the_wire_messages() {
    let mut e = Engine::new(tmp("req"));
    assert_eq!(e.request(Request::List("/home".into())), vec![Out::Text(json!({"t":"fs.list","path":"/home"}))]);
    assert_eq!(e.request(Request::Get("/home/a".into())), vec![Out::Text(json!({"t":"fs.get","path":"/home/a","batch":0}))]);
}

#[test]
fn a_download_is_accepted_written_checked_and_only_then_named() {
    let dir = tmp("dl");
    let mut e = Engine::new(dir.clone());
    let body = data(600_000);
    let out = offer(&mut e, ID, "report.bin", body.len() as u64);
    assert_eq!(texts(&out), vec![json!({"t":"file.accept","id":ID,"offset":0})]);
    assert_eq!(last(&out).phase, Phase::Active);
    let mut at = 0;
    for part in body.chunks(CHUNK) {
        let t = last(&e.on_binary(&chunk(ID, at as u64, part)));
        at += part.len();
        assert_eq!(t.done, at as u64);
    }
    // Not final until the host's file.end checks out.
    assert!(!dir.join("report.bin").exists());
    assert!(std::fs::read_dir(&dir).unwrap().flatten().any(|f| f.file_name().to_string_lossy().ends_with(".part")));
    let t = last(&e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(&body)})));
    assert_eq!(t.phase, Phase::Done);
    assert_eq!(std::fs::read(dir.join("report.bin")).unwrap(), body);
    assert_eq!(t.path, dir.join("report.bin").to_string_lossy());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no partial file is left");
    assert!(!e.busy());
}
#[test]
fn a_wrong_hash_fails_the_download_and_leaves_nothing() {
    let dir = tmp("badhash");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "x.bin", 4);
    e.on_binary(&chunk(ID, 0, b"abcd"));
    let t = last(&e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"abce")})));
    assert_eq!(t.phase, Phase::Failed);
    assert!(t.error.contains("damaged"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
#[test]
fn the_end_marker_may_arrive_before_the_last_chunk() {
    let dir = tmp("early-end");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "x.bin", 6);
    e.on_binary(&chunk(ID, 0, b"abc"));
    assert!(e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"abcdef")})).is_empty());
    let t = last(&e.on_binary(&chunk(ID, 3, b"def")));
    assert_eq!(t.phase, Phase::Done);
    assert_eq!(std::fs::read(dir.join("x.bin")).unwrap(), b"abcdef");
}
#[test]
fn an_empty_file_downloads() {
    let dir = tmp("empty");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "empty.txt", 0);
    let t = last(&e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"")})));
    assert_eq!(t.phase, Phase::Done);
    assert_eq!(std::fs::read(dir.join("empty.txt")).unwrap(), b"");
}
#[test]
fn stale_unknown_and_oversized_chunks_are_handled() {
    let dir = tmp("chunks");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "x.bin", 4);
    assert!(e.on_binary(&chunk(ID, 2, b"zz")).is_empty(), "out of order");
    assert!(e.on_binary(&chunk(ID2, 0, b"zz")).is_empty(), "another transfer");
    assert!(e.on_binary(&[9, 9]).is_empty(), "not a chunk");
    let out = e.on_binary(&chunk(ID, 0, b"too long"));
    assert_eq!(texts(&out), vec![json!({"t":"file.cancel","id":ID})]);
    assert_eq!(last(&out).phase, Phase::Failed);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
#[test]
fn offers_are_taken_one_at_a_time_in_order_and_keep_their_folders() {
    let dir = tmp("queue");
    let mut e = Engine::new(dir.clone());
    let first = offer(&mut e, ID, "Trip/a.txt", 1);
    assert_eq!(texts(&first).len(), 1);
    let second = offer(&mut e, ID2, "Trip/sub/../b.txt", 1);
    assert!(texts(&second).is_empty(), "the second waits");
    assert_eq!(last(&second).phase, Phase::Queued);
    // The same offer twice is one offer.
    assert!(offer(&mut e, ID2, "Trip/b.txt", 1).is_empty());
    e.on_binary(&chunk(ID, 0, b"a"));
    let out = e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"a")}));
    assert_eq!(texts(&out), vec![json!({"t":"file.accept","id":ID2,"offset":0})]);
    e.on_binary(&chunk(ID2, 0, b"b"));
    e.on_text(&json!({"t":"file.end","id":ID2,"sha256":sha(b"b")}));
    assert_eq!(std::fs::read(dir.join("Trip/a.txt")).unwrap(), b"a");
    assert_eq!(std::fs::read(dir.join("Trip/sub/b.txt")).unwrap(), b"b");
}
#[test]
fn a_name_that_exists_gets_a_number() {
    let dir = tmp("taken");
    std::fs::write(dir.join("a.txt"), b"old").unwrap();
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "a.txt", 3);
    e.on_binary(&chunk(ID, 0, b"new"));
    e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"new")}));
    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"old");
    assert_eq!(std::fs::read(dir.join("a (1).txt")).unwrap(), b"new");
}
#[test]
fn a_hostile_offer_name_stays_inside_the_save_folder() {
    let dir = tmp("hostile");
    let inner = dir.join("inner");
    let mut e = Engine::new(inner.clone());
    offer(&mut e, ID, "../../escape.txt", 1);
    e.on_binary(&chunk(ID, 0, b"x"));
    e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"x")}));
    assert!(inner.join("escape.txt").exists());
    assert!(!dir.join("escape.txt").exists());
}
#[test]
fn a_host_error_drops_the_partial_file_and_starts_the_next_offer() {
    let dir = tmp("hosterr");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "a.bin", 10);
    offer(&mut e, ID2, "b.bin", 1);
    e.on_binary(&chunk(ID, 0, b"12345"));
    let out = e.on_text(&json!({"t":"file.error","id":ID,"code":"denied"}));
    assert_eq!(transfers(&out)[0].phase, Phase::Failed);
    assert_eq!(texts(&out), vec![json!({"t":"file.accept","id":ID2,"offset":0})]);
    assert!(!std::fs::read_dir(&dir).unwrap().flatten().any(|f| f.file_name().to_string_lossy().starts_with("a.bin")));
}
#[test]
fn offers_with_a_bad_id_are_ignored() {
    let mut e = Engine::new(tmp("badid"));
    assert!(offer(&mut e, "not-a-uuid", "a.txt", 1).is_empty());
    assert!(e.on_text(&json!({"t":"file.offer","id":ID})).is_empty());
    assert!(!e.busy());
}
#[test]
fn cancelling_a_download_tells_the_host_and_removes_the_partial_file() {
    let dir = tmp("cancel-dl");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "a.bin", 10);
    offer(&mut e, ID2, "b.bin", 1);
    e.on_binary(&chunk(ID, 0, b"123"));
    let out = e.request(Request::Cancel(ID.into()));
    assert!(texts(&out).contains(&json!({"t":"file.cancel","id":ID})));
    assert!(texts(&out).contains(&json!({"t":"file.accept","id":ID2,"offset":0})), "the queue moves on");
    // A queued offer is rejected instead.
    offer(&mut e, "99999999-7777-4888-8999-aaaaaaaaaaaa", "c.bin", 1);
    let out = e.request(Request::Cancel("99999999-7777-4888-8999-aaaaaaaaaaaa".into()));
    assert_eq!(texts(&out), vec![json!({"t":"file.reject","id":"99999999-7777-4888-8999-aaaaaaaaaaaa"})]);
    e.request(Request::Stop);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
#[test]
fn losing_the_connection_fails_what_was_moving() {
    let dir = tmp("abort");
    let mut e = Engine::new(dir.clone());
    offer(&mut e, ID, "a.bin", 10);
    offer(&mut e, ID2, "b.bin", 1);
    e.on_binary(&chunk(ID, 0, b"123"));
    let out = e.abort("gone");
    let t = transfers(&out);
    assert_eq!(t.len(), 2);
    assert!(t.iter().all(|t| t.phase == Phase::Failed && t.error == "gone"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    assert!(!e.busy());
}

#[test]
fn an_upload_is_announced_chunked_hashed_and_finished() {
    let dir = tmp("up");
    let body = data(800_000);
    let file = dir.join("photo.jpg");
    std::fs::write(&file, &body).unwrap();
    let mut e = Engine::new(dir.clone());
    let out = e.request(Request::Send(file));
    let begin = texts(&out);
    assert_eq!(begin.len(), 1);
    assert_eq!(begin[0]["t"], "file.begin");
    assert_eq!(begin[0]["name"], "photo.jpg");
    assert_eq!(begin[0]["size"], 800_000);
    let id = begin[0]["id"].as_str().unwrap().to_string();
    assert_eq!(parse_uuid(&id).map(|k| format_uuid(&k)), Some(id.clone()));
    assert!(!e.sending(), "nothing is sent before file.ready");
    assert!(e.pump().is_empty());
    e.on_text(&json!({"t":"file.ready","id":id,"offset":0}));
    assert!(e.sending());
    let sent = drain(&mut e);
    let mut got = vec![];
    let mut end = None;
    for o in &sent {
        match o {
            Out::Bin(f) => {
                let (key, offset, data) = parse_chunk(f).unwrap();
                assert_eq!(format_uuid(&key), id);
                assert_eq!(offset as usize, got.len());
                assert!(data.len() <= CHUNK);
                got.extend_from_slice(data);
            }
            Out::Text(v) => end = Some(v.clone()),
            _ => {}
        }
    }
    assert_eq!(got, body);
    let end = end.expect("file.end");
    assert_eq!(end, json!({"t":"file.end","id":id,"sha256":sha(&body)}));
    assert!(matches!(sent.iter().rposition(|o| matches!(o, Out::Text(_))), Some(i) if i > sent.iter().rposition(|o| matches!(o, Out::Bin(_))).unwrap()), "file.end follows the last chunk");
    assert!(!e.sending());
    let t = last(&e.on_text(&json!({"t":"file.done","id":id,"path":"/home/me/Secpar/photo.jpg"})));
    assert_eq!((t.phase, t.done, t.path.as_str()), (Phase::Done, 800_000, "/home/me/Secpar/photo.jpg"));
    assert!(!e.busy());
}
#[test]
fn an_empty_upload_sends_only_the_end_marker() {
    let dir = tmp("up-empty");
    let file = dir.join("empty");
    std::fs::write(&file, b"").unwrap();
    let mut e = Engine::new(dir);
    let id = texts(&e.request(Request::Send(file)))[0]["id"].as_str().unwrap().to_string();
    e.on_text(&json!({"t":"file.ready","id":id,"offset":0}));
    let sent = drain(&mut e);
    assert!(sent.iter().all(|o| !matches!(o, Out::Bin(_))));
    assert_eq!(texts(&sent), vec![json!({"t":"file.end","id":id,"sha256":sha(b"")})]);
}
#[test]
fn uploads_go_one_at_a_time() {
    let dir = tmp("up-queue");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, b"aaa").unwrap();
    std::fs::write(&b, b"bbb").unwrap();
    let mut e = Engine::new(dir);
    let id_a = texts(&e.request(Request::Send(a)))[0]["id"].as_str().unwrap().to_string();
    let second = e.request(Request::Send(b));
    assert!(texts(&second).is_empty(), "b waits for a");
    assert_eq!(last(&second).phase, Phase::Queued);
    e.on_text(&json!({"t":"file.ready","id":id_a,"offset":0}));
    drain(&mut e);
    let next = e.on_text(&json!({"t":"file.done","id":id_a,"path":"p"}));
    let begin = texts(&next);
    assert_eq!(begin[0]["t"], "file.begin");
    assert_eq!(begin[0]["name"], "b");
}
#[test]
fn a_refused_upload_fails_with_a_plain_message_and_the_next_one_goes_on() {
    let dir = tmp("up-refused");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, b"aaa").unwrap();
    std::fs::write(&b, b"bbb").unwrap();
    let mut e = Engine::new(dir);
    let id_a = texts(&e.request(Request::Send(a)))[0]["id"].as_str().unwrap().to_string();
    e.request(Request::Send(b));
    let out = e.on_text(&json!({"t":"file.error","id":id_a,"code":"denied"}));
    let t = transfers(&out);
    assert_eq!(t[0].phase, Phase::Failed);
    assert!(t[0].error.contains("turned off"));
    assert_eq!(texts(&out)[0]["t"], "file.begin");
}
#[test]
fn an_upload_resumes_from_the_offset_the_host_reports() {
    let dir = tmp("up-resume");
    let body = data(300_000);
    let file = dir.join("big");
    std::fs::write(&file, &body).unwrap();
    let mut e = Engine::new(dir);
    let id = texts(&e.request(Request::Send(file)))[0]["id"].as_str().unwrap().to_string();
    e.on_text(&json!({"t":"file.ready","id":id,"offset":100_000}));
    let sent = drain(&mut e);
    let first = sent.iter().find_map(|o| if let Out::Bin(f) = o { parse_chunk(f).map(|c| c.1) } else { None });
    assert_eq!(first, Some(100_000));
    assert_eq!(texts(&sent)[0]["sha256"], sha(&body), "the hash still covers the whole file");
}
#[test]
fn cancelling_an_upload_tells_the_host_only_when_it_knows_about_it() {
    let dir = tmp("up-cancel");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::write(&a, b"aaa").unwrap();
    std::fs::write(&b, b"bbb").unwrap();
    let mut e = Engine::new(dir);
    let id_a = texts(&e.request(Request::Send(a)))[0]["id"].as_str().unwrap().to_string();
    let id_b = transfers(&e.request(Request::Send(b)))[0].id.clone();
    let out = e.request(Request::Cancel(id_b));
    assert!(texts(&out).is_empty(), "b was never announced");
    let out = e.request(Request::Cancel(id_a.clone()));
    assert_eq!(texts(&out), vec![json!({"t":"file.cancel","id":id_a})]);
    assert!(!e.busy());
}
#[test]
fn a_missing_file_or_a_folder_cannot_be_uploaded() {
    let dir = tmp("up-bad");
    let mut e = Engine::new(dir.clone());
    let t = last(&e.request(Request::Send(dir.join("missing"))));
    assert_eq!(t.phase, Phase::Failed);
    let t = last(&e.request(Request::Send(dir.clone())));
    assert!(t.error.contains("Folders"));
    assert!(!e.busy());
}
#[test]
fn a_file_that_shrinks_mid_upload_is_cancelled() {
    let dir = tmp("up-shrink");
    let file = dir.join("f");
    std::fs::write(&file, data(CHUNK * 2)).unwrap();
    let mut e = Engine::new(dir);
    let id = texts(&e.request(Request::Send(file.clone())))[0]["id"].as_str().unwrap().to_string();
    e.on_text(&json!({"t":"file.ready","id":id,"offset":0}));
    e.pump();
    std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_len(10).unwrap();
    let mut out = vec![];
    while e.sending() {
        out.extend(e.pump());
    }
    assert!(texts(&out).contains(&json!({"t":"file.cancel","id":id})));
    assert_eq!(last(&out).phase, Phase::Failed);
}
#[test]
fn unrelated_messages_do_nothing() {
    let mut e = Engine::new(tmp("noise"));
    assert!(e.on_text(&json!({"t":"pong"})).is_empty());
    assert!(e.on_text(&json!({"t":"file.done","id":ID,"path":"p"})).is_empty());
    assert!(e.on_text(&json!({"t":"file.ready","id":ID,"offset":0})).is_empty());
    assert!(e.on_text(&json!({"t":"file.error","id":ID,"code":"denied"})).is_empty());
    assert!(e.on_text(&json!({})).is_empty());
}

#[test]
fn folders_expand_to_files_that_keep_their_layout() {
    let dir = tmp("expand");
    std::fs::create_dir_all(dir.join("Trip/sub")).unwrap();
    std::fs::write(dir.join("Trip/a.txt"), b"a").unwrap();
    std::fs::write(dir.join("Trip/sub/b.txt"), b"b").unwrap();
    std::fs::write(dir.join("loose.txt"), b"c").unwrap();
    let got = expand(&[dir.join("Trip"), dir.join("loose.txt"), dir.join("missing")]);
    let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Trip/a.txt", "Trip/sub/b.txt", "loose.txt"]);
    assert_eq!(got[1].0, dir.join("Trip/sub/b.txt"));
}
#[test]
fn an_upload_can_carry_a_relative_name() {
    let dir = tmp("up-as");
    let file = dir.join("b.txt");
    std::fs::write(&file, b"b").unwrap();
    let mut e = Engine::new(dir);
    let out = texts(&e.request(Request::SendAs(file, "Trip/sub/b.txt".into())));
    assert_eq!(out[0]["name"], "Trip/sub/b.txt");
}

/// The whole `run` loop against a scripted host over real TLS and WebSocket: browse, upload,
/// download and a clean stop. The host stand-in only knows the wire format.
#[tokio::test]
async fn run_talks_to_a_scripted_host_over_tls() {
    use crate::control::Computer;
    use futures_util::{SinkExt, StreamExt};
    use tokio_rustls::{rustls::{self, pki_types::PrivatePkcs8KeyDer}, TlsAcceptor};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let fingerprint: [u8; 32] = digest(&SHA256, cert.der()).as_ref().try_into().unwrap();
    let config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![cert.der().clone()], PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into()).unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let served: Vec<u8> = data(CHUNK + 5000);
    let served_copy = served.clone();
    let received = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let received_copy = received.clone();
    let host = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(acceptor.accept(tcp).await.unwrap()).await.unwrap();
        let mut up_id = String::new();
        while let Some(Ok(m)) = ws.next().await {
            let send = |v: Value| Message::text(v.to_string());
            match m {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    match v["t"].as_str().unwrap() {
                        "hello" => ws.send(send(json!({"t":"welcome","v":1,"pc":{"id":"pc1","name":"Test"},"caps":["files","fsbrowse"],"locked":false}))).await.unwrap(),
                        "fs.list" => ws.send(send(json!({"t":"fs.list","path":v["path"],"entries":[{"name":"down.bin","path":"/h/down.bin","dir":false,"size":served_copy.len(),"mtime":1}]}))).await.unwrap(),
                        "fs.get" => ws.send(send(json!({"t":"file.offer","id":ID2,"name":"down.bin","size":served_copy.len()}))).await.unwrap(),
                        "file.accept" => {
                            let key = parse_uuid(ID2).unwrap();
                            for (i, part) in served_copy.chunks(CHUNK).enumerate() {
                                ws.send(Message::binary(chunk_frame(&key, (i * CHUNK) as u64, part))).await.unwrap();
                            }
                            ws.send(send(json!({"t":"file.end","id":ID2,"sha256":sha(&served_copy)}))).await.unwrap();
                        }
                        "file.begin" => {
                            up_id = v["id"].as_str().unwrap().to_string();
                            ws.send(send(json!({"t":"file.ready","id":up_id,"offset":0}))).await.unwrap();
                        }
                        "file.end" => {
                            let ok = sha(&received_copy.lock().unwrap()) == v["sha256"].as_str().unwrap();
                            let reply = if ok { json!({"t":"file.done","id":up_id,"path":"/r/up.bin"}) } else { json!({"t":"file.error","id":up_id,"code":"bad_hash"}) };
                            ws.send(send(reply)).await.unwrap();
                        }
                        _ => {}
                    }
                }
                Message::Binary(b) => {
                    let (_, offset, data) = parse_chunk(&b).unwrap();
                    let mut r = received_copy.lock().unwrap();
                    assert_eq!(offset as usize, r.len(), "chunks arrive in order");
                    r.extend_from_slice(data);
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });
    let computer = Computer { id: "pc1".into(), name: "Test".into(), hosts: vec!["127.0.0.1".into()], port, fingerprint, token: "t".into(), relay: None };
    let conn = crate::control::Connected::reconnect(computer, "Desktop").await.unwrap();
    let dir = tmp("run");
    let upload = dir.join("up.bin");
    let body = data(CHUNK * 2 + 123);
    std::fs::write(&upload, &body).unwrap();
    let (req_tx, req_rx) = mpsc::unbounded_channel();
    let (note_tx, mut notes) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(conn, dir.join("save"), req_rx, note_tx, Arc::new(|| {})));
    assert_eq!(notes.recv().await, Some(Notice::Ready { browse: true, upload: true }));
    req_tx.send(Request::List("/h".into())).unwrap();
    let Some(Notice::Listed { entries, .. }) = notes.recv().await else { panic!("a listing") };
    assert_eq!(entries[0].name, "down.bin");
    req_tx.send(Request::Send(upload)).unwrap();
    req_tx.send(Request::Get("/h/down.bin".into())).unwrap();
    let (mut up, mut down) = (None, None);
    while up.is_none() || down.is_none() {
        let Some(Notice::Transfer(t)) = tokio::time::timeout(Duration::from_secs(20), notes.recv()).await.unwrap().filter(|n| matches!(n, Notice::Transfer(_))) else { continue };
        match (t.upload, t.phase) {
            (true, Phase::Done) => up = Some(t),
            (false, Phase::Done) => down = Some(t),
            (_, Phase::Failed) => panic!("transfer failed: {}", t.error),
            _ => {}
        }
    }
    assert_eq!(up.unwrap().path, "/r/up.bin");
    assert_eq!(*received.lock().unwrap(), body, "the host got exactly the uploaded bytes");
    assert_eq!(std::fs::read(down.unwrap().path).unwrap(), served, "the download matches what the host served");
    req_tx.send(Request::Stop).unwrap();
    task.await.unwrap();
    host.abort();
}

#[test]
fn cancel_all_drains_downloads_without_accepting_children_or_stopping_uploads() {
    let dir = tmp("cancel-all");
    let source = dir.join("upload.txt");
    std::fs::write(&source, b"upload").unwrap();
    let mut e = Engine::new(dir.clone());
    e.request(Request::Get("/folder".into()));
    offer(&mut e, ID, "folder/first.bin", 10);
    offer(&mut e, ID2, "folder/second.bin", 10);
    let upload = e.request(Request::Send(source));
    let upload_id = last(&upload).id;
    let part = e.download.as_ref().unwrap().part.clone();
    e.on_binary(&chunk(ID, 0, b"part"));
    let out = e.request(Request::CancelDownloads);
    let messages = texts(&out);
    assert!(messages.contains(&json!({"t":"file.cancel","id":ID})));
    assert!(messages.contains(&json!({"t":"file.reject","id":ID2})));
    assert!(!messages.iter().any(|v| v["t"] == "file.accept"));
    assert_eq!(transfers(&out).len(), 2);
    assert!(transfers(&out).iter().all(|t| t.error == "Cancelled"));
    assert!(!part.exists());
    assert!(e.download.is_none() && e.offers.is_empty());
    assert_eq!(e.uploads.front().unwrap().t.id, upload_id);
    assert!(e.on_binary(&chunk(ID, 4, b"late")).is_empty());
    assert!(e.on_text(&json!({"t":"file.end","id":ID,"sha256":sha(b"partlate")})).is_empty());
}

#[test]
fn cancelled_pending_folder_offers_stay_cancelled_after_a_later_download_starts() {
    let mut e = Engine::new(tmp("late-folder"));
    e.request(Request::Get("/accidental-folder".into()));
    e.request(Request::CancelDownloads);
    let next = e.request(Request::Get("/wanted.txt".into()));
    assert_eq!(texts(&next)[0]["batch"], 1);
    let stale = e.on_text(&json!({"t":"file.offer","id":ID,"name":"folder/late.txt","size":3,"batch":0}));
    assert_eq!(texts(&stale), vec![json!({"t":"file.reject","id":ID})]);
    assert!(e.download.is_none());
    let wanted = e.on_text(&json!({"t":"file.offer","id":ID2,"name":"wanted.txt","size":3,"batch":1}));
    assert!(texts(&wanted).contains(&json!({"t":"file.accept","id":ID2,"offset":0})));
    e.on_binary(&chunk(ID2, 0, b"yes"));
    let done = e.on_text(&json!({"t":"file.end","id":ID2,"sha256":sha(b"yes")}));
    assert_eq!(last(&done).phase, Phase::Done);
    // An intentional Send to Secpar offer is independent of requested folder batches.
    assert!(texts(&offer(&mut e, "bbbbbbbb-2222-4333-8444-555555555555", "sent.txt", 0)).iter().any(|v| v["t"] == "file.accept"));
}

#[test]
fn legacy_host_late_offers_are_rejected_until_reconnecting() {
    let mut e = Engine::new(tmp("legacy-cancel"));
    e.batch_supported = false;
    e.request(Request::Get("/folder".into()));
    e.request(Request::CancelDownloads);
    assert_eq!(texts(&offer(&mut e, ID, "late", 1)), vec![json!({"t":"file.reject","id":ID})]);
    assert!(matches!(&e.request(Request::Get("/new".into()))[..], [Out::Note(Notice::Problem(message))] if message.starts_with("Reconnect")));
}

use secpar_clipboard::{Bridge,Event};
use std::{process::Command,time::{Duration,Instant}};
fn wait(mut check:impl FnMut()->bool){let end=Instant::now()+Duration::from_secs(4);while !check(){assert!(Instant::now()<end,"Wayland clipboard did not settle");std::thread::sleep(Duration::from_millis(20));}}
#[test]
#[ignore="Requires isolated synthetic Wayland compositor; never use a personal session"]
fn actual_wayland_copy_paste_and_remote_echo_suppression() {
    assert_eq!(std::env::var("SECPAR_CLIPBOARD_REVIEW").as_deref(),Ok("1"));
    assert!(Command::new("wl-copy").args(["--","Initial fixture"]).status().unwrap().success());
    let bridge=Bridge::start();
    std::thread::sleep(Duration::from_millis(250));
    assert!(bridge.poll().is_none(),"Connection must not export previous clipboard content");
    assert!(Command::new("wl-copy").args(["--","Local fixture: café ✓"]).status().unwrap().success());
    wait(||match bridge.poll(){Some(Event::Text(t))=>t=="Local fixture: café ✓",Some(Event::Unavailable)=>panic!("Native Wayland clipboard unavailable"),None=>false});
    assert!(bridge.set("Remote fixture: λ\nsecond line".into()));
    wait(||Command::new("wl-paste").arg("--no-newline").output().ok().is_some_and(|o|o.status.success() && o.stdout=="Remote fixture: λ\nsecond line".as_bytes()));
    std::thread::sleep(Duration::from_millis(200));
    assert!(bridge.poll().is_none(),"Remote content must not echo back");
}

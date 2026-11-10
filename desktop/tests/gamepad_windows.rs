//! Windows end to end: a ViGEm virtual pad stands in for a physical controller, the client's
//! XInput capture reads it, the messages go through the wire format into the host's pad code, and
//! a second virtual pad appears whose state XInput reads back. Needs ViGEmBus; skipped without it.
#![cfg(windows)]

use pc_remote_host::pad::{Bus, PadState, Pads, ViGEm};
use pc_remote_host::proto::ClientMsg;
use secpar_desktop::gamepad::{logic::*, xinput::XInput, Source};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    None
}

fn near(a: i16, b: i16) -> bool {
    (a as i32 - b as i32).abs() < 1500
}

#[test]
fn physical_pad_to_host_pad_through_the_wire_format() {
    let Some(bus) = ViGEm::connect() else {
        eprintln!("ViGEmBus not installed: skipped");
        return;
    };
    let Some(mut xinput) = XInput::load() else { panic!("xinput1_4.dll must load on Windows") };
    if xinput.poll().len() > 1 {
        eprintln!("other controllers are attached: skipped");
        return;
    }
    let bus: Arc<dyn Bus> = Arc::new(bus);

    // the stand in for the user's physical controller
    let (rumble_tx, rumble_rx) = std::sync::mpsc::channel();
    let mut physical = bus.plug(0, Box::new(move |lo, hi| { let _ = rumble_tx.send((lo, hi)); })).expect("plug physical");
    let sent = PadState { btn: (A | LB | DPAD_LEFT) as u16, lt: 200, rt: 0, lx: 20000, ly: -15000, rx: 0, ry: 30000 };
    assert!(physical.update(&sent));

    let mut hub = Hub::default();
    let mut host_pads = Pads::new(bus.clone(), Box::new(|_, _, _| {}));
    // XInput user index of the stand in (the only controller present until the host pad appears)
    let physical_key = wait(|| xinput.poll().first().map(|(k, _)| *k)).expect("XInput sees the physical pad");

    // one capture step: what the client would send, parsed the way the host parses it
    let mut step = |xinput: &mut XInput, present: bool, host_pads: &mut Pads| {
        let devices: Vec<_> = xinput.poll().into_iter().filter(|(k, _)| present && *k == physical_key).collect();
        let mut last = None;
        for out in hub.update(&devices, Instant::now()) {
            let msg: ClientMsg = serde_json::from_value(out.json()).expect("host parses the client message");
            match msg {
                ClientMsg::Pad { id, btn, lt, rt, lx, ly, rx, ry } => {
                    host_pads.pad(id, &PadState { btn, lt, rt, lx, ly, rx, ry }).expect("host pad");
                    last = Some((id, btn, lt, rt, lx, ly, rx, ry));
                }
                ClientMsg::PadOff { id } => host_pads.off(id),
                other => panic!("unexpected {other:?}"),
            }
        }
        last
    };

    let (id, btn, lt, rt, lx, ly, rx, ry) = wait(|| step(&mut xinput, true, &mut host_pads)).expect("capture saw the pad");
    assert_eq!(id, 0);
    assert_eq!((btn, lt, rt), (sent.btn, 200, 0));
    assert!(near(lx, 20000) && near(ly, -15000) && near(rx, 0) && near(ry, 30000), "{lx} {ly} {rx} {ry}");

    // the host's virtual pad is a second XInput device with the same state
    let both = wait(|| {
        step(&mut xinput, true, &mut host_pads);
        let d = xinput.poll();
        (d.len() == 2 && d.iter().all(|(_, s)| s.btn == sent.btn)).then_some(d)
    })
    .expect("host virtual pad appears next to the physical one");
    for (_, s) in &both {
        assert_eq!((s.btn, s.lt, s.rt), (sent.btn, 200, 0));
        assert!(near(s.lx, 20000) && near(s.ly, -15000) && near(s.ry, 30000));
    }

    // a button release reaches the host pad
    let mut released = sent;
    released.btn = 0;
    assert!(physical.update(&released));
    let after = wait(|| {
        step(&mut xinput, true, &mut host_pads);
        let d = xinput.poll();
        (d.len() == 2 && d.iter().all(|(_, s)| s.btn == 0)).then_some(())
    });
    assert!(after.is_some(), "the release reached the host pad");

    // rumble from the host reaches the controller: XInputSetState on the stand in comes back as its
    // vibration notification
    xinput.rumble(physical_key, 200, 100);
    let got = wait(|| rumble_rx.try_recv().ok().filter(|r| *r != (0, 0)));
    assert_eq!(got, Some((200, 100)), "rumble reached the controller");
    xinput.silence();

    // the physical pad is unplugged: pad.off removes the host pad too
    drop(physical);
    let gone = wait(|| {
        step(&mut xinput, false, &mut host_pads);
        xinput.poll().is_empty().then_some(())
    });
    assert!(gone.is_some(), "no controllers remain after unplug and pad.off");
}

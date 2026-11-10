//! Native keyboard events and compositor shortcut inhibition for our own Wayland surface.
//! No global keyboard grab. Inhibition exists only while a focused stream accepts keys.
use crate::keys::{Escape, EscapeKeys, Filter};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::collections::BTreeSet;
use wayland_client::{backend::{Backend,ObjectId}, protocol::{wl_registry,wl_seat,wl_keyboard,wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1 as manager,
    zwp_keyboard_shortcuts_inhibitor_v1 as inhibitor,
};

pub enum Output { Key(u16,bool), Escape(Escape) }
#[derive(Default)]
struct State {
    surface:u32,
    manager:Option<manager::ZwpKeyboardShortcutsInhibitManagerV1>,
    seat:Option<wl_seat::WlSeat>,
    keyboard:Option<wl_keyboard::WlKeyboard>,
    focused:bool, active:bool, wanted:bool,
    inhibitor_id:Option<ObjectId>,
    down:BTreeSet<u16>, filter:Option<Filter>, escape:EscapeKeys,
    out:Vec<Output>,
}
impl State {
    fn release(&mut self) {
        if let Some(filter)=&mut self.filter {self.out.extend(filter.release_all().into_iter().map(|(vk,down)|Output::Key(vk,down)));}
        self.filter=None;
    }
}
pub struct Keyboard {
    connection:Connection, queue:EventQueue<State>, state:State,
    surface:wl_surface::WlSurface,
    inhibitor:Option<inhibitor::ZwpKeyboardShortcutsInhibitorV1>,
}
impl Keyboard {
    pub fn new(cc:&eframe::CreationContext<'_>)->Option<Self> {
        let RawDisplayHandle::Wayland(display)=cc.display_handle().ok()?.as_raw() else {return None};
        let RawWindowHandle::Wayland(window)=cc.window_handle().ok()?.as_raw() else {return None};
        // SAFETY: eframe owns these live objects and outlives App and this helper.
        // The foreign backend does not disconnect the borrowed display on drop.
        let backend=unsafe {Backend::from_foreign_display(display.display.as_ptr().cast())};
        let connection=Connection::from_backend(backend);
        let id=unsafe {ObjectId::from_ptr(wl_surface::WlSurface::interface(),window.surface.as_ptr().cast()).ok()?};
        let surface=wl_surface::WlSurface::from_id(&connection,id).ok()?;
        let queue=connection.new_event_queue();
        let state=State{surface:surface.id().protocol_id(),..Default::default()};
        connection.display().get_registry(&queue.handle(),());
        // winit owns reads on this borrowed display. A roundtrip here can consume
        // its startup configure events before its event loop has begun dispatching.
        connection.flush().ok()?;
        Some(Self {connection,queue,state,surface,inhibitor:None})
    }
    pub fn update(&mut self,wanted:bool,escape:EscapeKeys)->(bool,Vec<Output>) {
        // winit reads the shared display. Dispatch only objects on our own event queue.
        if self.queue.dispatch_pending(&mut self.state).is_err() {self.state.active=false;self.state.release();}
        if self.state.escape!=escape {self.state.release();self.state.escape=escape;}
        #[cfg(feature="ui-review")]
        if self.state.wanted!=wanted && std::env::var_os("SECPAR_REVIEW_KEYS").is_some() {
            eprintln!("review_wayland wanted={wanted} seat={} manager={} focused={}",self.state.seat.is_some(),self.state.manager.is_some(),self.state.focused);
        }
        self.state.wanted=wanted;
        if wanted && self.inhibitor.is_none() {
            if let (Some(manager),Some(seat))=(&self.state.manager,&self.state.seat) {
                let inhibitor=manager.inhibit_shortcuts(&self.surface,seat,&self.queue.handle(),());
                self.state.inhibitor_id=Some(inhibitor.id());
                self.inhibitor=Some(inhibitor);
            }
        } else if !wanted {
            self.state.release();self.state.active=false;self.state.inhibitor_id=None;
            if let Some(inhibitor)=self.inhibitor.take() {if inhibitor.is_alive(){inhibitor.destroy();}}
        }
        let _=self.connection.flush();
        let active=wanted && self.state.active && self.state.focused;
        (active,std::mem::take(&mut self.state.out))
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        if let Some(inhibitor)=self.inhibitor.take(){if inhibitor.is_alive(){inhibitor.destroy();}}
        if let Some(keyboard)=self.state.keyboard.take(){if keyboard.is_alive() && keyboard.version()>=3 {keyboard.release();}}
        if let Some(seat)=self.state.seat.take(){if seat.is_alive() && seat.version()>=5 {seat.release();}}
        if let Some(manager)=self.state.manager.take(){if manager.is_alive(){manager.destroy();}}
        let _=self.connection.flush();
    }
}
impl Dispatch<wl_registry::WlRegistry,()> for State {
    fn event(state:&mut Self,registry:&wl_registry::WlRegistry,event:wl_registry::Event,_:&(),_:&Connection,qh:&QueueHandle<Self>) {
        if let wl_registry::Event::Global{name,interface,version}=event {
            if interface==manager::ZwpKeyboardShortcutsInhibitManagerV1::interface().name {
                state.manager=Some(registry.bind(name,1,qh,()));
            } else if interface==wl_seat::WlSeat::interface().name && state.seat.is_none() {
                state.seat=Some(registry.bind(name,version.min(7),qh,()));
            }
        }
    }
}
impl Dispatch<wl_seat::WlSeat,()> for State {
    fn event(state:&mut Self,seat:&wl_seat::WlSeat,event:wl_seat::Event,_:&(),_:&Connection,qh:&QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities{capabilities:WEnum::Value(capabilities)}=event {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none(){state.keyboard=Some(seat.get_keyboard(qh,()));}
            if !capabilities.contains(wl_seat::Capability::Keyboard) {
                state.release();state.focused=false;
                if let Some(keyboard)=state.keyboard.take(){if keyboard.is_alive() && keyboard.version()>=3 {keyboard.release();}}
            }
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard,()> for State {
    fn event(state:&mut Self,_:&wl_keyboard::WlKeyboard,event:wl_keyboard::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        match event {
            wl_keyboard::Event::Enter{surface,keys,..}=>{
                state.release();state.focused=surface.id().protocol_id()==state.surface;
                state.down=keys.chunks_exact(4).filter_map(|v|secpar_host_linux::input::virtual_key(u32::from_ne_bytes(v.try_into().unwrap()))).collect();
            }
            wl_keyboard::Event::Leave{..}=>{state.release();state.focused=false;state.down.clear();}
            wl_keyboard::Event::Key{key,state:WEnum::Value(key_state),..}=>{
                let Some(vk)=secpar_host_linux::input::virtual_key(key) else {return};
                let down=key_state==wl_keyboard::KeyState::Pressed;
                if down {state.down.insert(vk);}else{state.down.remove(&vk);}
                if state.wanted && state.active && state.focused {
                    let filter=state.filter.get_or_insert_with(||Filter::new(state.escape,true,[]));
                    let verdict=filter.key(vk,down);
                    state.out.extend(verdict.send.into_iter().map(|(vk,down)|Output::Key(vk,down)));
                    if let Some(action)=verdict.escape {state.out.push(Output::Escape(action));}
                }
            }
            _=>{}
        }
    }
}
impl Dispatch<manager::ZwpKeyboardShortcutsInhibitManagerV1,()> for State {
    fn event(_: &mut Self,_:&manager::ZwpKeyboardShortcutsInhibitManagerV1,_:manager::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
}
impl Dispatch<inhibitor::ZwpKeyboardShortcutsInhibitorV1,()> for State {
    fn event(state:&mut Self,proxy:&inhibitor::ZwpKeyboardShortcutsInhibitorV1,event:inhibitor::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {
        if state.inhibitor_id.as_ref()!=Some(&proxy.id()) {return;}
        match event {
            inhibitor::Event::Active=>{
                state.active=true;state.release();
                state.filter=Some(Filter::new(state.escape,true,state.down.iter().copied()));
            }
            inhibitor::Event::Inactive=>{state.active=false;state.release();}
            _=>{}
        }
    }
}

use crate::design;
use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum CaptureShortcut { #[default] CtrlAltG, CtrlShiftG, CtrlAltZ }
impl CaptureShortcut {
    pub fn label(self) -> &'static str { match self {Self::CtrlAltG=>"Ctrl+Alt+G",Self::CtrlShiftG=>"Ctrl+Shift+G",Self::CtrlAltZ=>"Ctrl+Alt+Z"} }
    pub fn matches(self,key:Key,m:Modifiers) -> bool {
        !m.mac_cmd && m.ctrl && match self {
            Self::CtrlAltG=>key==Key::G && m.alt && !m.shift,
            Self::CtrlShiftG=>key==Key::G && m.shift && !m.alt,
            Self::CtrlAltZ=>key==Key::Z && m.alt && !m.shift,
        }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width:u32, pub height:u32, pub fps:u32, pub kbps:u32,
    pub audio:bool, pub shortcut:CaptureShortcut,
    #[serde(skip)] note:String,
}
impl Default for Settings {
    fn default()->Self {Self{width:1280,height:720,fps:60,kbps:30000,audio:true,shortcut:CaptureShortcut::default(),note:String::new()}}
}
fn path()->Option<PathBuf> {
    #[cfg(windows)]
    { return std::env::var_os("APPDATA").map(|p|PathBuf::from(p).join("Secpar/client.json")); }
    #[cfg(target_os="linux")]
    { return std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p|p.is_absolute()).or_else(||std::env::var_os("HOME").map(|p|PathBuf::from(p).join(".config"))).map(|p|p.join("secpar/client.json")); }
    #[cfg(target_os="macos")]
    { return std::env::var_os("HOME").map(|p|PathBuf::from(p).join("Library/Application Support/Secpar/client.json")); }
    #[allow(unreachable_code)] None
}
impl Settings {
    pub fn load()->Self {
        // Review fixtures never read personal settings.
        #[cfg(feature="ui-review")]
        if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() {return Self::default()}
        let mut value=path().and_then(|p|std::fs::read(p).ok()).filter(|b|b.len()<=8192).and_then(|b|serde_json::from_slice::<Self>(&b).ok()).unwrap_or_default();
        if ![(0,0),(1280,720),(1920,1080),(2560,1440),(3840,2160)].contains(&(value.width,value.height)) {(value.width,value.height)=(1280,720);}
        if ![30,60,90,120].contains(&value.fps) {value.fps=60;}
        value.kbps=value.kbps.clamp(1000,150000);value
    }
    pub fn save(&mut self) {
        #[cfg(feature="ui-review")]
        if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() {return}
        let result=(||->Option<()> {let path=path()?;std::fs::create_dir_all(path.parent()?).ok()?;let temp=path.with_extension("tmp");std::fs::write(&temp,serde_json::to_vec_pretty(self).ok()?).ok()?;std::fs::rename(temp,path).ok()?;Some(())})();
        self.note=if result.is_some(){String::new()}else{"Could not save client settings".into()};
    }
    pub fn options(&self)->secpar_client::session::Options {
        secpar_client::session::Options{width:self.width,height:self.height,fps:self.fps,max_kbps:self.kbps,audio:self.audio,..Default::default()}
    }
    pub fn ui(&mut self,ui:&mut egui::Ui)->bool {
        let old=self.clone();
        ui.heading("Client settings");ui.add_space(8.);
        design::surface().inner_margin(10).show(ui,|ui| {
            ui.set_width(ui.available_width());ui.spacing_mut().item_spacing.y=4.;
            ui.label(egui::RichText::new("Video").strong());
            ui.columns(2,|c| {
                c[0].label("Resolution");
                let text=if self.width==0 {"Host native".into()}else{format!("{} × {}",self.width,self.height)};
                egui::ComboBox::from_id_salt("client-resolution").width(c[0].available_width()).selected_text(text).show_ui(&mut c[0],|ui| {
                    for (w,h,label) in [(0,0,"Host native"),(1280,720,"1280 × 720"),(1920,1080,"1920 × 1080"),(2560,1440,"2560 × 1440"),(3840,2160,"3840 × 2160")] {
                        if ui.selectable_label(self.width==w,label).clicked(){self.width=w;self.height=h;}
                    }
                });
                c[1].label("Frame rate");
                egui::ComboBox::from_id_salt("client-fps").width(c[1].available_width()).selected_text(format!("{} fps",self.fps)).show_ui(&mut c[1],|ui| {for fps in [30,60,90,120] {ui.selectable_value(&mut self.fps,fps,format!("{fps} fps"));}});
            });
            ui.columns(2,|c| {
                c[0].label("Bitrate limit");
                egui::ComboBox::from_id_salt("client-bitrate").width(c[0].available_width()).selected_text(format!("{} Mbps",self.kbps/1000)).show_ui(&mut c[0],|ui| {for kbps in [5000,10000,20000,30000,50000,80000] {ui.selectable_value(&mut self.kbps,kbps,format!("{} Mbps",kbps/1000));}});
                c[1].label("Decoder");c[1].add_sized([c[1].available_width(),32.],egui::Label::new("H.264 · software"));
            });
        });
        ui.add_space(4.);
        design::surface().inner_margin(10).show(ui,|ui| {
            ui.set_width(ui.available_width());ui.spacing_mut().item_spacing.y=4.;
            ui.columns(2,|c| {
                c[0].label("Capture / release mouse");
                egui::ComboBox::from_id_salt("client-shortcut").width(c[0].available_width()).selected_text(self.shortcut.label()).show_ui(&mut c[0],|ui| {for choice in [CaptureShortcut::CtrlAltG,CaptureShortcut::CtrlShiftG,CaptureShortcut::CtrlAltZ] {ui.selectable_value(&mut self.shortcut,choice,choice.label());}});
                c[1].label("Audio");c[1].checkbox(&mut self.audio,"Play host audio");
            });
        });
        if !self.note.is_empty(){ui.label(&self.note);}
        *self!=old
    }
}

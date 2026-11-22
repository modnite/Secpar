use crate::{design,keys,video};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum CaptureShortcut { #[default] CtrlAltG, CtrlShiftG, CtrlAltZ }
// Retained in persisted settings for compatibility; the stream's escape chord owns the hotkeys now.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width:u32, pub height:u32, pub fps:u32, pub kbps:u32,
    pub audio:bool, pub clipboard:bool, pub shortcut:CaptureShortcut,
    pub decoder:video::Preference,
    /// Ask for 4:4:4 colour: sharper coloured text, about twice the video to send.
    pub full_colour:bool,
    /// Where Files downloads go. Empty means Downloads/Secpar.
    pub files_dir:String,
    /// While streaming, Win, Alt+Tab, Alt+F4 and the like go to the host (Windows keyboard hook).
    pub system_keys:bool,
    /// Modifiers of the client's own hotkeys during a stream. The last key picks the action.
    pub escape:keys::EscapeKeys,
    #[serde(skip)] pub active_decoder:Option<String>,
    #[serde(skip)] note:String,
}
impl Default for Settings {
    fn default()->Self {Self{width:1280,height:720,fps:60,kbps:30000,audio:true,clipboard:true,shortcut:CaptureShortcut::default(),decoder:video::Preference::default(),full_colour:false,files_dir:String::new(),system_keys:true,escape:keys::EscapeKeys::default(),active_decoder:None,note:String::new()}}
}
/// Host native first, then largest to smallest. The stream menu uses the same list.
pub(crate) const RESOLUTIONS:[(u32,u32);6]=[(0,0),(3840,2160),(2560,1440),(1920,1080),(1280,720),(854,480)];
pub(crate) fn resolution_label(w:u32,h:u32)->String {
    if w==0 {"Host native".into()} else {format!("{w} × {h}")}
}
pub(crate) fn path()->Option<PathBuf> {
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
        if !RESOLUTIONS.contains(&(value.width,value.height)) {(value.width,value.height)=(1280,720);}
        if ![60,90,120].contains(&value.fps) {value.fps=60;}
        value.kbps=value.kbps.clamp(1000,150000);value
    }
    pub fn save(&mut self) {
        #[cfg(feature="ui-review")]
        if std::env::var_os("SECPAR_REVIEW_SCREENSHOT_TO").is_some() {return}
        let result=(||->Option<()> {let path=path()?;std::fs::create_dir_all(path.parent()?).ok()?;let temp=path.with_extension("tmp");std::fs::write(&temp,serde_json::to_vec_pretty(self).ok()?).ok()?;std::fs::rename(temp,path).ok()?;Some(())})();
        self.note=if result.is_some(){String::new()}else{"Could not save client settings".into()};
    }
    pub fn options(&self)->secpar_client::session::Options {
        secpar_client::session::Options{width:self.width,height:self.height,fps:self.fps,max_kbps:self.kbps,audio:self.audio,clipboard:self.clipboard,..Default::default()}
    }
    pub fn ui(&mut self,ui:&mut egui::Ui)->bool {
        let old=self.clone();
        ui.spacing_mut().item_spacing.y=8.;
        design::group(ui,"Picture",|ui| {
            let width=design::value_width(ui);
            design::row(ui,"Resolution",|ui| {
                egui::ComboBox::from_id_salt("client-resolution").width(width).selected_text(resolution_label(self.width,self.height)).show_ui(ui,|ui| {
                    for (w,h) in RESOLUTIONS {
                        if ui.selectable_label(self.width==w,resolution_label(w,h)).clicked(){self.width=w;self.height=h;}
                    }
                });
            });
            design::row(ui,"Frame rate",|ui| {
                egui::ComboBox::from_id_salt("client-fps").width(width).selected_text(format!("{} fps",self.fps)).show_ui(ui,|ui| {for fps in [60,90,120] {ui.selectable_value(&mut self.fps,fps,format!("{fps} fps"));}});
            });
            design::row(ui,"Bitrate",|ui| {
                egui::ComboBox::from_id_salt("client-bitrate").width(width).selected_text(format!("{} Mbps",self.kbps/1000)).show_ui(ui,|ui| {for kbps in [5000,10000,20000,30000,50000,80000] {ui.selectable_value(&mut self.kbps,kbps,format!("{} Mbps",kbps/1000));}});
            });
            let hardware=video::hardware_available(ui.ctx());
            design::row(ui,"Decoder",|ui| {
                egui::ComboBox::from_id_salt("client-decoder").width(width).selected_text(self.decoder.label()).show_ui(ui,|ui| {
                    ui.selectable_value(&mut self.decoder,video::Preference::Automatic,video::Preference::Automatic.label());
                    if hardware==Some(true) {ui.selectable_value(&mut self.decoder,video::Preference::HARDWARE,video::Preference::HARDWARE.label());}
                    ui.selectable_value(&mut self.decoder,video::Preference::Software,video::Preference::Software.label());
                }).response.on_hover_text(match hardware {
                    Some(true)=>"H.264 hardware decode verified on this computer",
                    Some(false)=>"No working hardware H.264 decoder detected",
                    None=>"Checking hardware decoding",
                });
            });
            design::switch_row(ui,&mut self.full_colour,"Full colour (4:4:4)");
            if let Some(active)=&self.active_decoder {ui.label(egui::RichText::new(active).size(12.).color(design::MUTED));ui.add_space(4.);}
        });
        design::group(ui,"Sound and clipboard",|ui| {
            design::switch_row(ui,&mut self.audio,"Play sound here");
            design::switch_row(ui,&mut self.clipboard,"Share clipboard");
        });
        design::group(ui,"Keyboard",|ui| {
            #[cfg(any(windows,target_os="linux"))]
            design::switch_row(ui,&mut self.system_keys,"Send system shortcuts");
            let width=design::value_width(ui);
            design::row(ui,"Release keys",|ui| {
                egui::ComboBox::from_id_salt("client-escape").width(width).selected_text(self.escape.label()).show_ui(ui,|ui| {
                    for keys in keys::EscapeKeys::ALL {ui.selectable_value(&mut self.escape,keys,keys.label());}
                });
            });
            ui.label(egui::RichText::new(format!("{}+  W full screen   Q disconnect   S menu",self.escape.label())).size(12.).color(design::MUTED));
            ui.add_space(4.);
        });
        if !self.note.is_empty(){ui.label(&self.note);}
        *self!=old
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_settings_files_load_with_the_new_keyboard_defaults() {
        let old=br#"{"width":1920,"height":1080,"fps":60,"kbps":30000,"audio":true,"clipboard":true,"shortcut":"CtrlAltZ","decoder":"Automatic","files_dir":""}"#;
        let loaded:Settings=serde_json::from_slice(old).unwrap();
        assert!(loaded.system_keys);
        assert_eq!(loaded.escape,keys::EscapeKeys::CtrlAlt);
        assert_eq!(loaded.width,1920);
        assert!(loaded.shortcut==CaptureShortcut::CtrlAltZ);
        let empty:Settings=serde_json::from_slice(b"{}").unwrap();
        assert!(empty==Settings::default());
    }
    #[test]
    fn keyboard_settings_round_trip() {
        let mut s=Settings::default();s.system_keys=false;s.escape=keys::EscapeKeys::CtrlShift;
        let back:Settings=serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert!(!back.system_keys);assert_eq!(back.escape,keys::EscapeKeys::CtrlShift);
    }
    #[test]
    fn resolutions_descend_after_native() {
        assert_eq!(RESOLUTIONS[0],(0,0));
        assert!(RESOLUTIONS[1..].windows(2).all(|p|p[0].0>p[1].0));
        assert!(RESOLUTIONS.contains(&(854,480)));
        assert_eq!(resolution_label(0,0),"Host native");
        assert_eq!(resolution_label(854,480),"854 × 480");
    }
}

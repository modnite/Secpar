use crate::{design,video};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum CaptureShortcut { #[default] CtrlAltG, CtrlShiftG, CtrlAltZ }
// Retained in persisted settings for compatibility; Ctrl+Alt+W now owns fullscreen.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width:u32, pub height:u32, pub fps:u32, pub kbps:u32,
    pub audio:bool, pub clipboard:bool, pub shortcut:CaptureShortcut,
    pub decoder:video::Preference,
    /// Where Files downloads go. Empty means Downloads/Secpar.
    pub files_dir:String,
    #[serde(skip)] pub active_decoder:Option<String>,
    #[serde(skip)] note:String,
}
impl Default for Settings {
    fn default()->Self {Self{width:1280,height:720,fps:60,kbps:30000,audio:true,clipboard:true,shortcut:CaptureShortcut::default(),decoder:video::Preference::default(),files_dir:String::new(),active_decoder:None,note:String::new()}}
}
/// Host native first, then largest to smallest. The stream menu uses the same list.
pub(crate) const RESOLUTIONS:[(u32,u32);6]=[(0,0),(3840,2160),(2560,1440),(1920,1080),(1280,720),(854,480)];
pub(crate) fn resolution_label(w:u32,h:u32)->String {
    if w==0 {"Host native".into()} else {format!("{w} × {h}")}
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
        if !RESOLUTIONS.contains(&(value.width,value.height)) {(value.width,value.height)=(1280,720);}
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
        secpar_client::session::Options{width:self.width,height:self.height,fps:self.fps,max_kbps:self.kbps,audio:self.audio,clipboard:self.clipboard,..Default::default()}
    }
    pub fn ui(&mut self,ui:&mut egui::Ui)->bool {
        let old=self.clone();
        design::page_header(ui,"Settings",|_|{});ui.add_space(8.);
        design::surface().inner_margin(10).show(ui,|ui| {
            ui.set_width(ui.available_width());ui.spacing_mut().item_spacing.y=4.;
            ui.label(egui::RichText::new("Video").strong());
            ui.columns(2,|c| {
                c[0].label("Resolution");
                let text=resolution_label(self.width,self.height);
                egui::ComboBox::from_id_salt("client-resolution").width(c[0].available_width()).selected_text(text).show_ui(&mut c[0],|ui| {
                    for (w,h) in RESOLUTIONS {
                        if ui.selectable_label(self.width==w,resolution_label(w,h)).clicked(){self.width=w;self.height=h;}
                    }
                });
                c[1].label("Frame rate");
                egui::ComboBox::from_id_salt("client-fps").width(c[1].available_width()).selected_text(format!("{} fps",self.fps)).show_ui(&mut c[1],|ui| {for fps in [30,60,90,120] {ui.selectable_value(&mut self.fps,fps,format!("{fps} fps"));}});
            });
            ui.columns(2,|c| {
                c[0].label("Bitrate limit");
                egui::ComboBox::from_id_salt("client-bitrate").width(c[0].available_width()).selected_text(format!("{} Mbps",self.kbps/1000)).show_ui(&mut c[0],|ui| {for kbps in [5000,10000,20000,30000,50000,80000] {ui.selectable_value(&mut self.kbps,kbps,format!("{} Mbps",kbps/1000));}});
                c[1].label("Decoder");
                let hardware=video::hardware_available(c[1].ctx());
                let selected=match self.decoder {video::Preference::Automatic=>"Automatic",video::Preference::Software=>"Software",video::Preference::Vaapi=>"VA-API"};
                egui::ComboBox::from_id_salt("client-decoder").width(c[1].available_width()).selected_text(selected).show_ui(&mut c[1],|ui| {
                    ui.selectable_value(&mut self.decoder,video::Preference::Automatic,"Automatic");
                    if hardware==Some(true) {ui.selectable_value(&mut self.decoder,video::Preference::Vaapi,"VA-API");}
                    ui.selectable_value(&mut self.decoder,video::Preference::Software,"Software");
                }).response.on_hover_text(match hardware {
                    Some(true)=>"H.264 VA-API decode verified on this computer",
                    Some(false) if cfg!(target_os="linux")=>"No working VA-API H.264 decoder detected",
                    Some(false)=>"Hardware decoding is currently implemented for Linux only",
                    None=>"Checking hardware decoding",
                });
            });
            if let Some(active)=&self.active_decoder {ui.small(active);}
        });
        ui.add_space(4.);
        design::surface().inner_margin(10).show(ui,|ui| {
            ui.set_width(ui.available_width());ui.spacing_mut().item_spacing.y=4.;
            ui.label(egui::RichText::new("Audio").strong());
            ui.checkbox(&mut self.audio,"Play host audio");
            ui.label(egui::RichText::new("Clipboard").strong());
            ui.checkbox(&mut self.clipboard,"Share clipboard");
        });
        ui.add_space(4.);
        design::surface().inner_margin(10).show(ui,|ui| {
            ui.set_width(ui.available_width());ui.spacing_mut().item_spacing.y=4.;
            ui.label(egui::RichText::new("Keyboard").strong());
            ui.columns(2,|c| {c[0].label("Fullscreen");c[1].label("Ctrl+Alt+W");});
        });
        if !self.note.is_empty(){ui.label(&self.note);}
        *self!=old
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolutions_descend_after_native() {
        assert_eq!(RESOLUTIONS[0],(0,0));
        assert!(RESOLUTIONS[1..].windows(2).all(|p|p[0].0>p[1].0));
        assert!(RESOLUTIONS.contains(&(854,480)));
        assert_eq!(resolution_label(0,0),"Host native");
        assert_eq!(resolution_label(854,480),"854 × 480");
    }
}

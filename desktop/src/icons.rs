//! Original Lucide0.468.0 artwork. Source, pinned commit and ISC license:
//! assets/icons/lucide. Rasterized from SVG; tinted after DPI-aware downsampling.
use eframe::egui::{self,Color32,Rect,TextureHandle,TextureOptions};
#[derive(Clone,Copy,Debug)]
pub enum Icon {Window,Windowed,Decoder,Display,Clipboard,Stats,Audio,Help,Exit,Back,Check,Chevron,Keyboard,Mouse,Refresh,SignIn,Download}
impl Icon {
    fn png(self)->&'static [u8] {match self {
        Self::Window=>include_bytes!("../../assets/icons/lucide/png/maximize.png"),
        Self::Windowed=>include_bytes!("../../assets/icons/lucide/png/minimize.png"),
        Self::Decoder=>include_bytes!("../../assets/icons/lucide/png/cpu.png"),
        Self::Display=>include_bytes!("../../assets/icons/lucide/png/monitor.png"),
        Self::Clipboard=>include_bytes!("../../assets/icons/lucide/png/clipboard.png"),
        Self::Stats=>include_bytes!("../../assets/icons/lucide/png/chart-no-axes-column-increasing.png"),
        Self::Audio=>include_bytes!("../../assets/icons/lucide/png/volume-2.png"),
        Self::Help=>include_bytes!("../../assets/icons/lucide/png/circle-help.png"),
        Self::Exit=>include_bytes!("../../assets/icons/lucide/png/log-out.png"),
        Self::Back=>include_bytes!("../../assets/icons/lucide/png/arrow-left.png"),
        Self::Check=>include_bytes!("../../assets/icons/lucide/png/check.png"),
        Self::Chevron=>include_bytes!("../../assets/icons/lucide/png/chevron-right.png"),
        Self::Keyboard=>include_bytes!("../../assets/icons/lucide/png/keyboard.png"),
        Self::Refresh=>include_bytes!("../../assets/icons/lucide/png/refresh-cw.png"),
        Self::SignIn=>include_bytes!("../../assets/icons/lucide/png/log-in.png"),
        Self::Download=>include_bytes!("../../assets/icons/lucide/png/download.png"),
        Self::Mouse=>include_bytes!("../../assets/icons/lucide/png/mouse.png"),
    }}
}
pub fn paint(ui:&egui::Ui,kind:Icon,rect:Rect,tint:Color32) {
    let pixels=(rect.width()*ui.ctx().pixels_per_point()).round().clamp(1.,96.) as u32;
    let id=egui::Id::new(("lucide",format!("{kind:?}"),pixels));
    let cached=ui.ctx().data(|d|d.get_temp::<TextureHandle>(id));
    let texture=cached.unwrap_or_else(||{
        let png=image::load_from_memory(kind.png()).expect("Bundled Lucide icon")
            .resize_exact(pixels,pixels,image::imageops::FilterType::Lanczos3).to_rgba8();
        let texture=ui.ctx().load_texture(format!("Lucide {kind:?} {pixels}"),
            egui::ColorImage::from_rgba_unmultiplied([pixels as usize,pixels as usize],png.as_raw()),TextureOptions::LINEAR);
        ui.ctx().data_mut(|d|d.insert_temp(id,texture.clone()));texture
    });
    ui.painter().image(texture.id(),rect,Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),tint);
}

pub fn status(ui:&mut egui::Ui,kind:Icon,label:&str,allowed:bool) {
    let (rect,response)=ui.allocate_exact_size(egui::vec2(18.,18.),egui::Sense::hover());
    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Label,true,label));
    response.on_hover_text(label);
    paint(ui,kind,rect,if allowed{crate::design::ACCENT}else{crate::design::MUTED});
}

pub fn button(ui:&mut egui::Ui,kind:Icon,label:&str,enabled:bool)->bool {
    let response=ui.add_enabled(enabled,egui::Button::new("").min_size(egui::vec2(32.,32.)))
        .on_hover_text(label).on_disabled_hover_text(label);
    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,enabled,label));
    paint(ui,kind,egui::Rect::from_center_size(response.rect.center(),egui::vec2(18.,18.)),
        if enabled {crate::design::TEXT}else{crate::design::MUTED});
    response.clicked()
}

//! One computer card. Names wrap instead of being cut, and the preview sits beside the text so
//! a card with a picture is only as tall as the picture.
use crate::{design, previews};
use eframe::egui::{self, Color32, RichText};

/// Width of the Connect button.
const BUTTON: f32 = 144.;
/// Width kept for a trailing menu button.
const MENU: f32 = 40.;
/// The menu button as drawn, measured in a screenshot. A card without a menu leaves this much room.
const MENU_DRAWN: f32 = 38.;
const MARGIN: f32 = 8.;
const NAME: f32 = 14.;
const SMALL: f32 = 12.;
/// Never narrower than this, so a very small window wraps the name instead of losing it.
const MIN_TEXT: f32 = 48.;

pub struct Card<'a> {
    pub name: &'a str,
    pub status: Option<(&'a str, Color32)>,
    pub platform: &'a str,
    pub preview: Option<&'a egui::TextureHandle>,
    /// `Some(enabled)` shows the Connect button.
    pub connect: Option<bool>,
    /// Show the Files action for owned and manually paired computers.
    pub files: Option<bool>,
    /// A trailing "..." menu, drawn by the caller.
    pub menu: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
}

#[derive(Default)]
pub struct Action { pub connect: bool, pub files: bool }

/// Draws the computer actions in the same row on narrow and wide cards.
pub fn show(ui: &mut egui::Ui, card: Card) -> Action {
    if ui.available_width()<520. {return vertical(ui,card);}
    let mut clicked = Action::default();
    design::surface().inner_margin(MARGIN as i8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let gap = ui.spacing().item_spacing.x;
        let platform_w = if card.platform.is_empty() {
            0.
        } else {
            ui.fonts_mut(|f| f.layout_no_wrap(card.platform.into(), egui::FontId::proportional(SMALL), design::MUTED).size().x) + gap
        };
        // Every card keeps the slot of the menu button, used or not, so Connect sits in the same place on all of them.
        let right = platform_w + if card.connect.is_some() { BUTTON + gap } else { 0. } + if card.files.is_some() { 72. + gap } else { 0. } + MENU + gap;
        let thumb = if card.preview.is_some() { previews::WIDTH + gap } else { 0. };
        let text_w = (ui.available_width() - thumb - 8. - gap - right).max(MIN_TEXT);
        let name_h = ui
            .fonts_mut(|f| f.layout(card.name.into(), egui::FontId::proportional(NAME), design::TEXT, text_w))
            .size()
            .y;
        let status_h = card.status.map_or(0., |_| ui.fonts_mut(|f| f.row_height(&egui::FontId::proportional(SMALL))) + 2.);
        let text_h = name_h + status_h;
        let row_h = text_h.max(if card.preview.is_some() { previews::HEIGHT } else { design::CONTROL });
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            design::dot(ui, card.status.map_or(design::ACCENT, |(_, color)| color));
            if let Some(texture) = card.preview {
                previews::show(ui, texture);
            }
            ui.allocate_ui_with_layout(egui::vec2(text_w, row_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.;
                ui.add_space((row_h - text_h) / 2.);
                ui.add(egui::Label::new(RichText::new(card.name).strong()).wrap());
                if let Some((status, _)) = card.status {
                    ui.label(RichText::new(status).size(SMALL).color(design::MUTED));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(menu) = card.menu {
                    menu(ui);
                } else {
                    ui.add_space(MENU_DRAWN + ui.spacing().item_spacing.x);
                }
                if let Some(enabled) = card.files {
                    clicked.files = ui.add_enabled(enabled, egui::Button::new("Files").min_size(egui::vec2(72., design::CONTROL))).on_hover_text("Browse and transfer files").clicked();
                }
                if let Some(enabled) = card.connect {
                    clicked.connect = ui.add_enabled(enabled, design::primary("Connect")).clicked();
                }
                if !card.platform.is_empty() {
                    ui.label(RichText::new(card.platform).size(SMALL).color(design::MUTED));
                }
            });
        });
    });
    clicked
}

/// Android's card order: identity, preview, then the connection action.
fn vertical(ui:&mut egui::Ui,mut card:Card)->Action {
    let mut clicked=Action::default();
    design::surface().inner_margin(12).show(ui,|ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y=8.;
        ui.horizontal_top(|ui| {
            design::dot(ui,card.status.map_or(design::ACCENT,|(_,color)|color));
            let platform=ui.fonts_mut(|fonts|fonts.layout_no_wrap(card.platform.into(),egui::FontId::proportional(SMALL),design::MUTED).size().x);
            let right=platform+if card.menu.is_some(){MENU}else{0.}+ui.spacing().item_spacing.x;
            let width=(ui.available_width()-right).max(MIN_TEXT);
            ui.allocate_ui_with_layout(egui::vec2(width,36.),egui::Layout::top_down(egui::Align::Min),|ui| {
                ui.set_width(width);
                ui.add(egui::Label::new(RichText::new(card.name).size(NAME).strong()).wrap());
                if let Some((status,_))=card.status {ui.add(egui::Label::new(RichText::new(status).size(SMALL).color(design::MUTED)).wrap());}
            });
            if !card.platform.is_empty(){ui.label(RichText::new(card.platform).size(SMALL).color(design::MUTED));}
            if let Some(menu)=card.menu.as_mut(){menu(ui);}
        });
        if let Some(texture)=card.preview {
            let width=ui.available_width().min(360.);
            ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(width,width*9./16.)));
        }
        if let Some(enabled)=card.connect {
            ui.horizontal(|ui| {
                let width=ui.available_width()-if card.files.is_some(){80.+ui.spacing().item_spacing.x}else{0.};
                clicked.connect=ui.add_enabled_ui(enabled,|ui|ui.add_sized([width,design::CONTROL],design::primary("Connect"))).inner.clicked();
                if let Some(enabled)=card.files {
                    clicked.files=ui.add_enabled_ui(enabled,|ui|ui.add_sized([80.,design::CONTROL],egui::Button::new("Files"))).inner.on_hover_text("Browse and transfer files").clicked();
                }
            });
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Height of one card drawn `width` wide.
    fn height(width: f32, name: &str, preview: bool) -> f32 {
        let ctx = egui::Context::default();
        design::configure(&ctx);
        let texture = ctx.load_texture("t", egui::ColorImage::example(), Default::default());
        let mut out = 0.;
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(width + 40., 600.))), ..Default::default() };
        for _ in 0..2 {
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let card = Card { name, status: Some(("Online", design::OK)), platform: "Linux", preview: preview.then_some(&texture), connect: Some(true), files: Some(true), menu: None };
                    let top = ui.cursor().top();
                    show(ui, card);
                    out = ui.cursor().top() - top - ui.spacing().item_spacing.y;
                });
            });
        }
        out
    }

    #[test]
    fn files_and_connect_are_distinct_card_actions_at_narrow_and_wide_widths() {
        for width in [380., 600.] {
            for label in ["Files", "Connect"] {
                let ctx=egui::Context::default();
                design::configure(&ctx);
                let mut target=egui::Pos2::ZERO;
                for frame in 0..3 {
                    let events=if frame==2 {vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton{pos:target,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()},
                        egui::Event::PointerButton{pos:target,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()},
                    ]}else{vec![]};
                    let mut action=Action::default();
                    let output=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(width,400.))),events,..Default::default()},|ctx| {
                        egui::CentralPanel::default().show(ctx,|ui| {
                            action=show(ui,Card{name:"Computer",status:None,platform:"",preview:None,connect:Some(true),files:Some(true),menu:None});
                        });
                    });
                    for shape in output.shapes {
                        if let egui::epaint::Shape::Text(text)=shape.shape {
                            if text.galley.job.text==label {target=egui::Rect::from_min_size(text.pos,text.galley.size()).center();}
                        }
                    }
                    if frame==2 {assert_eq!(action.files,label=="Files");assert_eq!(action.connect,label=="Connect");}
                }
            }
        }
    }

    #[test]
    fn a_preview_does_not_make_the_card_taller_than_it_needs_to_be() {
        let plain = height(640., "Garage PC", false);
        let with = height(640., "Garage PC", true);
        assert!((plain - (design::CONTROL + 2. * MARGIN + 2.)).abs() < 0.5);
        assert!((with - (previews::HEIGHT + 2. * MARGIN + 2.)).abs() < 0.5);
        assert!(with - plain <= 13.);
    }
    #[test]
    fn a_long_name_stays_on_one_line_at_the_default_width() {
        let long = "Brother's Gaming Laptop (Linux)";
        assert!((height(680., long, true) - (previews::HEIGHT + 2. * MARGIN + 2.)).abs() < 0.5);
    }
    #[test]
    fn a_long_name_wraps_instead_of_being_cut_in_a_narrow_window() {
        let long = "Brother's Gaming Laptop (Linux)";
        assert!(height(300., long, false) > height(680., long, false));
    }
}

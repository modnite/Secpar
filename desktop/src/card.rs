//! One computer card. Names wrap instead of being cut, and the preview sits beside the text so
//! a card with a picture is only as tall as the picture.
use crate::{design, previews};
use eframe::egui::{self, Color32, RichText};

/// Width of the Connect button.
const BUTTON: f32 = 144.;
/// Width kept for a trailing menu button.
const MENU: f32 = 40.;
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
    /// A trailing "..." menu, drawn by the caller.
    pub menu: Option<&'a mut dyn FnMut(&mut egui::Ui)>,
}

/// Draws the card. Returns true when Connect was clicked.
pub fn show(ui: &mut egui::Ui, card: Card) -> bool {
    let mut clicked = false;
    design::surface().inner_margin(MARGIN as i8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let gap = ui.spacing().item_spacing.x;
        let platform_w = if card.platform.is_empty() {
            0.
        } else {
            ui.fonts_mut(|f| f.layout_no_wrap(card.platform.into(), egui::FontId::proportional(SMALL), design::MUTED).size().x) + gap
        };
        let right = platform_w + if card.connect.is_some() { BUTTON + gap } else { 0. } + if card.menu.is_some() { MENU + gap } else { 0. };
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
            if let Some(texture) = card.preview {
                previews::show(ui, texture);
            }
            design::dot(ui, card.status.map_or(design::ACCENT, |(_, color)| color));
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
                }
                if let Some(enabled) = card.connect {
                    clicked = ui.add_enabled(enabled, design::primary("Connect")).clicked();
                }
                if !card.platform.is_empty() {
                    ui.label(RichText::new(card.platform).size(SMALL).color(design::MUTED));
                }
            });
        });
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
                    let card = Card { name, status: Some(("Online", design::OK)), platform: "Linux", preview: preview.then_some(&texture), connect: Some(true), menu: None };
                    let top = ui.cursor().top();
                    show(ui, card);
                    out = ui.cursor().top() - top - ui.spacing().item_spacing.y;
                });
            });
        }
        out
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

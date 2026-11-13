use eframe::egui::{self, Color32, RichText, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(10, 255, 182);
pub const BG: Color32 = Color32::from_rgb(17, 21, 28);
pub const MUTED: Color32 = Color32::from_rgb(152, 162, 184);
pub const BAD: Color32 = Color32::from_rgb(248, 113, 113);
pub const OK: Color32 = Color32::from_rgb(52, 211, 153);
pub const WARN: Color32 = Color32::from_rgb(251, 191, 36);
pub const TEXT: Color32 = Color32::from_rgb(230, 234, 242);
pub const LINE: Color32 = Color32::from_rgb(49, 59, 73);
pub const CONTROL: f32 = 32.;

pub fn configure(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(17, 21, 28);
    style.visuals.window_fill = Color32::from_rgb(25, 31, 41);
    style.visuals.extreme_bg_color = Color32::from_rgb(13, 17, 23);
    style.visuals.faint_bg_color = Color32::from_rgb(27, 34, 44);
    style.visuals.selection.bg_fill = Color32::from_rgb(24, 90, 75);
    style.visuals.selection.stroke = Stroke::new(1_f32, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1_f32, LINE);
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(31, 39, 51);
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 39, 51);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1_f32, Color32::from_rgb(65, 77, 94));
    style.visuals.widgets.inactive.fg_stroke.color = TEXT;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 52, 67);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(40, 52, 67);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1_f32, Color32::from_rgb(105, 128, 150));
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(35, 65, 65);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(35, 65, 65);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1_f32, ACCENT);
    style.visuals.widgets.inactive.corner_radius = 5.into();
    style.visuals.widgets.hovered.corner_radius = 5.into();
    style.visuals.widgets.active.corner_radius = 5.into();
    style.spacing.item_spacing = egui::vec2(8., 8.);
    style.spacing.button_padding = egui::vec2(12., 7.);
    style.spacing.interact_size.y = CONTROL;
    // A plain bar in its own gutter. The default floating one is thin and lies over the controls.
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 10.;
    style.spacing.scroll.bar_inner_margin = 6.;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(20.));
    ctx.set_style(style);
}
pub fn surface() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(23, 29, 39))
        .stroke(Stroke::new(1_f32, LINE))
        .corner_radius(7)
        .inner_margin(12)
}
pub fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::from_rgb(7, 38, 30)))
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .min_size(egui::vec2(144., CONTROL))
}
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8., 8.), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3., color);
}

pub fn page_header(ui: &mut egui::Ui, title: &str, actions: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 32.),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.heading(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
        },
    );
}

/// Back navigation uses the same pinned Lucide artwork as the stream menu.
pub fn back_button(ui:&mut egui::Ui)->bool {
    let response=ui.add_sized([28.,28.],egui::Button::new("").frame(false)).on_hover_text("Back to settings");
    response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::Button,true,"Back to settings"));
    crate::icons::paint(ui,crate::icons::Icon::Back,response.rect.shrink(6.),TEXT);
    response.clicked()
}

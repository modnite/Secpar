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
    // Secpar has one look. egui follows the system's light or dark mode and keeps a style for each,
    // so with Windows in light mode the default light style (white fields and buttons) took over.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
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
/// A square button showing one Lucide glyph, named on hover and to screen readers.
pub fn icon_button(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str) -> egui::Response {
    let response = ui.add(egui::Button::new("").min_size(egui::vec2(CONTROL, CONTROL))).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    crate::icons::paint(ui, icon, egui::Rect::from_center_size(response.rect.center(), egui::vec2(16., 16.)), MUTED);
    response
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

/// A titled settings group: one surface, its rows inside, full width.
pub fn group<R>(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    surface()
        .inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.;
            ui.label(RichText::new(title).size(12.).color(MUTED));
            ui.add_space(2.);
            contents(ui)
        })
        .inner
}

/// How wide the value of a settings row is: about half the row, within limits.
pub fn value_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() * 0.5).clamp(120., 220.)
}

/// One settings row: the label at the left, its control at the right edge.
pub fn row<R>(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    let value = value_width(ui);
    ui.allocate_ui_with_layout(egui::vec2(width, CONTROL + 8.), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.allocate_ui_with_layout(egui::vec2((width - value - 8.).max(60.), CONTROL), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(label).truncate());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), contents).inner
    })
    .inner
}

/// An on and off switch. Returns true when it was flipped.
pub fn switch(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(34., 28.), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let on = *value;
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), on, label));
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(32., 18.));
    ui.painter().rect_filled(track, 9, if on { ACCENT } else { Color32::from_rgb(62, 73, 87) });
    let x = if on { track.right() - 9. } else { track.left() + 9. };
    ui.painter().circle_filled(egui::pos2(x, track.center().y), 6., if on { Color32::from_rgb(7, 38, 30) } else { Color32::from_rgb(205, 216, 226) });
    response.changed()
}

/// A row whose value is a switch.
pub fn switch_row(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    row(ui, label, |ui| switch(ui, value, label))
}

/// A segmented control: one bordered strip, equal parts, the chosen one filled.
/// Returns the new choice when one is clicked.
pub fn segmented(ui: &mut egui::Ui, options: &[&str], chosen: usize) -> Option<usize> {
    let width = ui.available_width();
    let (outer, _) = ui.allocate_exact_size(egui::vec2(width, CONTROL + 4.), egui::Sense::hover());
    ui.painter().rect(outer, 7, Color32::from_rgb(23, 29, 39), Stroke::new(1., LINE), egui::StrokeKind::Inside);
    let inner = outer.shrink(2.);
    let part = inner.width() / options.len() as f32;
    let mut picked = None;
    for (index, name) in options.iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(inner.left() + part * index as f32, inner.top()), egui::vec2(part, inner.height()));
        let response = ui.interact(rect, ui.id().with(("segment", index)), egui::Sense::click());
        let on = index == chosen;
        if on {
            ui.painter().rect_filled(rect, 5, Color32::from_rgb(37, 45, 58));
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 5, Color32::from_rgb(29, 36, 47));
        }
        let size = if ui.fonts_mut(|f| f.layout_no_wrap((*name).into(), egui::FontId::proportional(13.), TEXT).size().x) > part - 8. { 12. } else { 13. };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, *name, egui::FontId::proportional(size), if on { ACCENT } else { MUTED });
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, *name));
        if response.clicked() && !on {
            picked = Some(index);
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_dark_look_stays_when_the_system_is_in_light_mode() {
        let ctx = eframe::egui::Context::default();
        super::configure(&ctx);
        // Windows in light mode: egui is told the system theme is light.
        ctx.set_theme(eframe::egui::Theme::Light);
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(ctx.style().visuals.widgets.inactive.bg_fill, eframe::egui::Color32::from_rgb(31, 39, 51));
    }
}

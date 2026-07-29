//! Centralized visual styling so the whole app reads as one deliberate
//! product instead of default-egui-gray. Call `apply` once per frame (cheap)
//! right after handling theme toggles.

use eframe::egui::{self, Color32, FontId, Rounding, Stroke};

pub const ACCENT: Color32 = Color32::from_rgb(0, 122, 204); // professional blue
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(30, 144, 220);
pub const SUCCESS: Color32 = Color32::from_rgb(46, 160, 67);
pub const DANGER: Color32 = Color32::from_rgb(217, 61, 61);
pub const WARNING: Color32 = Color32::from_rgb(224, 152, 20);

pub fn apply(ctx: &egui::Context, dark_mode: bool) {
    let mut visuals = if dark_mode {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let rounding = Rounding::same(6.0);
    visuals.window_rounding = Rounding::same(10.0);
    visuals.menu_rounding = Rounding::same(8.0);
    visuals.widgets.noninteractive.rounding = rounding;
    visuals.widgets.inactive.rounding = rounding;
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.active.rounding = rounding;
    visuals.widgets.open.rounding = rounding;

    visuals.selection.bg_fill = ACCENT;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.bg_fill = ACCENT_HOVER;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(16.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(14.0);
    style.spacing.indent = 18.0;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(21.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.5));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.5));
    ctx.set_style(style);
}

/// A card-like frame used to group a section of the UI (Connection, Live
/// Value, Wizard step, etc.) so the layout reads as distinct panels rather
/// than one flat scroll of widgets.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    let visuals = ui.visuals();
    egui::Frame::none()
        .fill(visuals.faint_bg_color)
        .stroke(Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color))
        .rounding(Rounding::same(8.0))
        .inner_margin(egui::Margin::same(14.0))
}

/// Solid accent-colored primary action button (Connect, Start, Write, Save).
pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong())
            .fill(ACCENT)
            .rounding(Rounding::same(6.0))
            .min_size(egui::vec2(0.0, 34.0)),
    )
}

/// Red destructive button (Disconnect, Abort, Reset to Defaults).
pub fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Color32::WHITE).strong())
            .fill(DANGER)
            .rounding(Rounding::same(6.0))
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

/// A small colored status dot + label, used for connection status.
pub fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 5.0, color);
        ui.label(text);
    });
}
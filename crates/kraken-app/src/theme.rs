use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals};

pub const SPACE_XS: f32 = 8.0;
pub const SPACE_S: f32 = 12.0;
pub const SPACE_M: f32 = 16.0;
pub const SPACE_L: f32 = 24.0;
pub const RADIUS: u8 = 10;
pub const RAIL_WIDTH: f32 = 64.0;
pub const TOP_BAR_HEIGHT: f32 = 56.0;

pub const ABYSS: Color32 = Color32::from_rgb(9, 17, 26);
pub const DEEP: Color32 = Color32::from_rgb(13, 25, 37);
pub const SURFACE: Color32 = Color32::from_rgb(19, 34, 49);
pub const RAISED: Color32 = Color32::from_rgb(27, 45, 63);
pub const LINE: Color32 = Color32::from_rgb(36, 58, 79);
pub const TEXT: Color32 = Color32::from_rgb(222, 232, 240);
pub const MUTED: Color32 = Color32::from_rgb(132, 154, 173);
pub const ACCENT: Color32 = Color32::from_rgb(38, 198, 186);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(22, 92, 92);
pub const SUCCESS: Color32 = Color32::from_rgb(92, 204, 120);
pub const WARNING: Color32 = Color32::from_rgb(240, 181, 74);
pub const DANGER: Color32 = Color32::from_rgb(235, 96, 96);
pub const NEARBY: Color32 = Color32::from_rgb(110, 184, 255);

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = DEEP;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = ABYSS;
    visuals.faint_bg_color = RAISED;
    visuals.window_corner_radius = CornerRadius::same(RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    for (widget, fill) in [
        (&mut visuals.widgets.noninteractive, SURFACE),
        (&mut visuals.widgets.inactive, RAISED),
        (&mut visuals.widgets.hovered, LINE),
        (&mut visuals.widgets.active, ACCENT_DIM),
        (&mut visuals.widgets.open, RAISED),
    ] {
        widget.corner_radius = CornerRadius::same(6);
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);

    ctx.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.item_spacing = egui::vec2(SPACE_XS, SPACE_XS);
        style.spacing.button_padding = egui::vec2(SPACE_S, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.window_margin = Margin::same(SPACE_M as i8);
        style.text_styles = [
            (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
    });
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(SPACE_M as i8))
}

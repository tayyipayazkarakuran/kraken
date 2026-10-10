use eframe::egui::{self, Color32, CornerRadius, Response, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::theme;

pub const WIDE: f32 = 760.0;

pub fn card<R>(ui: &mut Ui, icon: &str, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    theme::card_frame()
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(18.0).color(theme::ACCENT));
                ui.label(RichText::new(title).size(16.0).strong());
            });
            ui.add_space(theme::SPACE_XS);
            add(ui)
        })
        .inner
}

pub fn page_title(ui: &mut Ui, icon: &str, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(26.0).color(theme::ACCENT));
        ui.vertical(|ui| {
            ui.label(RichText::new(title).heading().strong());
            ui.label(RichText::new(subtitle).color(theme::MUTED));
        });
    });
    ui.add_space(theme::SPACE_M);
}

pub fn columns(ui: &mut Ui, count: usize, mut add: impl FnMut(usize, &mut Ui)) {
    let gap = theme::SPACE_M;
    let width = ui.available_width();
    let fits = ((width + gap) / (WIDE / 2.0 + gap)).floor().max(1.0) as usize;
    let per_row = count.min(fits).max(1);
    let column = (width - gap * (per_row as f32 - 1.0)) / per_row as f32;
    for row in (0..count).collect::<Vec<_>>().chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &index in row {
                ui.allocate_ui_with_layout(
                    vec2(column, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(column);
                        ui.set_max_width(column);
                        add(index, ui);
                    },
                );
            }
        });
        ui.add_space(gap);
    }
}

/// Cards in balanced columns: each card goes under the shortest column so
/// far (by the heights measured last frame), keeping the given order when
/// the panel only has room for one column.
pub fn masonry(ui: &mut Ui, id_salt: &str, count: usize, mut add: impl FnMut(usize, &mut Ui)) {
    let gap = theme::SPACE_M;
    let width = ui.available_width();
    let fits = ((width + gap) / (WIDE / 2.0 + gap)).floor().max(1.0) as usize;
    let per_row = count.min(fits).max(1);
    if per_row == 1 {
        for index in 0..count {
            add(index, ui);
            ui.add_space(gap);
        }
        return;
    }
    let id = ui.id().with(("masonry", id_salt));
    let mut heights: Vec<f32> = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    heights.resize(count, 0.0);
    let mut stacks = vec![Vec::new(); per_row];
    let mut filled = vec![0.0f32; per_row];
    for (index, height) in heights.iter().enumerate() {
        let shortest = (0..per_row)
            .min_by(|a, b| filled[*a].total_cmp(&filled[*b]))
            .unwrap_or(0);
        stacks[shortest].push(index);
        filled[shortest] += height + gap;
    }
    let column = (width - gap * (per_row as f32 - 1.0)) / per_row as f32;
    let mut measured = heights.clone();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for stack in &stacks {
            ui.allocate_ui_with_layout(
                vec2(column, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(column);
                    ui.set_max_width(column);
                    for (n, &index) in stack.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(gap);
                        }
                        measured[index] = ui.scope(|ui| add(index, ui)).response.rect.height();
                    }
                },
            );
        }
    });
    ui.add_space(gap);
    if measured.iter().zip(&heights).any(|(a, b)| (a - b).abs() > 1.0) {
        ui.data_mut(|d| d.insert_temp(id, measured));
        ui.ctx().request_repaint();
    }
}

pub fn stat_tile(ui: &mut Ui, icon: &str, label: &str, value: &str, tint: Color32) {
    egui::Frame::new()
        .fill(theme::RAISED)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(theme::SPACE_S as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(22.0).color(tint));
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).small().color(theme::MUTED));
                    ui.label(RichText::new(value).size(17.0).strong());
                });
            });
        });
}

pub fn bar(ui: &mut Ui, fraction: f32, tint: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), theme::ABYSS);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    painter.rect_filled(fill, CornerRadius::same(3), tint);
}

pub fn hp_tint(percent: f32) -> Color32 {
    if percent >= 60.0 {
        theme::SUCCESS
    } else if percent >= 30.0 {
        theme::WARNING
    } else {
        theme::DANGER
    }
}

pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(2.0, 1.0) * 20.0;
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let fill = if *on { theme::ACCENT_DIM } else { theme::ABYSS };
        let stroke = if *on { theme::ACCENT } else { theme::LINE };
        let radius = rect.height() / 2.0;
        ui.painter().rect(
            rect,
            CornerRadius::same(radius as u8),
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        let knob = if *on { theme::ACCENT } else { theme::MUTED };
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), radius * 0.72, knob);
    }
    response
}

pub fn toggle_row(ui: &mut Ui, label: &str, hint: &str, on: &mut bool) -> Response {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 56.0).max(0.0));
            ui.label(label);
            if !hint.is_empty() {
                ui.label(RichText::new(hint).small().color(theme::MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            switch(ui, on)
        })
        .inner
    })
    .inner
}

/// Width kept free for the control at the right of a field row (a slider
/// with its value box, a combo box).
const FIELD_CONTROL: f32 = 190.0;

/// A label on the left, a control on the right. The label gets what the
/// control leaves and is cut with "…" (full text on hover); in a narrow card
/// the control goes under the label instead.
pub fn field_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    if width < FIELD_CONTROL + 110.0 {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
        return;
    }
    ui.horizontal(|ui| {
        let label_width = (width - FIELD_CONTROL).max(width * 0.4);
        let height = ui.spacing().interact_size.y;
        ui.allocate_ui_with_layout(
            vec2(label_width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(label_width);
                ui.add(egui::Label::new(label).truncate()).on_hover_text(label);
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
    });
}

pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    options: &[T],
    label: impl Fn(T) -> String,
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for &option in options {
            let selected = *value == option;
            let text = RichText::new(label(option)).color(if selected { theme::TEXT } else { theme::MUTED });
            let button = egui::Button::new(text)
                .fill(if selected {
                    theme::ACCENT_DIM
                } else {
                    theme::RAISED
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected { theme::ACCENT } else { theme::LINE },
                ));
            if ui.add(button).clicked() && !selected {
                *value = option;
                changed = true;
            }
        }
    });
    changed
}

pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, tint: Color32, enabled: bool) -> Response {
    let button =
        egui::Button::new(
            RichText::new(icon)
                .size(18.0)
                .color(if enabled { tint } else { theme::MUTED }),
        )
        .min_size(vec2(36.0, 32.0))
        .fill(theme::RAISED);
    ui.add_enabled(enabled, button).on_hover_text(tip)
}

pub fn pill(ui: &mut Ui, icon: &str, text: &str, tint: Color32) -> Response {
    pill_fit(ui, icon, text, tint, f32::INFINITY)
}

/// A pill no wider than `max_width` (and never wider than the room left);
/// a longer text is cut with "…" and shown in full on hover.
pub fn pill_fit(ui: &mut Ui, icon: &str, text: &str, tint: Color32, max_width: f32) -> Response {
    let max_width = max_width.min(ui.available_width()).max(32.0);
    let response = egui::Frame::new()
        .fill(theme::RAISED)
        .stroke(Stroke::new(1.0, tint.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_max_width(max_width - 18.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(icon).color(tint));
                if !text.is_empty() {
                    ui.add(egui::Label::new(RichText::new(text).small().color(theme::TEXT)).truncate());
                }
            });
        })
        .response;
    if text.is_empty() {
        response
    } else {
        response.on_hover_text(text)
    }
}

/// A status pill showing only its icon; the text is the tooltip.
pub fn icon_pill(ui: &mut Ui, icon: &str, tip: &str, tint: Color32) {
    pill(ui, icon, "", tint).on_hover_text(tip);
}

/// Cuts `text` to `max` characters with an ellipsis.
pub fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        format!(
            "{}…",
            text.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

pub fn muted(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::MUTED));
}

/// A table column: a fixed width, or the share of what is left.
#[derive(Clone, Copy, Debug)]
pub enum Col {
    Fill,
    Fixed(f32),
}

/// One layout for every list in the interface: rows span the card's full
/// width, columns keep their widths from row to row and page to page, and
/// the row actions sit at the right end.
pub struct Table {
    widths: Vec<f32>,
    height: f32,
}

pub struct Cells<'a> {
    ui: &'a mut Ui,
    widths: &'a [f32],
    next: usize,
    height: f32,
}

impl Cells<'_> {
    fn take(&mut self) -> f32 {
        let width = self.widths.get(self.next).copied().unwrap_or(0.0);
        self.next += 1;
        width
    }

    fn cell_with<R>(&mut self, layout: egui::Layout, add: impl FnOnce(&mut Ui) -> R) -> R {
        let width = self.take();
        let height = self.height;
        self.ui
            .allocate_ui_with_layout(vec2(width, height), layout, |ui| {
                ui.set_width(width);
                ui.set_max_width(width);
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                add(ui)
            })
            .inner
    }

    /// A left-aligned cell.
    pub fn cell<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.cell_with(egui::Layout::left_to_right(egui::Align::Center), add)
    }

    /// A right-aligned cell (numbers, row actions).
    pub fn right<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.cell_with(egui::Layout::right_to_left(egui::Align::Center), add)
    }

    /// A text cell cut with "…" and shown in full on hover.
    pub fn text(&mut self, text: impl Into<egui::WidgetText>) {
        let text = text.into();
        self.cell(|ui| {
            let full = text.text().to_owned();
            ui.add(egui::Label::new(text).truncate()).on_hover_text(full);
        });
    }
}

impl Table {
    pub const GAP: f32 = 8.0;
    const FILL_MIN: f32 = 80.0;
    const MIN_SCALE: f32 = 0.6;

    pub fn new(ui: &Ui, cols: &[Col]) -> Self {
        let total = ui.available_width();
        let fixed: f32 = cols
            .iter()
            .map(|c| match c {
                Col::Fixed(w) => *w,
                Col::Fill => 0.0,
            })
            .sum();
        let fills = cols.iter().filter(|c| matches!(c, Col::Fill)).count() as f32;
        let gaps = Self::GAP * (cols.len().saturating_sub(1) as f32 + 2.0);
        // In a narrow card the fixed columns give way first (down to a share
        // of their width), so the row still fits; the cells clip the rest.
        let fill_min = Self::FILL_MIN * fills;
        let scale = if fixed > 0.0 {
            ((total - gaps - fill_min) / fixed).clamp(Self::MIN_SCALE, 1.0)
        } else {
            1.0
        };
        let fill = ((total - fixed * scale - gaps) / fills.max(1.0)).max(40.0);
        Self {
            widths: cols
                .iter()
                .map(|c| match c {
                    Col::Fixed(w) => (*w * scale).floor(),
                    Col::Fill => fill,
                })
                .collect(),
            height: ui.spacing().interact_size.y + 6.0,
        }
    }

    fn line<R>(&self, ui: &mut Ui, fill: Option<Color32>, add: impl FnOnce(&mut Cells<'_>) -> R) -> R {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, self.height), Sense::hover());
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(vec2(Self::GAP, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = Self::GAP;
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let mut cells = Cells {
            ui: &mut child,
            widths: &self.widths,
            next: 0,
            height: self.height,
        };
        add(&mut cells)
    }

    pub fn header(&self, ui: &mut Ui, labels: &[&str]) {
        self.line(ui, None, |cells| {
            for (index, label) in labels.iter().enumerate() {
                let text = RichText::new(*label).small().color(theme::MUTED);
                if index > 0 && label.chars().all(|c| !c.is_alphabetic()) {
                    cells.right(|ui| ui.label(text));
                } else {
                    cells.text(text);
                }
            }
        });
    }

    pub fn row<R>(&self, ui: &mut Ui, index: usize, add: impl FnOnce(&mut Cells<'_>) -> R) -> R {
        let fill = index.is_multiple_of(2).then_some(theme::RAISED);
        let result = self.line(ui, fill, add);
        ui.add_space(2.0);
        result
    }
}

/// Status chips in an even grid, in the given order.
pub fn chip_grid(ui: &mut Ui, chips: &[(String, bool)]) {
    let width = ui.available_width();
    let columns = if width >= 420.0 { 3 } else { 2 };
    let cell = (width - Table::GAP * (columns as f32 - 1.0)) / columns as f32;
    for row in chips.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = Table::GAP;
            for (text, ok) in row {
                let tint = if *ok { theme::SUCCESS } else { theme::DANGER };
                let glyph = if *ok {
                    egui_phosphor::regular::CHECK_CIRCLE
                } else {
                    egui_phosphor::regular::X_CIRCLE
                };
                ui.allocate_ui_with_layout(
                    vec2(cell, 26.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        egui::Frame::new()
                            .fill(theme::RAISED)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.set_width(cell - 16.0);
                                ui.label(RichText::new(glyph).color(tint));
                                ui.add(egui::Label::new(RichText::new(text).small()).truncate());
                            });
                    },
                );
            }
        });
        ui.add_space(4.0);
    }
}

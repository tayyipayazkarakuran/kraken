//! The always-visible part of the in-game interface: a status card in the
//! top-right corner (with the panel button) and the markers on the sea.

use eframe::egui::{self, Color32, CornerRadius, Pos2, RichText, Stroke, pos2};
use egui_phosphor::regular as icon;
use kraken_proto::Telemetry;
use rust_i18n::t;

use super::{State, stats, widgets};
use crate::{i18n, theme};

const HUD_WIDTH: f32 = 250.0;
const MARGIN: f32 = 12.0;
/// Wallet lines shown on the card.
const HUD_GAINS: usize = 4;

/// Draws the card; returns `true` when its panel button was clicked.
pub fn card(ctx: &egui::Context, state: &State, panel_open: bool) -> bool {
    let mut toggle = false;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("kraken-hud"))
        .fixed_pos(pos2(screen.max.x - HUD_WIDTH - MARGIN, screen.min.y + MARGIN))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(13, 25, 37, 225))
                .stroke(Stroke::new(1.0, theme::LINE))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(HUD_WIDTH - 20.0);
                    let live = state.live();
                    // The button first, from the right; the status gets the rest.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let glyph = if panel_open {
                                icon::X
                            } else {
                                icon::SLIDERS_HORIZONTAL
                            };
                            if ui
                                .add(egui::Button::new(RichText::new(glyph).size(14.0)).fill(theme::RAISED))
                                .on_hover_text(t!("hud.toggle"))
                                .clicked()
                            {
                                toggle = true;
                            }
                            let activity =
                                live.map_or_else(|| i18n::link(state.link), |l| i18n::activity(l.activity));
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                let text = format!("{} KRAKEN · {activity}", icon::ANCHOR);
                                ui.add(
                                    egui::Label::new(RichText::new(&text).size(13.0).color(theme::ACCENT))
                                        .truncate(),
                                )
                                .on_hover_text(text);
                            });
                        });
                    });
                    if let Some(live) = live {
                        details(ui, state, live);
                    }
                });
        });
    toggle
}

fn details(ui: &mut egui::Ui, state: &State, live: &Telemetry) {
    let target = live
        .target
        .as_ref()
        .map_or_else(|| "—".to_owned(), |t| format!("{} · {:.0}", t.name, t.distance));
    ui.add(egui::Label::new(RichText::new(format!("{} {target}", icon::TARGET)).size(12.5)).truncate());
    let hp = live.hero.as_ref().map_or(0.0, kraken_proto::Hero::hp_percent);
    widgets::bar(ui, hp / 100.0, widgets::hp_tint(hp));
    let counters = live.counters;
    ui.label(
        RichText::new(format!(
            "{} {}  {} {}  {} {}  {} {}",
            icon::CROSSHAIR,
            counters.npc_kills,
            icon::FISH,
            counters.monster_kills,
            icon::PACKAGE,
            counters.boxes,
            icon::TIMER,
            i18n::duration(counters.run_secs)
        ))
        .size(12.0)
        .color(theme::MUTED),
    );
    if let Some(info) = live
        .eldorado
        .filter(|i| i.available && state.profile.config.eldorado.enabled)
    {
        let tint = if info.phase == kraken_proto::ElDoradoPhase::Run || info.charge >= 100 {
            theme::SUCCESS
        } else {
            theme::MUTED
        };
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} {} · {}",
                    icon::CROWN,
                    t!("eldorado.title"),
                    i18n::eldorado_status(&info)
                ))
                .size(12.0)
                .color(tint),
            )
            .truncate(),
        );
    }
    if let Some(info) = live
        .bonus
        .as_ref()
        .filter(|i| i.phase != kraken_proto::BonusPhase::Waiting)
    {
        let status = i18n::bonus_status(info, |id| super::bonus::map_name(state, id));
        ui.add(
            egui::Label::new(
                RichText::new(format!("{} {status}", icon::MAP_TRIFOLD))
                    .size(12.0)
                    .color(theme::SUCCESS),
            )
            .truncate(),
        );
    }
    if let Some(info) = live
        .workmap
        .filter(|i| i.phase != kraken_proto::WorkmapPhase::Idle)
    {
        let status = super::workmap::status(&info, |id| super::workmap::map_name(state, id));
        ui.add(
            egui::Label::new(
                RichText::new(format!("{} {status}", icon::GLOBE))
                    .size(12.0)
                    .color(theme::ACCENT),
            )
            .truncate(),
        );
    }
    let gains: Vec<_> = state
        .stats
        .gains()
        .into_iter()
        .filter(|g| g.delta() > 0)
        .take(HUD_GAINS)
        .collect();
    if gains.is_empty() {
        return;
    }
    ui.separator();
    for gain in gains {
        // The amount first, from the right; the name is cut to what is left.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(stats::signed(gain.delta()))
                        .size(12.0)
                        .color(theme::SUCCESS)
                        .strong(),
                );
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(stats::gain_name(state, &gain))
                                .size(12.0)
                                .color(theme::TEXT),
                        )
                        .truncate(),
                    );
                });
            });
        });
    }
}

/// Waypoint line, target rings and the range circle, behind everything.
pub fn markers(ctx: &egui::Context, state: &State, scale: f32) {
    let settings = &state.profile.overlay;
    let Some(live) = state.live() else { return };
    let Some(screen) = &live.screen else { return };
    if !settings.markers {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::background());
    let map = |p: kraken_proto::Point| pos2(p.x / scale, p.y / scale);
    if settings.range_ring && screen.range_ring.len() > 2 {
        let points: Vec<Pos2> = screen.range_ring.iter().copied().map(map).collect();
        painter.add(egui::Shape::closed_line(
            points,
            Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.55)),
        ));
    }
    let hero = screen.hero.map(map);
    if let (Some(hero), Some(waypoint)) = (hero, screen.waypoint.map(map)) {
        painter.line_segment(
            [hero, waypoint],
            Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.7)),
        );
        painter.circle(
            waypoint,
            5.0,
            theme::ACCENT.gamma_multiply(0.4),
            Stroke::new(1.5, theme::ACCENT),
        );
    }
    if let Some(target) = screen.target.map(map) {
        painter.circle_stroke(target, 22.0, Stroke::new(2.0, theme::DANGER));
        painter.circle_stroke(target, 28.0, Stroke::new(1.0, theme::DANGER.gamma_multiply(0.5)));
    }
}

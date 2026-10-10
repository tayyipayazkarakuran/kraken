use eframe::egui::{self, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::names::NameKind;
use kraken_proto::{BoxTarget, ElDoradoInfo};
use rust_i18n::t;

use super::npc::{TARGET_COLUMNS, entry_text, picker, priority, remove_button};
use super::widgets::Table;
use super::{State, names, widgets};
use crate::{i18n, theme};

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(
        ui,
        icon::PACKAGE,
        &t!("page.collector"),
        &t!("collector.subtitle"),
    );
    eldorado(ui, state);
    let entries = names::entries(state, NameKind::Box, &state.catalog.boxes);
    let targets: Vec<(i32, String)> = state
        .profile
        .config
        .collector
        .boxes
        .iter()
        .map(|t| (t.type_id, t.name.clone()))
        .collect();
    let rows = names::order(state, NameKind::Box, &targets);
    let collector = &mut state.profile.config.collector;
    widgets::columns(ui, 2, |index, ui| match index {
        0 => widgets::card(ui, icon::PACKAGE, &t!("collector.behaviour"), |ui| {
            widgets::toggle_row(
                ui,
                &t!("collector.enabled"),
                &t!("collector.enabled_hint"),
                &mut collector.enabled,
            );
            widgets::toggle_row(
                ui,
                &t!("collector.collect_all"),
                &t!("collector.collect_all_hint"),
                &mut collector.collect_all,
            );
        }),
        _ => widgets::card(ui, icon::COMPASS, &t!("collector.detour"), |ui| {
            widgets::toggle_row(
                ui,
                &t!("collector.during_combat"),
                &t!("collector.during_combat_hint"),
                &mut collector.during_combat,
            );
            ui.add_enabled_ui(collector.during_combat, |ui| {
                widgets::field_row(ui, &t!("collector.radius"), |ui| {
                    ui.add(egui::Slider::new(&mut collector.detour_radius, 0..=40))
                        .on_hover_text(t!("collector.radius_auto"));
                });
                widgets::field_row(ui, &t!("collector.duration"), |ui| {
                    ui.add(egui::Slider::new(&mut collector.detour_secs, 2..=30).suffix(" s"));
                });
            });
        }),
    });
    widgets::card(
        ui,
        icon::TARGET,
        &t!("collector.boxes", count = collector.boxes.len()),
        |ui| {
            let taken: Vec<i32> = collector.boxes.iter().map(|b| b.type_id).collect();
            if let Some(entry) = picker(ui, "box-picker", &mut state.search.boxes, &entries, &taken) {
                collector.boxes.push(BoxTarget {
                    type_id: entry.id,
                    name: entry.name,
                    priority: 5,
                });
            }
            if entries.is_empty() {
                widgets::muted(ui, &t!("target.catalog_empty"));
            }
            ui.add_space(theme::SPACE_XS);
            let mut remove = None;
            let table = Table::new(ui, &TARGET_COLUMNS);
            for (row, (index, entry)) in rows.iter().enumerate() {
                let index = *index;
                let Some(target) = collector.boxes.get_mut(index) else {
                    continue;
                };
                table.row(ui, row, |cells| {
                    cells.text(entry_text(entry));
                    cells.cell(|ui| priority(ui, &mut target.priority));
                    cells.cell(|_| {});
                    cells.cell(|_| {});
                    cells.right(|ui| {
                        if remove_button(ui) {
                            remove = Some(index);
                        }
                    });
                });
            }
            if let Some(index) = remove {
                collector.boxes.remove(index);
            }
            if collector.boxes.is_empty() {
                widgets::muted(ui, &t!("target.none"));
            }
        },
    );
}

/// The El Dorado card: live charge and phase, and the automation switches.
fn eldorado(ui: &mut Ui, state: &mut State) {
    let info: Option<ElDoradoInfo> = state.live().and_then(|l| l.eldorado);
    let cfg = &mut state.profile.config.eldorado;
    widgets::card(ui, icon::CROWN, &t!("eldorado.title"), |ui| {
        match info {
            None => widgets::muted(ui, &t!("eldorado.unknown")),
            Some(info) if !info.available => widgets::muted(ui, &t!("eldorado.unavailable")),
            Some(info) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(i18n::eldorado_status(&info)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        widgets::muted(ui, &t!("eldorado.runs", runs = info.runs, chests = info.chests));
                    });
                });
                let tint = if info.charge >= 100 {
                    theme::SUCCESS
                } else {
                    theme::ACCENT
                };
                widgets::bar(ui, f32::from(info.charge) / 100.0, tint);
            }
        }
        ui.add_space(theme::SPACE_XS);
        widgets::columns(ui, 2, |index, ui| {
            if index == 0 {
                widgets::toggle_row(
                    ui,
                    &t!("eldorado.enabled"),
                    &t!("eldorado.enabled_hint"),
                    &mut cfg.enabled,
                );
                widgets::toggle_row(
                    ui,
                    &t!("eldorado.retreat"),
                    &t!("eldorado.retreat_hint"),
                    &mut cfg.retreat,
                );
            } else {
                widgets::toggle_row(
                    ui,
                    &t!("eldorado.swiftstone"),
                    &t!("eldorado.swiftstone_hint"),
                    &mut cfg.swiftstone,
                );
                widgets::toggle_row(
                    ui,
                    &t!("eldorado.finish"),
                    &t!("eldorado.finish_hint"),
                    &mut cfg.finish_target,
                );
            }
        });
    });
}

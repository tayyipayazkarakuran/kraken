use eframe::egui::Ui;
use egui_phosphor::regular as icon;
use kraken_proto::names::NameKind;
use kraken_proto::{MonsterTarget, StockKind};
use rust_i18n::t;

use super::npc::{TARGET_COLUMNS, entry_text, item_combo, picker, priority, remove_button, settings_button};
use super::widgets::Table;
use super::{State, names, widgets};
use crate::theme;

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::FISH, &t!("page.monsters"), &t!("monster.subtitle"));
    let stock = state.live().map(|t| t.stock.clone()).unwrap_or_default();
    let harpoons = names::items(state, StockKind::Harpoon, &state.catalog.harpoons, &stock);
    let entries = names::entries(state, NameKind::Monster, &state.catalog.monsters);
    let targets: Vec<(i32, String)> = state
        .profile
        .config
        .monster
        .targets
        .iter()
        .map(|t| (t.type_id, t.name.clone()))
        .collect();
    let rows = names::order(state, NameKind::Monster, &targets);
    let mut editing = None;
    let monster = &mut state.profile.config.monster;
    widgets::columns(ui, 2, |index, ui| match index {
        0 => widgets::card(ui, icon::SWORD, &t!("monster.behaviour"), |ui| {
            widgets::toggle_row(
                ui,
                &t!("monster.enabled"),
                &t!("monster.enabled_hint"),
                &mut monster.enabled,
            );
            widgets::toggle_row(
                ui,
                &t!("monster.attack_all"),
                &t!("monster.attack_all_hint"),
                &mut monster.attack_all,
            );
        }),
        _ => widgets::card(ui, icon::LIGHTNING, &t!("monster.harpoons"), |ui| {
            widgets::field_row(ui, &t!("monster.default_harpoon"), |ui| {
                item_combo(
                    ui,
                    "monster-default-harpoon",
                    &mut monster.default_harpoon,
                    &harpoons,
                    &stock,
                    StockKind::Harpoon,
                );
            });
            widgets::muted(ui, &t!("monster.harpoon_hint"));
        }),
    });
    widgets::card(
        ui,
        icon::TARGET,
        &t!("monster.targets", count = monster.targets.len()),
        |ui| {
            let taken: Vec<i32> = monster.targets.iter().map(|t| t.type_id).collect();
            if let Some(entry) = picker(ui, "monster-picker", &mut state.search.monster, &entries, &taken) {
                monster.targets.push(MonsterTarget {
                    type_id: entry.id,
                    name: entry.name,
                    priority: 5,
                    harpoon: None,
                    items: Vec::new(),
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
                let Some(target) = monster.targets.get_mut(index) else {
                    continue;
                };
                table.row(ui, row, |cells| {
                    cells.text(entry_text(entry));
                    cells.cell(|ui| priority(ui, &mut target.priority));
                    cells.cell(|ui| {
                        item_combo(
                            ui,
                            ("monster-harpoon", index),
                            &mut target.harpoon,
                            &harpoons,
                            &stock,
                            StockKind::Harpoon,
                        );
                    });
                    cells.cell(|_| {});
                    cells.right(|ui| {
                        if remove_button(ui) {
                            remove = Some(index);
                        }
                        if settings_button(ui, target.items.len()) {
                            editing = Some((NameKind::Monster, target.type_id));
                        }
                    });
                });
            }
            if let Some(index) = remove {
                monster.targets.remove(index);
            }
            if monster.targets.is_empty() {
                widgets::muted(ui, &t!("target.none"));
            }
        },
    );
    if editing.is_some() {
        state.search.editing = editing;
    }
    super::items::target_dialog(ui, state);
}

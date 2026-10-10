//! Bonus maps: the queue (which maps, how many, in what order) and the live
//! progress of the one being played.

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{BonusEntry, BonusInfo, StockKind};
use rust_i18n::t;

use super::names::{self, Entry};
use super::npc::{item_combo, picker_labeled, remove_button};
use super::widgets::{Col, Table};
use super::{State, widgets};
use crate::{i18n, theme};

const COLUMNS: [Col; 5] = [
    Col::Fill,
    Col::Fixed(90.0),
    Col::Fixed(150.0),
    Col::Fixed(80.0),
    Col::Fixed(84.0),
];

/// "Virgo map" and the like, from the game's own map names.
pub fn map_name(state: &State, id: i16) -> String {
    let reported = state
        .catalog
        .bonus_maps
        .iter()
        .find(|m| m.id == id)
        .map_or("", |m| m.name.as_str());
    names::name(state, kraken_proto::names::NameKind::Map, i32::from(id), reported)
}

fn owned(state: &State, id: i16) -> i64 {
    state
        .catalog
        .bonus_maps
        .iter()
        .find(|m| m.id == id)
        .map_or(0, |m| m.amount)
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::MAP_TRIFOLD, &t!("page.bonus"), &t!("bonus.subtitle"));
    let info: Option<BonusInfo> = state.live().and_then(|l| l.bonus.clone());
    let stock = state.live().map(|t| t.stock.clone()).unwrap_or_default();
    let ammo = names::items(state, StockKind::Ammo, &state.catalog.ammo, &stock);
    let status = info
        .as_ref()
        .map(|i| i18n::bonus_status(i, |id| map_name(state, id)));
    let entries: Vec<Entry> = state
        .catalog
        .bonus_maps
        .iter()
        .map(|m| Entry {
            id: i32::from(m.id),
            name: t!("bonus.owned", name = map_name(state, m.id), count = m.amount).into_owned(),
            nearby: false,
        })
        .collect();
    // By map id, not by row: the picker may add a row below before the table is drawn.
    let ids = state
        .catalog
        .bonus_maps
        .iter()
        .map(|m| m.id)
        .chain(state.profile.config.bonus.queue.iter().map(|e| e.map));
    let rows: std::collections::HashMap<i16, (String, i64, u32)> = ids
        .map(|id| {
            let done = info
                .as_ref()
                .and_then(|i| i.done.iter().find(|(m, _)| *m == id))
                .map_or(0, |(_, n)| *n);
            (id, (map_name(state, id), owned(state, id), done))
        })
        .collect();

    let cfg = &mut state.profile.config.bonus;
    widgets::card(ui, icon::MAP_TRIFOLD, &t!("bonus.settings"), |ui| {
        if let Some(status) = &status {
            ui.label(RichText::new(status).strong());
            ui.add_space(theme::SPACE_XS);
        }
        widgets::columns(ui, 2, |index, ui| {
            if index == 0 {
                widgets::toggle_row(
                    ui,
                    &t!("bonus.enabled"),
                    &t!("bonus.enabled_hint"),
                    &mut cfg.enabled,
                );
                widgets::toggle_row(
                    ui,
                    &t!("bonus.retreat"),
                    &t!("bonus.retreat_hint"),
                    &mut cfg.retreat,
                );
            } else {
                widgets::field_row(ui, &t!("bonus.ammo"), |ui| {
                    item_combo(ui, "bonus-ammo", &mut cfg.ammo, &ammo, &stock, StockKind::Ammo);
                });
                widgets::muted(ui, &t!("bonus.ammo_hint"));
            }
        });
    });

    widgets::card(
        ui,
        icon::LIST_NUMBERS,
        &t!("bonus.queue", count = cfg.queue.len()),
        |ui| {
            widgets::muted(ui, &t!("bonus.queue_hint"));
            let taken: Vec<i32> = cfg.queue.iter().map(|e| i32::from(e.map)).collect();
            let label = |count: usize| t!("bonus.pick", count = count).into_owned();
            if let Some(entry) = picker_labeled(
                ui,
                "bonus-picker",
                &mut state.search.bonus,
                &entries,
                &taken,
                label,
            ) {
                let name = entry
                    .name
                    .rsplit_once(" (")
                    .map_or(entry.name.as_str(), |(n, _)| n);
                cfg.queue.push(BonusEntry {
                    map: entry.id as i16,
                    name: name.to_owned(),
                    count: Some(1),
                });
            }
            if entries.is_empty() {
                widgets::muted(ui, &t!("bonus.none_owned"));
            }
            ui.add_space(theme::SPACE_XS);
            if cfg.queue.is_empty() {
                widgets::muted(ui, &t!("bonus.empty"));
                return;
            }
            let table = Table::new(ui, &COLUMNS);
            table.header(
                ui,
                &[
                    &t!("bonus.map"),
                    &t!("bonus.left"),
                    &t!("bonus.count"),
                    &t!("bonus.done"),
                    "",
                ],
            );
            let mut remove = None;
            let mut swap = None;
            let len = cfg.queue.len();
            for (index, entry) in cfg.queue.iter_mut().enumerate() {
                let (name, left, done) = rows
                    .get(&entry.map)
                    .cloned()
                    .unwrap_or_else(|| (entry.name.clone(), 0, 0));
                table.row(ui, index, |cells| {
                    cells.text(format!("{}. {name}", index + 1));
                    cells.text(left.to_string());
                    cells.cell(|ui| {
                        let mut all = entry.count.is_none();
                        if ui
                            .checkbox(&mut all, t!("bonus.all"))
                            .on_hover_text(t!("bonus.all_hint"))
                            .changed()
                        {
                            entry.count = if all { None } else { Some(1) };
                        }
                        if let Some(count) = entry.count.as_mut() {
                            ui.add(egui::DragValue::new(count).range(1..=999));
                        }
                    });
                    cells.text(match entry.count {
                        Some(n) => format!("{done}/{n}"),
                        None => done.to_string(),
                    });
                    cells.right(|ui| {
                        if remove_button(ui) {
                            remove = Some(index);
                        }
                        if widgets::icon_button(
                            ui,
                            icon::ARROW_DOWN,
                            &t!("bonus.down"),
                            theme::MUTED,
                            index + 1 < len,
                        )
                        .clicked()
                        {
                            swap = Some((index, index + 1));
                        }
                        if widgets::icon_button(ui, icon::ARROW_UP, &t!("bonus.up"), theme::MUTED, index > 0)
                            .clicked()
                        {
                            swap = Some((index - 1, index));
                        }
                    });
                });
            }
            if let Some((a, b)) = swap {
                cfg.queue.swap(a, b);
            }
            if let Some(index) = remove {
                cfg.queue.remove(index);
            }
        },
    );
}

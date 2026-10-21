//! Item automation: the Items tab (rules for every mode) and the rule editor
//! shared with the NPC and monster target settings.

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::names::NameKind;
use kraken_proto::{Condition, ItemRule, Scene, suggested_rules};
use rust_i18n::t;

use super::names::Entry;
use super::npc::{picker_labeled, remove_button};
use super::widgets::{Col, Table};
use super::{State, widgets};
use crate::{i18n, theme};

pub const COLUMNS: [Col; 7] = [
    Col::Fill,
    Col::Fixed(140.0),
    Col::Fixed(140.0),
    Col::Fixed(58.0),
    Col::Fixed(70.0),
    Col::Fixed(80.0),
    Col::Fixed(78.0),
];

/// Rule list with its picker. `scenes` limits the scene choice; `target`
/// names the NPC or monster a target's own rules belong to.
pub fn editor(
    ui: &mut Ui,
    id: &str,
    rules: &mut Vec<ItemRule>,
    scenes: &[Scene],
    owned: &[Entry],
    query: &mut String,
    target: bool,
) {
    let taken: Vec<i32> = Vec::new();
    let label = |count: usize| t!("items.pick", count = count).into_owned();
    if let Some(entry) = picker_labeled(ui, &format!("{id}-picker"), query, owned, &taken, label) {
        let name = entry
            .name
            .rsplit_once(" (")
            .map_or(entry.name.as_str(), |(n, _)| n)
            .to_owned();
        rules.push(ItemRule::new(entry.id as i16, name, scenes[0], Condition::None));
    }
    if owned.is_empty() {
        widgets::muted(ui, &t!("war.no_items"));
    }
    ui.add_space(theme::SPACE_XS);
    if rules.is_empty() {
        widgets::muted(ui, &t!("items.none"));
        return;
    }
    let table = Table::new(ui, &COLUMNS);
    table.header(
        ui,
        &[
            &t!("items.item"),
            &t!("items.scene"),
            &t!("items.condition"),
            "%",
            &t!("items.every"),
            &t!("items.off"),
            "",
        ],
    );
    let mut remove = None;
    for (index, rule) in rules.iter_mut().enumerate() {
        table.row(ui, index, |cells| {
            let owned_entry = owned.iter().find(|e| e.id == i32::from(rule.item));
            let name = owned_entry.map_or_else(|| rule.name.clone(), |e| e.name.clone());
            cells.text(if owned_entry.is_some() {
                RichText::new(name)
            } else {
                RichText::new(name).color(theme::MUTED)
            });
            cells.cell(|ui| {
                egui::ComboBox::from_id_salt((id, "scene", index))
                    .width(ui.available_width() - 4.0)
                    .truncate()
                    .selected_text(i18n::scene(rule.scene, target))
                    .show_ui(ui, |ui| {
                        for scene in scenes {
                            ui.selectable_value(&mut rule.scene, *scene, i18n::scene(*scene, target));
                        }
                    });
            });
            cells.cell(|ui| {
                egui::ComboBox::from_id_salt((id, "condition", index))
                    .width(ui.available_width() - 4.0)
                    .truncate()
                    .selected_text(i18n::condition(rule.condition))
                    .show_ui(ui, |ui| {
                        for kind in Condition::KINDS {
                            let selected = rule.condition.same_kind(kind);
                            if ui.selectable_label(selected, i18n::condition(kind)).clicked() && !selected {
                                rule.condition = kind;
                            }
                        }
                    });
            });
            cells.cell(|ui| match &mut rule.condition {
                Condition::HpBelow(p) | Condition::VpBelow(p) | Condition::TargetHpBelow(p) => {
                    ui.add(egui::DragValue::new(p).range(1..=99).suffix("%"));
                }
                Condition::None | Condition::Held => {}
            });
            cells.cell(|ui| {
                ui.add(
                    egui::DragValue::new(&mut rule.every_secs)
                        .range(0..=3600)
                        .suffix(" s"),
                )
                .on_hover_text(t!("items.every_hint"));
            });
            cells.cell(|ui| {
                ui.add(
                    egui::DragValue::new(&mut rule.off_delay_ms)
                        .range(0..=30_000)
                        .speed(50.0)
                        .suffix(" ms"),
                )
                .on_hover_text(t!("items.off_hint"));
            });
            cells.right(|ui| {
                if remove_button(ui) {
                    remove = Some(index);
                }
                widgets::switch(ui, &mut rule.enabled);
            });
        });
    }
    if let Some(index) = remove {
        rules.remove(index);
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::FLASK, &t!("page.items"), &t!("items.subtitle"));
    let owned = super::war::owned(state);
    let names: Vec<(i16, String)> = owned
        .iter()
        .map(|e| {
            let name = e.name.rsplit_once(" (").map_or(e.name.as_str(), |(n, _)| n);
            (e.id as i16, name.to_owned())
        })
        .collect();
    let rules = &mut state.profile.config.items;
    widgets::card(ui, icon::FLASK, &t!("items.rules", count = rules.len()), |ui| {
        widgets::muted(ui, &t!("items.hint"));
        let suggest: Vec<(i16, Scene, Condition, u16, String)> = suggested_rules()
            .into_iter()
            .filter(|(item, ..)| !rules.iter().any(|r| r.item == *item))
            .filter_map(|(item, scene, condition, every)| {
                let name = names.iter().find(|(id, _)| *id == item)?.1.clone();
                Some((item, scene, condition, every, name))
            })
            .collect();
        if !suggest.is_empty()
            && ui
                .button(format!("{} {}", icon::MAGIC_WAND, t!("items.suggest")))
                .on_hover_text(t!("items.suggest_hint"))
                .clicked()
        {
            for (item, scene, condition, every, name) in suggest {
                rules.push(ItemRule {
                    every_secs: every,
                    ..ItemRule::new(item, name, scene, condition)
                });
            }
        }
        ui.add_space(theme::SPACE_XS);
        editor(
            ui,
            "global-items",
            rules,
            &Scene::GLOBAL,
            &owned,
            &mut state.search.items,
            false,
        );
    });
}

/// The settings window of one NPC or monster target.
pub fn target_dialog(ui: &mut Ui, state: &mut State) {
    let Some((kind, type_id)) = state.search.editing else {
        return;
    };
    let owned = super::war::owned(state);
    let stock = state.live().map(|t| t.stock.clone()).unwrap_or_default();
    let ammo = super::names::items(state, kraken_proto::StockKind::Ammo, &state.catalog.ammo, &stock);
    let torpedoes = super::names::items(
        state,
        kraken_proto::StockKind::Torpedo,
        &state.catalog.torpedoes,
        &stock,
    );
    let harpoons = super::names::items(
        state,
        kraken_proto::StockKind::Harpoon,
        &state.catalog.harpoons,
        &stock,
    );
    let title_name = super::names::name(state, kind, type_id, "");
    let mut open = true;
    // Centred on the panel, and never wider than it.
    let area = ui.clip_rect();
    let width = (area.width() - 48.0).clamp(240.0, 860.0);
    let config = &mut state.profile.config;
    let query = &mut state.search.items_target;
    egui::Window::new(t!("target.settings_title", name = title_name))
        .id(egui::Id::new(("target-settings", type_id)))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .pivot(egui::Align2::CENTER_CENTER)
        .fixed_pos(area.center())
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            match kind {
                NameKind::Npc => {
                    let Some(target) = config.npc.targets.iter_mut().find(|t| t.type_id == type_id) else {
                        return;
                    };
                    widgets::field_row(ui, &t!("target.priority"), |ui| {
                        super::npc::priority(ui, &mut target.priority);
                    });
                    widgets::field_row(ui, &t!("target.ammo"), |ui| {
                        super::npc::item_combo(
                            ui,
                            ("dialog-ammo", type_id),
                            &mut target.ammo,
                            &ammo,
                            &stock,
                            kraken_proto::StockKind::Ammo,
                        );
                    });
                    widgets::field_row(ui, &t!("target.torpedo"), |ui| {
                        super::npc::item_combo(
                            ui,
                            ("dialog-torpedo", type_id),
                            &mut target.torpedo,
                            &torpedoes,
                            &stock,
                            kraken_proto::StockKind::Torpedo,
                        );
                    });
                    widgets::field_row(ui, &t!("target.range"), |ui| {
                        egui::ComboBox::from_id_salt(("dialog-range", type_id))
                            .selected_text(i18n::range(target.range))
                            .show_ui(ui, |ui| {
                                for style in kraken_proto::RangeStyle::ALL {
                                    ui.selectable_value(&mut target.range, style, i18n::range(style));
                                }
                            });
                    });
                    ui.separator();
                    ui.label(RichText::new(t!("target.item_rules")).strong());
                    editor(
                        ui,
                        "npc-items",
                        &mut target.items,
                        &Scene::NPC,
                        &owned,
                        query,
                        true,
                    );
                }
                NameKind::Monster => {
                    let Some(target) = config.monster.targets.iter_mut().find(|t| t.type_id == type_id)
                    else {
                        return;
                    };
                    widgets::field_row(ui, &t!("target.priority"), |ui| {
                        super::npc::priority(ui, &mut target.priority);
                    });
                    widgets::field_row(ui, &t!("target.harpoon"), |ui| {
                        super::npc::item_combo(
                            ui,
                            ("dialog-harpoon", type_id),
                            &mut target.harpoon,
                            &harpoons,
                            &stock,
                            kraken_proto::StockKind::Harpoon,
                        );
                    });
                    ui.separator();
                    ui.label(RichText::new(t!("target.item_rules")).strong());
                    editor(
                        ui,
                        "monster-items",
                        &mut target.items,
                        &Scene::MONSTER,
                        &owned,
                        query,
                        true,
                    );
                }
                _ => {}
            }
        });
    if !open {
        state.search.editing = None;
    }
}

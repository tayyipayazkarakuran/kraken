use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::names::NameKind;
use kraken_proto::{
    EnemyScope, ITEM_HAILSTORM, ITEM_UNSTOPPABLE, ITEM_WINDSTORM, ItemWhen, WarConfig, WarItemRule, WorkMode,
};
use rust_i18n::t;

use super::names::{self, Entry};
use super::npc::{picker, remove_button};
use super::widgets::{Col, Table};
use super::{State, widgets};
use crate::{i18n, theme};

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::SWORD, &t!("page.war"), &t!("war.subtitle"));
    status(ui, state);
    let items = owned(state);
    widgets::columns(ui, 2, |index, ui| match index {
        0 => enemies(ui, &mut state.profile.config.war),
        _ => friends(ui, &mut state.profile.config.war, &mut state.search.friend),
    });
    rules(ui, state, &items);
}

/// Owned action items with their amounts.
/// War item rules: item, condition, threshold, off delay, actions.
const RULE_COLUMNS: [Col; 5] = [
    Col::Fill,
    Col::Fixed(200.0),
    Col::Fixed(70.0),
    Col::Fixed(88.0),
    Col::Fixed(82.0),
];

pub(super) fn owned(state: &State) -> Vec<Entry> {
    let mut list: Vec<Entry> = state
        .catalog
        .items
        .iter()
        .map(|item| Entry {
            id: i32::from(item.id),
            name: format!(
                "{} ({})",
                names::name(state, NameKind::ActionItem, i32::from(item.id), &item.name),
                item.amount
            ),
            nearby: false,
        })
        .collect();
    names::sort(&mut list);
    list
}

fn status(ui: &mut Ui, state: &mut State) {
    let mode = state.profile.config.mode;
    widgets::card(ui, icon::CROSSHAIR_SIMPLE, &t!("war.status"), |ui| {
        if mode != WorkMode::War {
            ui.horizontal_wrapped(|ui| {
                widgets::muted(ui, &t!("war.mode_off"));
                if ui
                    .button(format!("{} {}", icon::SWORD, t!("war.enable")))
                    .clicked()
                {
                    state.profile.config.mode = WorkMode::War;
                }
            });
            return;
        }
        let Some(live) = state.live() else {
            widgets::muted(ui, &t!("dashboard.waiting_hint"));
            return;
        };
        if let Some(hero) = &live.hero
            && let Some(vp) = hero.vp_percent()
        {
            widgets::field_row(ui, &t!("stat.vp"), |ui| {
                ui.label(format!("{vp:.0}%"));
            });
            widgets::bar(ui, vp / 100.0, theme::ACCENT);
        }
        let Some(war) = &live.war else {
            return;
        };
        widgets::field_row(ui, &t!("war.target"), |ui| {
            let text = war.target.as_ref().map_or_else(
                || "—".to_owned(),
                |name| {
                    let lock = if war.locked {
                        format!(" {}", icon::LOCK_SIMPLE)
                    } else {
                        String::new()
                    };
                    format!("{name} · {:.0}{lock}", war.distance)
                },
            );
            ui.label(RichText::new(text).color(theme::WARNING));
        });
        widgets::field_row(ui, &t!("war.enemies"), |ui| {
            ui.label(war.enemies.to_string());
        });
        let active: Vec<String> = war
            .active_items
            .iter()
            .map(|id| {
                let fallback = state
                    .catalog
                    .items
                    .iter()
                    .find(|i| i.id == *id)
                    .map_or_else(String::new, |i| i.name.clone());
                names::name(state, NameKind::ActionItem, i32::from(*id), &fallback)
            })
            .collect();
        widgets::field_row(ui, &t!("war.active_items"), |ui| {
            ui.label(if active.is_empty() {
                "—".to_owned()
            } else {
                active.join(", ")
            });
        });
    });
}

fn enemies(ui: &mut Ui, war: &mut WarConfig) {
    widgets::card(ui, icon::SKULL, &t!("war.enemies_title"), |ui| {
        widgets::segmented(ui, &mut war.scope, &EnemyScope::ALL, i18n::enemy_scope);
        ui.add_space(theme::SPACE_XS);
        ui.add_enabled_ui(war.scope == EnemyScope::ByRelation, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut war.relations.war, t!("war.relation_war"));
                ui.checkbox(&mut war.relations.neutral, t!("war.relation_neutral"));
                ui.checkbox(&mut war.relations.custom1, t!("war.relation_custom1"));
                ui.checkbox(&mut war.relations.custom2, t!("war.relation_custom2"));
            });
            widgets::toggle_row(
                ui,
                &t!("war.guildless"),
                &t!("war.guildless_hint"),
                &mut war.guildless,
            );
        });
        widgets::muted(ui, &t!("war.always_friendly"));
        widgets::field_row(ui, &t!("war.sight"), |ui| {
            ui.add(egui::Slider::new(&mut war.sight_pct, 100..=400).suffix(" %"));
        });
    });
}

fn friends(ui: &mut Ui, war: &mut WarConfig, draft: &mut String) {
    widgets::card(ui, icon::HANDSHAKE, &t!("war.friends"), |ui| {
        widgets::toggle_row(ui, &t!("war.support"), &t!("war.support_hint"), &mut war.support);
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(draft)
                    .hint_text(t!("war.friend_hint"))
                    .desired_width((ui.available_width() - 90.0).clamp(80.0, 240.0)),
            );
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = draft.trim().to_owned();
            if (ui.button(format!("{} {}", icon::PLUS, t!("war.add"))).clicked() || enter)
                && !name.is_empty()
                && !war.is_friend(&name)
            {
                war.friends.push(name);
                draft.clear();
            }
        });
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for (index, name) in war.friends.iter().enumerate() {
                if ui
                    .button(format!("{name} {}", icon::X))
                    .on_hover_text(t!("target.remove"))
                    .clicked()
                {
                    remove = Some(index);
                }
            }
        });
        if let Some(index) = remove {
            war.friends.remove(index);
        }
        if war.friends.is_empty() {
            widgets::muted(ui, &t!("war.no_friends"));
        }
    });
}

fn rules(ui: &mut Ui, state: &mut State, items: &[Entry]) {
    let active: Vec<i16> = state
        .catalog
        .items
        .iter()
        .filter(|i| i.active)
        .map(|i| i.id)
        .collect();
    let war = &mut state.profile.config.war;
    widgets::card(ui, icon::FLASK, &t!("war.items", count = war.items.len()), |ui| {
        widgets::muted(ui, &t!("war.items_hint"));
        let taken: Vec<i32> = war.items.iter().map(|r| i32::from(r.item)).collect();
        {
            if let Some(entry) = picker(ui, "war-item-picker", &mut state.search.war_items, items, &taken) {
                let name = entry
                    .name
                    .rsplit_once(" (")
                    .map_or(entry.name.as_str(), |(n, _)| n);
                war.items.push(WarItemRule::new(
                    entry.id as i16,
                    name,
                    default_when(entry.id as i16),
                ));
            }
            let suggest = [ITEM_HAILSTORM, ITEM_WINDSTORM, ITEM_UNSTOPPABLE]
                .into_iter()
                .filter(|id| !taken.contains(&i32::from(*id)))
                .filter_map(|id| {
                    items
                        .iter()
                        .find(|e| e.id == i32::from(id))
                        .map(|e| (id, e.name.clone()))
                })
                .collect::<Vec<_>>();
            if !suggest.is_empty()
                && ui
                    .button(format!("{} {}", icon::MAGIC_WAND, t!("war.suggest")))
                    .on_hover_text(t!("war.suggest_hint"))
                    .clicked()
            {
                for (id, name) in suggest {
                    let name = name
                        .rsplit_once(" (")
                        .map_or(name.as_str(), |(n, _)| n)
                        .to_owned();
                    war.items.push(WarItemRule::new(id, name, default_when(id)));
                }
            }
        }
        if items.is_empty() {
            widgets::muted(ui, &t!("war.no_items"));
        }
        ui.add_space(theme::SPACE_XS);
        let mut remove = None;
        let table = Table::new(ui, &RULE_COLUMNS);
        if !war.items.is_empty() {
            table.header(
                ui,
                &[
                    &t!("items.item"),
                    &t!("items.condition"),
                    "%",
                    &t!("items.off"),
                    "",
                ],
            );
        }
        for (index, rule) in war.items.iter_mut().enumerate() {
            table.row(ui, index, |cells| {
                let owned = items.iter().find(|e| e.id == i32::from(rule.item));
                let name = owned.map_or_else(|| rule.name.clone(), |e| e.name.clone());
                let on = active.contains(&rule.item);
                cells.text(if on {
                    RichText::new(format!("{} {name}", icon::LIGHTNING)).color(theme::SUCCESS)
                } else if owned.is_some() {
                    RichText::new(name)
                } else {
                    RichText::new(name).color(theme::MUTED)
                });
                cells.cell(|ui| condition(ui, index, &mut rule.when));
                cells.cell(|ui| threshold(ui, &mut rule.when));
                cells.cell(|ui| {
                    ui.add_enabled(
                        toggles(rule.when),
                        egui::DragValue::new(&mut rule.off_delay_ms)
                            .range(0..=30_000)
                            .speed(50.0)
                            .suffix(" ms"),
                    )
                    .on_hover_text(t!("war.off_delay"));
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
            war.items.remove(index);
        }
        if war.items.is_empty() {
            widgets::muted(ui, &t!("war.no_rules"));
        }
    });
}

fn default_when(item: i16) -> ItemWhen {
    ItemWhen::default_for(item).0
}

fn toggles(when: ItemWhen) -> bool {
    matches!(
        when,
        ItemWhen::TargetNear { .. } | ItemWhen::TargetFar { .. } | ItemWhen::InCombat | ItemWhen::Always
    )
}

fn condition(ui: &mut Ui, index: usize, when: &mut ItemWhen) {
    egui::ComboBox::from_id_salt(("war-when", index))
        .width(ui.available_width() - 4.0)
        .selected_text(i18n::item_when(*when))
        .show_ui(ui, |ui| {
            for kind in ItemWhen::KINDS {
                let selected = when.same_kind(kind);
                if ui.selectable_label(selected, i18n::item_when(kind)).clicked() && !selected {
                    *when = kind;
                }
            }
        });
}

fn threshold(ui: &mut Ui, when: &mut ItemWhen) {
    let percent = |ui: &mut Ui, value: &mut u16, hint: String| {
        ui.add(egui::DragValue::new(value).range(5..=400).suffix(" %"))
            .on_hover_text(hint);
    };
    let health = |ui: &mut Ui, value: &mut u8| {
        ui.add(egui::DragValue::new(value).range(1..=99).suffix(" %"));
    };
    match when {
        ItemWhen::TargetNear { max_pct } => percent(ui, max_pct, t!("war.range_pct_hint").into_owned()),
        ItemWhen::TargetFar { min_pct } => percent(ui, min_pct, t!("war.range_pct_hint").into_owned()),
        ItemWhen::FriendNear { max_pct } => percent(ui, max_pct, t!("war.range_pct_hint").into_owned()),
        ItemWhen::HpBelow(p) | ItemWhen::VpBelow(p) | ItemWhen::FriendHpBelow(p) => health(ui, p),
        ItemWhen::InCombat | ItemWhen::Always | ItemWhen::Immobilized => {}
    }
}

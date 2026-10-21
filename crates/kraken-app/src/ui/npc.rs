use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::names::NameKind;
use kraken_proto::{ItemEntry, NpcTarget, RangeStyle, Stock, StockKind};
use rust_i18n::t;

use super::names::{self, Entry};
use super::widgets::{Col, Table};
use super::{Request, State, widgets};
use crate::{i18n, theme};

const PICKER_HEIGHT: f32 = 300.0;

/// The name of a listed target; the "nearby" marker is only for the picker.
pub fn entry_text(entry: &Entry) -> RichText {
    RichText::new(format!("{} · #{}", entry.name, entry.id))
}

fn option_text(entry: &Entry) -> RichText {
    if entry.nearby {
        RichText::new(format!("{} {} · #{}", icon::MAP_PIN, entry.name, entry.id)).color(theme::NEARBY)
    } else {
        entry_text(entry)
    }
}

/// Search box plus an expandable list of every match (virtualised, so the
/// whole catalogue is reachable without typing).
pub fn picker(ui: &mut Ui, id: &str, query: &mut String, entries: &[Entry], taken: &[i32]) -> Option<Entry> {
    picker_labeled(ui, id, query, entries, taken, |count| {
        t!("target.pick", count = count).into_owned()
    })
}

/// `picker` with its own button text (`label` gets the number of matches).
pub fn picker_labeled(
    ui: &mut Ui,
    id: &str,
    query: &mut String,
    entries: &[Entry],
    taken: &[i32],
    label: impl Fn(usize) -> String,
) -> Option<Entry> {
    let mut picked = None;
    let open_id = ui.id().with((id, "open"));
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let needle = query.trim().to_lowercase();
    let options: Vec<&Entry> = entries
        .iter()
        .filter(|e| !taken.contains(&e.id))
        .filter(|e| {
            needle.is_empty() || e.name.to_lowercase().contains(&needle) || e.id.to_string() == needle
        })
        .collect();
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(theme::MUTED));
        let search = ui.add(
            egui::TextEdit::singleline(query)
                .hint_text(t!("target.search"))
                .desired_width(200.0),
        );
        if search.changed() && !query.trim().is_empty() {
            open = true;
        }
        let caret = if open { icon::CARET_UP } else { icon::CARET_DOWN };
        if ui.button(format!("{} {caret}", label(options.len()))).clicked() {
            open = !open;
        }
        if let Ok(type_id) = needle.parse::<i32>()
            && !taken.contains(&type_id)
            && !entries.iter().any(|e| e.id == type_id)
            && ui
                .button(format!("{} #{type_id}", icon::PLUS))
                .on_hover_text(t!("target.add_by_id"))
                .clicked()
        {
            picked = Some(Entry {
                id: type_id,
                name: format!("#{type_id}"),
                nearby: false,
            });
        }
    });
    if open && !options.is_empty() {
        egui::Frame::new()
            .fill(theme::ABYSS)
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                let row = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y + 4.0;
                egui::ScrollArea::vertical()
                    .id_salt((id, "list"))
                    .max_height(PICKER_HEIGHT)
                    .auto_shrink([false, true])
                    .show_rows(ui, row, options.len(), |ui, range| {
                        for entry in &options[range] {
                            let response = ui.add(
                                egui::Button::selectable(false, option_text(entry))
                                    .truncate()
                                    .min_size(egui::vec2(ui.available_width(), row - 4.0)),
                            );
                            let response = if entry.nearby {
                                response.on_hover_text(t!("target.nearby"))
                            } else {
                                response
                            };
                            if response.clicked() {
                                picked = Some((*entry).clone());
                            }
                        }
                    });
            });
    }
    if picked.is_some() {
        query.clear();
        open = false;
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    picked
}

pub fn stock_of(stock: &[Stock], kind: StockKind, id: i16) -> Option<i64> {
    stock
        .iter()
        .find(|s| s.kind == kind && s.id == id)
        .map(|s| s.amount)
}

pub fn item_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<i16>,
    items: &[ItemEntry],
    stock: &[Stock],
    kind: StockKind,
) -> bool {
    let width = (ui.available_width() - 4.0).clamp(60.0, 220.0);
    item_combo_sized(ui, id, value, items, stock, kind, width)
}

/// `item_combo` with a set width (two side by side in one cell).
pub fn item_combo_sized(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut Option<i16>,
    items: &[ItemEntry],
    stock: &[Stock],
    kind: StockKind,
    width: f32,
) -> bool {
    let label = |id: Option<i16>| match id {
        None => t!("target.auto").into_owned(),
        Some(id) => {
            let name = items
                .iter()
                .find(|i| i.id == id)
                .map_or_else(|| format!("#{id}"), |i| i.name.clone());
            match stock_of(stock, kind, id) {
                Some(amount) => format!("{name} ({amount})"),
                None => name,
            }
        }
    };
    let before = *value;
    let selected = label(*value);
    // A box of exactly `width`: a long name is cut with "…" (full text on
    // hover) instead of widening the box and pushing the row apart.
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_max_width(width);
            egui::ComboBox::from_id_salt(id)
                .width(width)
                .truncate()
                .selected_text(selected.clone())
                .show_ui(ui, |ui| {
                    ui.set_min_width(260.0);
                    ui.selectable_value(value, None, label(None));
                    for item in items {
                        ui.selectable_value(value, Some(item.id), label(Some(item.id)));
                    }
                })
                .response
                .on_hover_text(selected);
        },
    );
    *value != before
}

pub fn priority(ui: &mut Ui, value: &mut u8) {
    ui.add(
        egui::DragValue::new(value)
            .range(1..=10)
            .speed(0.1)
            .prefix(format!("{} ", icon::ARROWS_CLOCKWISE)),
    )
    .on_hover_text(t!("target.priority_hint"));
}

/// Target tables: name, priority, weapon, range style, actions.
pub const TARGET_COLUMNS: [Col; 5] = [
    Col::Fill,
    Col::Fixed(64.0),
    Col::Fixed(220.0),
    Col::Fixed(120.0),
    Col::Fixed(84.0),
];

/// The NPC table: its weapon cell holds the cannonball and the Atlantis
/// torpedo side by side.
const NPC_COLUMNS: [Col; 5] = [
    Col::Fill,
    Col::Fixed(64.0),
    Col::Fixed(300.0),
    Col::Fixed(120.0),
    Col::Fixed(84.0),
];

/// Cannonball and torpedo pickers in one cell, each behind its icon.
#[allow(clippy::too_many_arguments)]
fn weapons(
    ui: &mut Ui,
    index: usize,
    ammo_value: &mut Option<i16>,
    torpedo_value: &mut Option<i16>,
    ammo: &[ItemEntry],
    torpedoes: &[ItemEntry],
    stock: &[Stock],
) {
    ui.spacing_mut().item_spacing.x = 4.0;
    let width = ((ui.available_width() - 2.0 * 18.0 - 3.0 * 4.0) / 2.0).max(48.0);
    ui.label(RichText::new(icon::CIRCLE).color(theme::MUTED))
        .on_hover_text(t!("npc.ammo_column"));
    item_combo_sized(
        ui,
        ("npc-ammo", index),
        ammo_value,
        ammo,
        stock,
        StockKind::Ammo,
        width,
    );
    ui.label(RichText::new(icon::ROCKET).color(theme::SUCCESS))
        .on_hover_text(t!("npc.torpedo_column"));
    item_combo_sized(
        ui,
        ("npc-torpedo", index),
        torpedo_value,
        torpedoes,
        stock,
        StockKind::Torpedo,
        width,
    );
}

/// Opens a target's own settings; shows how many item rules it has.
pub fn settings_button(ui: &mut Ui, rules: usize) -> bool {
    let tip = t!("target.settings");
    let tint = if rules > 0 { theme::ACCENT } else { theme::MUTED };
    widgets::icon_button(ui, icon::GEAR_SIX, &tip, tint, true).clicked()
}

pub fn remove_button(ui: &mut Ui) -> bool {
    widgets::icon_button(ui, icon::TRASH, &t!("target.remove"), theme::DANGER, true).clicked()
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::CROSSHAIR, &t!("page.npc"), &t!("npc.subtitle"));
    let stock = state.live().map(|t| t.stock.clone()).unwrap_or_default();
    let ammo = names::items(state, StockKind::Ammo, &state.catalog.ammo, &stock);
    let torpedoes = names::items(state, StockKind::Torpedo, &state.catalog.torpedoes, &stock);
    let entries = names::entries(state, NameKind::Npc, &state.catalog.npcs);
    let rows = names::order(
        state,
        NameKind::Npc,
        &state
            .profile
            .config
            .npc
            .targets
            .iter()
            .map(|t| (t.type_id, t.name.clone()))
            .collect::<Vec<_>>(),
    );
    let mut editing = None;
    let npc = &mut state.profile.config.npc;
    widgets::columns(ui, 2, |index, ui| match index {
        0 => widgets::card(ui, icon::SWORD, &t!("npc.behaviour"), |ui| {
            widgets::toggle_row(ui, &t!("npc.enabled"), &t!("npc.enabled_hint"), &mut npc.enabled);
            widgets::toggle_row(
                ui,
                &t!("npc.attack_all"),
                &t!("npc.attack_all_hint"),
                &mut npc.attack_all,
            );
            widgets::toggle_row(
                ui,
                &t!("npc.keep_range"),
                &t!("npc.keep_range_hint"),
                &mut npc.keep_range,
            );
            ui.add_enabled_ui(npc.keep_range, |ui| {
                widgets::field_row(ui, &t!("npc.range_percent"), |ui| {
                    ui.add(egui::Slider::new(&mut npc.range_percent, 60..=95).suffix("%"));
                });
            });
        }),
        _ => widgets::card(ui, icon::LIGHTNING, &t!("npc.ammo"), |ui| {
            widgets::field_row(ui, &t!("npc.default_ammo"), |ui| {
                item_combo(
                    ui,
                    "npc-default-ammo",
                    &mut npc.default_ammo,
                    &ammo,
                    &stock,
                    StockKind::Ammo,
                );
            });
            widgets::field_row(
                ui,
                &format!("{} {}", icon::ROCKET, t!("npc.default_torpedo")),
                |ui| {
                    item_combo(
                        ui,
                        "npc-default-torpedo",
                        &mut npc.default_torpedo,
                        &torpedoes,
                        &stock,
                        StockKind::Torpedo,
                    );
                },
            );
            widgets::muted(ui, &t!("npc.ammo_hint"));
            widgets::muted(ui, &t!("npc.torpedo_hint"));
        }),
    });
    widgets::card(
        ui,
        icon::TARGET,
        &t!("npc.targets", count = npc.targets.len()),
        |ui| {
            let taken: Vec<i32> = npc.targets.iter().map(|t| t.type_id).collect();
            if let Some(entry) = picker(ui, "npc-picker", &mut state.search.npc, &entries, &taken) {
                npc.targets.push(NpcTarget {
                    type_id: entry.id,
                    name: entry.name,
                    priority: 5,
                    ammo: None,
                    torpedo: None,
                    range: RangeStyle::Default,
                    items: Vec::new(),
                });
            }
            if entries.is_empty() {
                widgets::muted(ui, &t!("target.catalog_empty"));
            }
            ui.add_space(theme::SPACE_XS);
            let mut remove = None;
            let table = Table::new(ui, &NPC_COLUMNS);
            for (row, (index, entry)) in rows.iter().enumerate() {
                let index = *index;
                let Some(target) = npc.targets.get_mut(index) else {
                    continue;
                };
                table.row(ui, row, |cells| {
                    cells.text(entry_text(entry));
                    cells.cell(|ui| priority(ui, &mut target.priority));
                    cells.cell(|ui| {
                        weapons(
                            ui,
                            index,
                            &mut target.ammo,
                            &mut target.torpedo,
                            &ammo,
                            &torpedoes,
                            &stock,
                        );
                    });
                    cells.cell(|ui| {
                        egui::ComboBox::from_id_salt(("npc-range", index))
                            .width(ui.available_width() - 4.0)
                            .truncate()
                            .selected_text(i18n::range(target.range))
                            .show_ui(ui, |ui| {
                                for style in RangeStyle::ALL {
                                    ui.selectable_value(&mut target.range, style, i18n::range(style));
                                }
                            });
                    });
                    cells.right(|ui| {
                        if remove_button(ui) {
                            remove = Some(index);
                        }
                        if settings_button(ui, target.items.len()) {
                            editing = Some((NameKind::Npc, target.type_id));
                        }
                    });
                });
            }
            if let Some(index) = remove {
                npc.targets.remove(index);
            }
            if npc.targets.is_empty() {
                widgets::muted(ui, &t!("target.none"));
            }
        },
    );
    if editing.is_some() {
        state.search.editing = editing;
    }
    super::items::target_dialog(ui, state);
    if state.connected() {
        ui.add_space(theme::SPACE_S);
        if ui
            .button(format!("{} {}", icon::ARROWS_CLOCKWISE, t!("target.refresh")))
            .clicked()
        {
            state.requests.push(Request::RefreshCatalog);
        }
    }
}

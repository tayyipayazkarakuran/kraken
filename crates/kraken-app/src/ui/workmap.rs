//! Work maps: the maps the bot works on in turn, how long on each, and how
//! it travels between them. Maps are picked on a copy of the game's world
//! map, one tab per region (Nautica, Atlantis, Ragnarök).

use std::collections::HashMap;

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use egui_phosphor::regular as icon;
use kraken_proto::{AtlasEntry, MapRegion, WorkmapEntry, WorkmapInfo, WorkmapPhase};
use rust_i18n::t;

use super::npc::remove_button;
use super::widgets::{Col, Table};
use super::{State, widgets};
use crate::theme;

const LIST_COLUMNS: [Col; 4] = [Col::Fixed(34.0), Col::Fill, Col::Fixed(96.0), Col::Fixed(124.0)];
const CELL_GAP: f32 = 4.0;
const CELL_MIN: f32 = 40.0;
const CELL_MAX: f32 = 74.0;
const CELL_HEIGHT: f32 = 38.0;

/// The world map's label for the map ("30/1", "SH-6", "RÖK-1"), with the
/// localised name of a safe haven after it.
pub fn map_name(state: &State, id: i16) -> String {
    match state.catalog.atlas.iter().find(|m| m.id == id) {
        Some(m) if !m.name.is_empty() && m.name != m.label => format!("{} · {}", m.label, m.name),
        Some(m) if !m.label.is_empty() => m.label.clone(),
        _ => state
            .profile
            .config
            .workmap
            .maps
            .iter()
            .find(|e| e.map == id)
            .map_or_else(|| format!("#{id}"), |e| e.name.clone()),
    }
}

fn region_name(region: MapRegion) -> String {
    match region {
        MapRegion::Nautica => t!("workmap.world_nautica"),
        MapRegion::Atlantis => t!("workmap.world_atlantis"),
        MapRegion::Ragnarok => t!("workmap.world_ragnarok"),
    }
    .into_owned()
}

fn region_tint(region: MapRegion) -> Color32 {
    match region {
        MapRegion::Nautica => theme::ACCENT,
        MapRegion::Atlantis => theme::SUCCESS,
        MapRegion::Ragnarok => theme::WARNING,
    }
}

/// "Working on 13 · 12:30 left", "Sailing to 14 (next: 13)"…
pub fn status(info: &WorkmapInfo, name: impl Fn(i16) -> String) -> String {
    let target = info.target.map(&name).unwrap_or_default();
    match info.phase {
        WorkmapPhase::Idle => t!("workmap.idle").into_owned(),
        WorkmapPhase::Working => {
            let secs = info.remaining_secs;
            t!(
                "workmap.working",
                map = target,
                left = format!("{}:{:02}", secs / 60, secs % 60)
            )
            .into_owned()
        }
        WorkmapPhase::Teleport => t!("workmap.teleporting", map = target).into_owned(),
        WorkmapPhase::Sailing => match info.next_hop {
            Some(hop) => t!("workmap.sailing_via", map = target, hop = name(hop)).into_owned(),
            None => t!("workmap.sailing", map = target).into_owned(),
        },
        WorkmapPhase::Crossing => t!("workmap.crossing", map = target).into_owned(),
        WorkmapPhase::Blocked => t!("workmap.blocked", map = target).into_owned(),
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::GLOBE, &t!("page.workmap"), &t!("workmap.subtitle"));
    let info: Option<WorkmapInfo> = state.live().and_then(|l| l.workmap);
    let line = info.as_ref().map(|i| status(i, |id| map_name(state, id)));
    let current = state
        .live()
        .and_then(|l| l.map.as_ref())
        .and_then(|m| i16::try_from(m.id).ok());
    let target = info.and_then(|i| i.target);
    let atlas = state.catalog.atlas.clone();
    let names: HashMap<i16, String> = atlas
        .iter()
        .map(|m| m.id)
        .chain(state.profile.config.workmap.maps.iter().map(|e| e.map))
        .map(|id| (id, map_name(state, id)))
        .collect();

    let cfg = &mut state.profile.config.workmap;
    widgets::card(ui, icon::GEAR, &t!("workmap.settings"), |ui| {
        if let Some(line) = &line {
            ui.label(RichText::new(line).strong().color(theme::ACCENT));
            ui.add_space(theme::SPACE_XS);
        }
        widgets::columns(ui, 2, |index, ui| {
            if index == 0 {
                widgets::toggle_row(
                    ui,
                    &t!("workmap.enabled"),
                    &t!("workmap.enabled_hint"),
                    &mut cfg.enabled,
                );
                widgets::field_row(ui, &t!("workmap.minutes"), |ui| {
                    ui.add(egui::Slider::new(&mut cfg.minutes, 1..=1440).suffix(" min"));
                });
            } else {
                widgets::toggle_row(
                    ui,
                    &t!("workmap.teleport"),
                    &t!("workmap.teleport_hint"),
                    &mut cfg.teleport,
                );
                widgets::toggle_row(
                    ui,
                    &t!("workmap.safe_havens"),
                    &t!("workmap.safe_havens_hint"),
                    &mut cfg.safe_havens,
                );
            }
        });
    });
    ui.add_space(theme::SPACE_M);

    widgets::card(ui, icon::MAP_TRIFOLD, &t!("workmap.pick_title"), |ui| {
        if atlas.is_empty() {
            ui.label(RichText::new(t!("workmap.no_atlas")).color(theme::WARNING));
            return;
        }
        let tab_id = ui.id().with("workmap-region");
        let mut region: MapRegion = ui.data(|d| d.get_temp(tab_id)).unwrap_or_default();
        let counts: HashMap<MapRegion, (usize, usize)> = MapRegion::ALL
            .iter()
            .map(|r| {
                let maps: Vec<&AtlasEntry> = atlas.iter().filter(|m| m.region == *r).collect();
                let picked = maps
                    .iter()
                    .filter(|m| cfg.maps.iter().any(|e| e.map == m.id))
                    .count();
                (*r, (maps.len(), picked))
            })
            .collect();
        if widgets::segmented(ui, &mut region, &MapRegion::ALL, |r| {
            let (all, picked) = counts.get(&r).copied().unwrap_or_default();
            if picked > 0 {
                format!("{} · {picked}/{all}", region_name(r))
            } else {
                format!("{} · {all}", region_name(r))
            }
        }) {
            ui.data_mut(|d| d.insert_temp(tab_id, region));
        }
        ui.add_space(theme::SPACE_XS);
        widgets::muted(ui, &t!("workmap.pick_hint"));
        ui.add_space(theme::SPACE_XS);
        let cells: Vec<&AtlasEntry> = atlas.iter().filter(|m| m.region == region).collect();
        if let Some(id) = grid(ui, &cells, &cfg.maps, current, target) {
            match cfg.maps.iter().position(|e| e.map == id) {
                Some(index) => {
                    cfg.maps.remove(index);
                }
                None => cfg.maps.push(WorkmapEntry {
                    map: id,
                    name: names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
                }),
            }
        }
        ui.add_space(theme::SPACE_XS);
        legend(ui);
    });
    ui.add_space(theme::SPACE_M);

    widgets::card(
        ui,
        icon::LIST_NUMBERS,
        &t!("workmap.list", count = cfg.maps.len()),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                let taken = current.is_some_and(|id| cfg.maps.iter().any(|e| e.map == id));
                if let Some(id) = current.filter(|_| !taken)
                    && ui
                        .button(format!("{} {}", icon::MAP_PIN, t!("workmap.add_current")))
                        .clicked()
                {
                    cfg.maps.push(WorkmapEntry {
                        map: id,
                        name: names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
                    });
                }
                if !cfg.maps.is_empty()
                    && ui
                        .button(
                            RichText::new(format!("{} {}", icon::TRASH, t!("workmap.clear")))
                                .color(theme::DANGER),
                        )
                        .clicked()
                {
                    cfg.maps.clear();
                }
            });
            ui.add_space(theme::SPACE_XS);
            if cfg.maps.is_empty() {
                widgets::muted(ui, &t!("workmap.empty"));
                return;
            }
            let table = Table::new(ui, &LIST_COLUMNS);
            table.header(ui, &["#", &t!("workmap.map"), &t!("workmap.region"), ""]);
            let mut remove = None;
            let mut swap = None;
            let len = cfg.maps.len();
            for (index, entry) in cfg.maps.iter().enumerate() {
                let cell = atlas.iter().find(|m| m.id == entry.map);
                let name = names
                    .get(&entry.map)
                    .cloned()
                    .unwrap_or_else(|| entry.name.clone());
                table.row(ui, index, |cells| {
                    cells.text(RichText::new(format!("{}", index + 1)).strong());
                    cells.cell(|ui| {
                        let mut text = RichText::new(&name).strong();
                        if Some(entry.map) == target {
                            text = text.color(theme::SUCCESS);
                        }
                        ui.add(egui::Label::new(text).truncate());
                        if let Some(c) = cell.filter(|c| c.min_level > 0) {
                            widgets::muted(ui, &t!("workmap.level", level = c.min_level));
                        }
                        if Some(entry.map) == current {
                            ui.label(RichText::new(icon::MAP_PIN).color(theme::ACCENT))
                                .on_hover_text(t!("workmap.here"));
                        }
                    });
                    cells.cell(|ui| match cell {
                        Some(c) => {
                            ui.label(
                                RichText::new(region_name(c.region))
                                    .small()
                                    .color(region_tint(c.region)),
                            );
                        }
                        None => widgets::muted(ui, "—"),
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
                cfg.maps.swap(a, b);
            }
            if let Some(index) = remove {
                cfg.maps.remove(index);
            }
        },
    );
}

/// The region's part of the world map, laid out like the game's window.
/// Returns the map that was clicked.
fn grid(
    ui: &mut Ui,
    cells: &[&AtlasEntry],
    picked: &[WorkmapEntry],
    current: Option<i16>,
    target: Option<i16>,
) -> Option<i16> {
    let (Some(rows), Some(cols)) = (
        cells
            .iter()
            .map(|c| c.row)
            .min()
            .zip(cells.iter().map(|c| c.row).max()),
        cells
            .iter()
            .map(|c| c.col)
            .min()
            .zip(cells.iter().map(|c| c.col).max()),
    ) else {
        return None;
    };
    let columns = f32::from(cols.1 - cols.0 + 1);
    let width = ui.available_width();
    let cell = ((width - CELL_GAP * (columns - 1.0)) / columns).clamp(CELL_MIN, CELL_MAX);
    let total = cell * columns + CELL_GAP * (columns - 1.0);
    let mut clicked = None;
    egui::ScrollArea::horizontal()
        .id_salt("workmap-grid")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let height = f32::from(rows.1 - rows.0 + 1) * (CELL_HEIGHT + CELL_GAP);
            let (area, _) = ui.allocate_exact_size(vec2(total.max(width), height), Sense::hover());
            let left = area.left() + ((area.width() - total) / 2.0).max(0.0);
            for map in cells {
                let x = left + f32::from(map.col - cols.0) * (cell + CELL_GAP);
                let y = area.top() + f32::from(map.row - rows.0) * (CELL_HEIGHT + CELL_GAP);
                let rect = egui::Rect::from_min_size(egui::pos2(x, y), vec2(cell, CELL_HEIGHT));
                let response = ui.interact(rect, ui.id().with(("cell", map.id)), Sense::click());
                let order = picked.iter().position(|e| e.map == map.id);
                let hovered = response.hovered();
                let tint = region_tint(map.region);
                let (fill, stroke) = match order {
                    Some(_) => (tint.gamma_multiply(0.35), Stroke::new(1.5, tint)),
                    None if map.safe_haven => (theme::RAISED, Stroke::new(1.0, theme::LINE)),
                    None => (theme::SURFACE, Stroke::new(1.0, theme::LINE)),
                };
                let fill = if hovered { fill.gamma_multiply(1.4) } else { fill };
                let painter = ui.painter();
                painter.rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
                if Some(map.id) == target {
                    painter.rect_stroke(
                        rect.expand(2.0),
                        CornerRadius::same(8),
                        Stroke::new(1.5, theme::SUCCESS),
                        StrokeKind::Outside,
                    );
                }
                let label_color = if order.is_some() {
                    theme::TEXT
                } else {
                    theme::MUTED
                };
                let font = egui::FontId::proportional(if cell < 52.0 { 11.0 } else { 12.5 });
                painter.text(
                    rect.center() - vec2(0.0, 5.0),
                    egui::Align2::CENTER_CENTER,
                    &map.label,
                    font,
                    if map.safe_haven && order.is_none() {
                        theme::WARNING
                    } else {
                        label_color
                    },
                );
                let sub = if map.safe_haven {
                    t!("workmap.safe_haven").into_owned()
                } else if map.min_level > 0 {
                    format!("Lv {}", map.min_level)
                } else {
                    String::new()
                };
                painter.text(
                    rect.center() + vec2(0.0, 9.0),
                    egui::Align2::CENTER_CENTER,
                    sub,
                    egui::FontId::proportional(9.5),
                    theme::MUTED,
                );
                if let Some(order) = order {
                    let badge = egui::pos2(rect.right() - 8.0, rect.top() + 8.0);
                    painter.circle_filled(badge, 7.0, tint);
                    painter.text(
                        badge,
                        egui::Align2::CENTER_CENTER,
                        (order + 1).to_string(),
                        egui::FontId::proportional(9.5),
                        theme::ABYSS,
                    );
                }
                if Some(map.id) == current {
                    painter.text(
                        egui::pos2(rect.left() + 8.0, rect.top() + 8.0),
                        egui::Align2::CENTER_CENTER,
                        icon::MAP_PIN,
                        egui::FontId::proportional(10.0),
                        theme::ACCENT,
                    );
                }
                let mut tip = map.label.clone();
                if !map.name.is_empty() {
                    tip.push_str(&format!(" · {}", map.name));
                }
                if map.min_level > 0 {
                    tip.push_str(&format!(" · {}", t!("workmap.level", level = map.min_level)));
                }
                let tip = format!(
                    "{tip}\n{}",
                    if order.is_some() {
                        t!("workmap.click_remove")
                    } else {
                        t!("workmap.click_add")
                    }
                );
                if response.on_hover_text(tip).clicked() {
                    clicked = Some(map.id);
                }
            }
        });
    clicked
}

fn legend(ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_S;
        let swatch = |ui: &mut Ui, fill: Color32, stroke: Color32, text: String| {
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(3),
                fill,
                Stroke::new(1.0, stroke),
                StrokeKind::Inside,
            );
            ui.label(RichText::new(text).small().color(theme::MUTED));
        };
        swatch(
            ui,
            theme::ACCENT.gamma_multiply(0.35),
            theme::ACCENT,
            t!("workmap.legend_picked").into_owned(),
        );
        swatch(
            ui,
            theme::SURFACE,
            theme::SUCCESS,
            t!("workmap.legend_target").into_owned(),
        );
        swatch(
            ui,
            theme::RAISED,
            theme::WARNING,
            t!("workmap.legend_haven").into_owned(),
        );
        ui.label(
            RichText::new(format!("{} {}", icon::MAP_PIN, t!("workmap.here")))
                .small()
                .color(theme::MUTED),
        );
    });
}

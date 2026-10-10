//! Work maps: the maps the bot works on in turn, how long on each, and how
//! it travels between them.

use eframe::egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{WorkmapEntry, WorkmapInfo, WorkmapPhase};
use rust_i18n::t;

use super::names::{self, Entry};
use super::npc::{picker_labeled, remove_button};
use super::widgets::{Col, Table};
use super::{State, widgets};
use crate::theme;

const COLUMNS: [Col; 3] = [Col::Fill, Col::Fixed(110.0), Col::Fixed(124.0)];

/// The map's name in the game language, from the world map or the loca.
pub fn map_name(state: &State, id: i16) -> String {
    let reported = state
        .catalog
        .atlas
        .iter()
        .find(|m| m.id == id)
        .map_or("", |m| m.name.as_str());
    names::name(state, kraken_proto::names::NameKind::Map, i32::from(id), reported)
}

fn world_name(world: u8) -> String {
    match world {
        0 => t!("workmap.world_nautica").into_owned(),
        1 => t!("workmap.world_atlantis").into_owned(),
        n => format!("#{n}"),
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
    let entries: Vec<Entry> = state
        .catalog
        .atlas
        .iter()
        .map(|m| {
            let mut name = format!("{} · {}", map_name(state, m.id), world_name(m.world));
            if m.safe_haven {
                name.push_str(&format!(" · {}", t!("workmap.safe_haven")));
            }
            Entry {
                id: i32::from(m.id),
                name,
                nearby: Some(m.id) == current,
            }
        })
        .collect();
    let names: std::collections::HashMap<i16, String> = state
        .profile
        .config
        .workmap
        .maps
        .iter()
        .map(|e| (e.map, map_name(state, e.map)))
        .chain(current.map(|id| (id, map_name(state, id))))
        .collect();
    let atlas_known = !state.catalog.atlas.is_empty();

    let cfg = &mut state.profile.config.workmap;
    widgets::masonry(ui, "workmap", 2, |index, ui| {
        if index == 0 {
            widgets::card(ui, icon::GEAR, &t!("workmap.settings"), |ui| {
                if let Some(line) = &line {
                    ui.label(RichText::new(line).strong());
                    ui.add_space(theme::SPACE_XS);
                }
                widgets::toggle_row(
                    ui,
                    &t!("workmap.enabled"),
                    &t!("workmap.enabled_hint"),
                    &mut cfg.enabled,
                );
                widgets::field_row(ui, &t!("workmap.minutes"), |ui| {
                    ui.add(eframe::egui::Slider::new(&mut cfg.minutes, 1..=1440).suffix(" min"));
                });
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
            });
        } else {
            widgets::card(ui, icon::INFO, &t!("workmap.how"), |ui| {
                widgets::muted(ui, &t!("workmap.how_text"));
                if !atlas_known {
                    ui.add_space(theme::SPACE_XS);
                    ui.label(RichText::new(t!("workmap.no_atlas")).color(theme::WARNING));
                }
            });
        }
    });

    widgets::card(
        ui,
        icon::LIST_NUMBERS,
        &t!("workmap.list", count = cfg.maps.len()),
        |ui| {
            let taken: Vec<i32> = cfg.maps.iter().map(|e| i32::from(e.map)).collect();
            ui.horizontal_wrapped(|ui| {
                let label = |count: usize| t!("workmap.pick", count = count).into_owned();
                if let Some(entry) = picker_labeled(
                    ui,
                    "workmap-picker",
                    &mut state.search.workmap,
                    &entries,
                    &taken,
                    label,
                ) {
                    let id = entry.id as i16;
                    cfg.maps.push(WorkmapEntry {
                        map: id,
                        name: names.get(&id).cloned().unwrap_or(entry.name),
                    });
                }
                if let Some(id) = current.filter(|id| !taken.contains(&i32::from(*id)))
                    && ui
                        .button(format!("{} {}", icon::MAP_PIN, t!("workmap.add_current")))
                        .clicked()
                {
                    cfg.maps.push(WorkmapEntry {
                        map: id,
                        name: names.get(&id).cloned().unwrap_or_else(|| id.to_string()),
                    });
                }
            });
            ui.add_space(theme::SPACE_XS);
            if cfg.maps.is_empty() {
                widgets::muted(ui, &t!("workmap.empty"));
                return;
            }
            let target = info.and_then(|i| i.target);
            let table = Table::new(ui, &COLUMNS);
            table.header(ui, &[&t!("workmap.map"), &t!("workmap.state"), ""]);
            let mut remove = None;
            let mut swap = None;
            let len = cfg.maps.len();
            for (index, entry) in cfg.maps.iter().enumerate() {
                let name = names
                    .get(&entry.map)
                    .cloned()
                    .unwrap_or_else(|| entry.name.clone());
                table.row(ui, index, |cells| {
                    cells.text(format!("{}. {name}", index + 1));
                    let here = Some(entry.map) == current;
                    let state_text = if Some(entry.map) == target {
                        RichText::new(t!("workmap.current_target")).color(theme::SUCCESS)
                    } else if here {
                        RichText::new(t!("workmap.here")).color(theme::ACCENT)
                    } else {
                        RichText::new("—").color(theme::MUTED)
                    };
                    cells.text(state_text);
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

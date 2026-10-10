use eframe::egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{Capabilities, Telemetry};
use rust_i18n::t;

use super::{State, names, widgets};
use crate::{i18n, theme};

fn per_hour(count: u32, secs: u64) -> String {
    if secs < 60 {
        return "—".into();
    }
    format!("{:.0}", f64::from(count) * 3600.0 / secs as f64)
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::GAUGE, &t!("page.dashboard"), &t!("dashboard.subtitle"));
    let Some(live) = state.live().cloned() else {
        widgets::card(ui, icon::PLUG, &t!("dashboard.waiting_title"), |ui| {
            widgets::muted(ui, &i18n::link(state.link));
            widgets::muted(ui, &t!("dashboard.waiting_hint"));
        });
        ui.add_space(theme::SPACE_M);
        recent(ui, state);
        return;
    };
    tiles(ui, state, &live);
    widgets::columns(ui, 2, |index, ui| match index {
        0 => world(ui, &live),
        _ => bindings(ui, state),
    });
    recent(ui, state);
}

fn tiles(ui: &mut Ui, state: &State, live: &Telemetry) {
    let hp = live.hero.as_ref().map_or(0.0, kraken_proto::Hero::hp_percent);
    let target = live.target.as_ref().map_or_else(
        || "—".to_owned(),
        |t| {
            let name = names::kind_of(t.kind).map_or_else(
                || t.name.clone(),
                |kind| names::name(state, kind, t.type_id, &t.name),
            );
            format!("{name} · {:.0}", t.distance)
        },
    );
    let kills = live.counters.npc_kills + live.counters.monster_kills;
    let items: [(&str, String, String, eframe::egui::Color32); 8] = [
        (
            icon::HEART,
            t!("stat.hp").into_owned(),
            format!("{hp:.0}%"),
            widgets::hp_tint(hp),
        ),
        (
            icon::LIGHTNING,
            t!("stat.state").into_owned(),
            i18n::activity(live.activity),
            theme::ACCENT,
        ),
        (
            icon::TARGET,
            t!("stat.target").into_owned(),
            target,
            theme::WARNING,
        ),
        (
            icon::SKULL,
            t!("stat.kills_h").into_owned(),
            per_hour(kills, live.counters.run_secs),
            theme::DANGER,
        ),
        (
            icon::PACKAGE,
            t!("stat.boxes_h").into_owned(),
            per_hour(live.counters.boxes, live.counters.run_secs),
            theme::SUCCESS,
        ),
        (
            icon::TIMER,
            t!("stat.uptime").into_owned(),
            i18n::duration(live.counters.run_secs),
            theme::ACCENT,
        ),
        (
            icon::CPU,
            t!("stat.pump").into_owned(),
            format!(
                "{:.2} / {:.2} ms",
                f64::from(live.frame_us) / 1000.0,
                f64::from(live.scan_us) / 1000.0
            ),
            theme::MUTED,
        ),
        (
            icon::X_CIRCLE,
            t!("stat.deaths").into_owned(),
            live.counters.deaths.to_string(),
            theme::MUTED,
        ),
    ];
    widgets::columns(ui, 4, |index, ui| {
        for (glyph, label, value, tint) in items.iter().skip(index * 2).take(2) {
            widgets::stat_tile(ui, glyph, label, value, *tint);
        }
    });
}

fn world(ui: &mut Ui, live: &Telemetry) {
    widgets::card(ui, icon::MAP_TRIFOLD, &t!("dashboard.world"), |ui| {
        if let Some(hero) = &live.hero {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&hero.name).strong());
                widgets::muted(ui, &format!("{:.0}, {:.0}", hero.position.x, hero.position.y));
            });
            widgets::bar(ui, hero.hp_percent() / 100.0, widgets::hp_tint(hero.hp_percent()));
            widgets::muted(ui, &format!("{:.0} / {:.0}", hero.hp, hero.hp_max));
            if let Some(vp) = hero.vp_percent() {
                widgets::bar(ui, vp / 100.0, theme::ACCENT);
                widgets::muted(
                    ui,
                    &format!("{} {:.0} / {:.0}", t!("stat.vp"), hero.vp, hero.vp_max),
                );
            }
        }
        if let Some(map) = &live.map {
            widgets::field_row(ui, &t!("dashboard.map"), |ui| {
                ui.label(format!(
                    "#{} · {} · {}×{}",
                    map.id, map.kind, map.width, map.height
                ));
            });
        }
        let visible = live.visible;
        widgets::field_row(ui, &t!("dashboard.visible"), |ui| {
            ui.label(format!(
                "{} {}  {} {}  {} {}  {} {}",
                icon::USERS,
                visible.players,
                icon::CROSSHAIR,
                visible.npcs,
                icon::FISH,
                visible.monsters,
                icon::PACKAGE,
                visible.boxes
            ));
        });
        let counters = live.counters;
        widgets::field_row(ui, &t!("dashboard.totals"), |ui| {
            ui.label(format!(
                "{} {}  {} {}  {} {}",
                icon::CROSSHAIR,
                counters.npc_kills,
                icon::FISH,
                counters.monster_kills,
                icon::PACKAGE,
                counters.boxes
            ));
        });
    });
}

fn capability_list(caps: Capabilities) -> [(String, bool); 12] {
    [
        (t!("cap.world").into_owned(), caps.world),
        (t!("cap.movement").into_owned(), caps.movement),
        (t!("cap.cannon").into_owned(), caps.cannon),
        (t!("cap.harpoon").into_owned(), caps.harpoon),
        (t!("cap.abort_attack").into_owned(), caps.abort_attack),
        (t!("cap.repair").into_owned(), caps.repair),
        (t!("cap.respawn").into_owned(), caps.respawn),
        (t!("cap.reconnect").into_owned(), caps.reconnect),
        (t!("cap.terrain").into_owned(), caps.terrain),
        (t!("cap.screen").into_owned(), caps.screen),
        (t!("cap.select_target").into_owned(), caps.select_target),
        (t!("cap.use_item").into_owned(), caps.use_item),
    ]
}

fn bindings(ui: &mut Ui, state: &State) {
    widgets::card(ui, icon::PLUGS_CONNECTED, &t!("dashboard.bindings"), |ui| {
        let Some(report) = &state.bindings else {
            widgets::muted(ui, &t!("dashboard.no_bindings"));
            return;
        };
        widgets::chip_grid(ui, &capability_list(report.capabilities));
        if !report.missing.is_empty() {
            ui.add_space(theme::SPACE_XS);
            for missing in &report.missing {
                ui.label(
                    RichText::new(t!("dashboard.unsupported", name = missing))
                        .color(theme::WARNING)
                        .small(),
                );
            }
        }
    });
}

fn recent(ui: &mut Ui, state: &State) {
    widgets::card(ui, icon::SCROLL, &t!("dashboard.recent"), |ui| {
        if state.log.is_empty() {
            widgets::muted(ui, &t!("log.empty"));
        }
        for entry in state.log.iter().rev().take(8) {
            super::log::row(ui, entry);
        }
    });
}

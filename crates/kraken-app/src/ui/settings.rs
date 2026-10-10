use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{MixedPriority, WorkMode};
use rust_i18n::t;

use super::{Request, State, widgets};
use crate::{i18n, theme};

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::GEAR, &t!("page.settings"), &t!("settings.subtitle"));
    widgets::masonry(ui, "settings", 5, |index, ui| match index {
        0 => general(ui, state),
        1 => safety(ui, state),
        2 => humanize(ui, state),
        3 => overlay(ui, state),
        _ => connection(ui, state),
    });
}

fn general(ui: &mut Ui, state: &mut State) {
    widgets::card(ui, icon::TRANSLATE, &t!("settings.general"), |ui| {
        widgets::field_row(ui, &t!("settings.language"), |ui| {
            let current = state.profile.locale.clone();
            let label = i18n::LOCALES
                .iter()
                .find(|(code, _)| *code == current)
                .map_or("English", |(_, name)| name);
            egui::ComboBox::from_id_salt("locale")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (code, name) in i18n::LOCALES {
                        if ui.selectable_label(current == code, name).clicked() {
                            state.profile.locale = code.to_owned();
                            i18n::apply(code);
                        }
                    }
                });
        });
        ui.add_space(theme::SPACE_XS);
        ui.label(t!("settings.mode"));
        let config = &mut state.profile.config;
        widgets::segmented(ui, &mut config.mode, &WorkMode::ALL, i18n::work_mode);
        if config.mode == WorkMode::Mixed {
            ui.add_space(theme::SPACE_XS);
            ui.label(t!("settings.mixed_priority"));
            widgets::segmented(
                ui,
                &mut config.mixed_priority,
                &[MixedPriority::Combat, MixedPriority::Collect],
                i18n::mixed_priority,
            );
        }
    });
}

fn safety(ui: &mut Ui, state: &mut State) {
    let safety = &mut state.profile.config.safety;
    widgets::card(ui, icon::WRENCH, &t!("settings.safety"), |ui| {
        widgets::toggle_row(
            ui,
            &t!("settings.repair"),
            &t!("settings.repair_hint"),
            &mut safety.repair,
        );
        ui.add_enabled_ui(safety.repair, |ui| {
            widgets::field_row(ui, &t!("settings.repair_start"), |ui| {
                ui.add(egui::Slider::new(&mut safety.repair_start, 5..=95).suffix("%"));
            });
            widgets::field_row(ui, &t!("settings.repair_end"), |ui| {
                ui.add(egui::Slider::new(&mut safety.repair_end, 10..=100).suffix("%"));
            });
        });
        if safety.repair_end <= safety.repair_start {
            safety.repair_end = (safety.repair_start + 1).min(100);
        }
        widgets::toggle_row(
            ui,
            &t!("settings.respawn"),
            &t!("settings.respawn_hint"),
            &mut safety.respawn,
        );
        widgets::toggle_row(
            ui,
            &t!("settings.reconnect"),
            &t!("settings.reconnect_hint"),
            &mut safety.reconnect,
        );
        ui.add_enabled_ui(safety.reconnect, |ui| {
            widgets::field_row(ui, &t!("settings.reconnect_wait"), |ui| {
                ui.add(egui::Slider::new(&mut safety.reconnect_wait_mins, 1..=60).suffix(" min"));
            });
        });
        widgets::muted(ui, &t!("settings.reconnect_wait_hint"));
    });
}

fn humanize(ui: &mut Ui, state: &mut State) {
    let h = &mut state.profile.config.humanize;
    widgets::card(ui, icon::GHOST, &t!("settings.humanize"), |ui| {
        widgets::toggle_row(
            ui,
            &t!("settings.humanize_enabled"),
            &t!("settings.humanize_hint"),
            &mut h.enabled,
        );
        ui.add_enabled_ui(h.enabled, |ui| {
            widgets::field_row(ui, &t!("settings.humanize_jitter"), |ui| {
                ui.add(egui::Slider::new(&mut h.jitter_pct, 0..=80).suffix("%"));
            });
            widgets::field_row(ui, &t!("settings.humanize_reaction"), |ui| {
                ui.add(egui::Slider::new(&mut h.reaction_ms, 0..=1500).suffix(" ms"));
            });
            widgets::toggle_row(
                ui,
                &t!("settings.humanize_clock"),
                &t!("settings.humanize_clock_hint"),
                &mut h.clock_guard,
            );
        });
    });
}

fn overlay(ui: &mut Ui, state: &mut State) {
    let overlay = &mut state.profile.overlay;
    widgets::card(ui, icon::MONITOR, &t!("settings.overlay"), |ui| {
        widgets::toggle_row(
            ui,
            &t!("settings.overlay_enabled"),
            &t!("settings.overlay_hint"),
            &mut overlay.enabled,
        );
        ui.add_enabled_ui(overlay.enabled, |ui| {
            widgets::toggle_row(ui, &t!("settings.overlay_hud"), "", &mut overlay.hud);
            widgets::toggle_row(
                ui,
                &t!("settings.overlay_markers"),
                &t!("settings.overlay_markers_hint"),
                &mut overlay.markers,
            );
            widgets::toggle_row(
                ui,
                &t!("settings.overlay_ring"),
                &t!("settings.overlay_ring_hint"),
                &mut overlay.range_ring,
            );
            widgets::field_row(ui, &t!("settings.overlay_scale"), |ui| {
                ui.add(egui::Slider::new(&mut overlay.scale, 0.6..=1.5).step_by(0.05));
            });
            widgets::muted(ui, &t!("settings.overlay_keys"));
        });
    });
}

fn account(ui: &mut Ui, state: &mut State) {
    let Some(account) = state.account.clone() else {
        widgets::muted(ui, &t!("settings.account_unknown"));
        return;
    };
    widgets::field_row(ui, &t!("settings.account"), |ui| {
        let text = format!("{} · #{}", account.name, account.user_id);
        ui.add(egui::Label::new(&text).truncate()).on_hover_text(&text);
    });
    let others: Vec<(i64, String)> = crate::profile::accounts()
        .into_iter()
        .filter(|(id, _)| *id != account.user_id)
        .collect();
    if others.is_empty() {
        return;
    }
    widgets::field_row(ui, &t!("settings.copy_from"), |ui| {
        egui::ComboBox::from_id_salt("copy-account")
            .selected_text(t!("settings.copy_pick"))
            .show_ui(ui, |ui| {
                for (id, name) in others {
                    let label = if name.is_empty() { format!("#{id}") } else { name };
                    if ui.selectable_label(false, label).clicked() {
                        state.requests.push(Request::CopyFrom(id));
                    }
                }
            });
    });
}

fn connection(ui: &mut Ui, state: &mut State) {
    widgets::card(ui, icon::PLUGS_CONNECTED, &t!("settings.connection"), |ui| {
        account(ui, state);
        ui.separator();
        widgets::toggle_row(
            ui,
            &t!("settings.auto_attach"),
            &t!("settings.auto_attach_hint"),
            &mut state.profile.auto_attach,
        );
        widgets::toggle_row(
            ui,
            &t!("settings.launch"),
            &t!("settings.launch_hint"),
            &mut state.profile.launch,
        );
        let launching = state.profile.launch;
        ui.add_enabled_ui(launching, |ui| {
            widgets::toggle_row(
                ui,
                &t!("settings.parent_spoof"),
                &t!("settings.parent_spoof_hint"),
                &mut state.profile.parent_spoof,
            );
        });
        ui.add_space(theme::SPACE_XS);
        if ui
            .button(RichText::new(format!("{} {}", icon::PLAY, t!("action.launch"))))
            .on_hover_text(t!("action.launch_hint"))
            .clicked()
        {
            state.requests.push(Request::Launch);
        }
        ui.add_space(theme::SPACE_XS);
        ui.label(RichText::new(t!("settings.processes", count = state.games.len())).strong());
        if state.games.is_empty() {
            widgets::muted(ui, &t!("settings.no_process"));
        }
        let attached = state.link.pid();
        for game in &state.games {
            ui.horizontal(|ui| {
                let current = attached == Some(game.pid);
                ui.label(RichText::new(icon::BOAT).color(if current {
                    theme::SUCCESS
                } else {
                    theme::MUTED
                }));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if current {
                        widgets::muted(ui, &t!("settings.attached"));
                    } else if ui.button(t!("action.attach")).clicked() {
                        state.requests.push(Request::Attach(Some(game.pid)));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(format!("Seafight.exe · PID {}", game.pid)).truncate());
                    });
                });
            });
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    state.connected(),
                    egui::Button::new(format!("{} {}", icon::FILE_TEXT, t!("settings.dump_types"))),
                )
                .clicked()
            {
                state.requests.push(Request::DumpTypes);
            }
            if ui
                .button(
                    RichText::new(format!("{} {}", icon::POWER, t!("settings.quit"))).color(theme::DANGER),
                )
                .on_hover_text(t!("settings.quit_hint"))
                .clicked()
            {
                state.requests.push(Request::Quit);
            }
        });
    });
}

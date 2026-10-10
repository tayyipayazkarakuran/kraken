use eframe::egui::{self, Align, CornerRadius, Layout, Margin, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use kraken_proto::{Control, RunState};
use rust_i18n::t;

use super::{Page, Request, State, widgets};
use crate::supervisor::Link;
use crate::{i18n, theme};

pub fn shell(ui: &mut Ui, state: &mut State) {
    egui::Panel::left("rail")
        .exact_size(theme::RAIL_WIDTH)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(theme::ABYSS)
                .inner_margin(Margin::symmetric(8, 12)),
        )
        .show(ui, |ui| rail(ui, state));
    egui::Panel::top("top")
        .exact_size(theme::TOP_BAR_HEIGHT)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(theme::DEEP)
                .stroke(Stroke::new(1.0, theme::LINE))
                .inner_margin(Margin::symmetric(theme::SPACE_M as i8, 10)),
        )
        .show(ui, |ui| top_bar(ui, state));
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::DEEP)
                .inner_margin(Margin::same(theme::SPACE_L as i8)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match state.page {
                    Page::Dashboard => super::dashboard::show(ui, state),
                    Page::Navigation => super::navigation::show(ui, state),
                    Page::Workmap => super::workmap::show(ui, state),
                    Page::Npc => super::npc::show(ui, state),
                    Page::Monsters => super::monsters::show(ui, state),
                    Page::Collector => super::collector::show(ui, state),
                    Page::War => super::war::show(ui, state),
                    Page::Items => super::items::show(ui, state),
                    Page::Bonus => super::bonus::show(ui, state),
                    Page::Stats => super::stats::show(ui, state),
                    Page::Settings => super::settings::show(ui, state),
                    Page::Log => super::log::show(ui, state),
                });
        });
}

fn rail(ui: &mut Ui, state: &mut State) {
    // A short in-game panel cannot fit every page: the rail scrolls.
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| rail_buttons(ui, state));
}

fn rail_buttons(ui: &mut Ui, state: &mut State) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon::ANCHOR).size(28.0).color(theme::ACCENT));
        ui.add_space(theme::SPACE_L);
        for page in Page::ALL {
            if page == Page::Settings {
                ui.add_space(theme::SPACE_M);
            }
            let selected = state.page == page;
            let button = egui::Button::new(RichText::new(page.icon()).size(22.0).color(if selected {
                theme::ACCENT
            } else {
                theme::MUTED
            }))
            .min_size(vec2(44.0, 44.0))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .fill(if selected {
                theme::RAISED
            } else {
                egui::Color32::TRANSPARENT
            })
            .stroke(Stroke::new(
                1.0,
                if selected {
                    theme::ACCENT_DIM
                } else {
                    egui::Color32::TRANSPARENT
                },
            ));
            if ui.add(button).on_hover_text(page.title()).clicked() {
                state.page = page;
            }
            ui.add_space(4.0);
        }
    });
}

fn link_tint(link: Link) -> egui::Color32 {
    match link {
        Link::Connected(_) => theme::SUCCESS,
        Link::Failed(_) => theme::DANGER,
        Link::Idle => theme::MUTED,
        _ => theme::WARNING,
    }
}

fn top_bar(ui: &mut Ui, state: &mut State) {
    // Controls first, from the right, so they always fit; the status pills
    // get whatever width is left and are clipped in a narrow panel.
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if state.connected() {
            if widgets::icon_button(ui, icon::PLUG, &t!("action.detach"), theme::MUTED, true).clicked() {
                state.requests.push(Request::Detach);
            }
        } else if widgets::icon_button(
            ui,
            icon::PLUGS_CONNECTED,
            &t!("action.attach"),
            theme::ACCENT,
            true,
        )
        .clicked()
        {
            state.requests.push(Request::Attach(None));
        }
        ui.separator();
        let run = state.run_state();
        let connected = state.connected();
        // Settings are per account: nothing runs before the account is known.
        let known = state.account.is_some();
        let buttons = [
            (
                icon::STOP,
                t!("action.stop"),
                theme::DANGER,
                Control::Stop,
                connected && run != RunState::Stopped,
            ),
            (
                icon::PAUSE,
                t!("action.pause"),
                theme::WARNING,
                Control::Pause,
                connected && run == RunState::Running,
            ),
            (
                icon::PLAY,
                t!("action.start"),
                theme::SUCCESS,
                Control::Start,
                connected && known && run != RunState::Running,
            ),
        ];
        for (glyph, tip, tint, control, enabled) in buttons {
            if widgets::icon_button(ui, glyph, &tip, tint, enabled).clicked() {
                state.requests.push(Request::Control(control));
            }
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
            if ui.available_width() >= 700.0 {
                ui.label(RichText::new("KRAKEN").size(16.0).strong().color(theme::TEXT));
                ui.add_space(theme::SPACE_XS);
            }
            let link_icon = if state.connected() {
                icon::PLUGS_CONNECTED
            } else {
                icon::PLUG
            };
            if state.tabs.len() > 1 {
                for tab in state.tabs.clone() {
                    let current = state.pid == Some(tab.pid);
                    let tint = if tab.connected {
                        theme::SUCCESS
                    } else {
                        theme::MUTED
                    };
                    let text = RichText::new(format!("{} {}", icon::BOAT, widgets::short(&tab.label, 16)))
                        .color(if current { theme::TEXT } else { theme::MUTED });
                    let button = egui::Button::new(text)
                        .fill(if current {
                            theme::RAISED
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(Stroke::new(1.0, if current { tint } else { theme::LINE }));
                    if ui.add(button).clicked() && !current {
                        state.requests.push(Request::Select(tab.pid));
                    }
                }
                ui.separator();
            }
            widgets::icon_pill(ui, link_icon, &i18n::link(state.link), link_tint(state.link));
            if let Some(account) = &state.account {
                let room = (ui.available_width() * 0.4).clamp(90.0, 220.0);
                widgets::pill_fit(ui, icon::USER, &account.name, theme::ACCENT, room);
            }
            if let Some(live) = state.live() {
                let session = i18n::session(live.session);
                widgets::icon_pill(ui, icon::BOAT, &session, theme::ACCENT);
                let run = i18n::run_state(live.run);
                let tint = match live.run {
                    RunState::Running => theme::SUCCESS,
                    RunState::Paused => theme::WARNING,
                    RunState::Stopped => theme::MUTED,
                };
                widgets::pill_fit(
                    ui,
                    icon::LIGHTNING,
                    &format!("{run} · {}", i18n::activity(live.activity)),
                    tint,
                    f32::INFINITY,
                );
            }
        });
    });
}

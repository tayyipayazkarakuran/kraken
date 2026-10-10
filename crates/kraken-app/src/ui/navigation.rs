use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{RoamMode, Route};
use rust_i18n::t;

use super::{State, widgets};
use crate::{i18n, theme};

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::COMPASS, &t!("page.navigation"), &t!("nav.subtitle"));
    let live = state.live().cloned();
    let navigation = &mut state.profile.config.navigation;
    widgets::columns(ui, 2, |index, ui| match index {
        0 => widgets::card(ui, icon::COMPASS, &t!("nav.roam"), |ui| {
            widgets::segmented(ui, &mut navigation.roam, &RoamMode::ALL, i18n::roam);
            ui.add_space(theme::SPACE_XS);
            let hint = match navigation.roam {
                RoamMode::Random => t!("roam.random_hint"),
                RoamMode::Corners => t!("roam.corners_hint"),
                RoamMode::Lanes => t!("roam.lanes_hint"),
                RoamMode::Route => t!("roam.route_hint"),
                RoamMode::Idle => t!("roam.idle_hint"),
            };
            widgets::muted(ui, &hint);
            ui.add_space(theme::SPACE_XS);
            widgets::toggle_row(
                ui,
                &t!("nav.avoid_safe_haven"),
                &t!("nav.avoid_safe_haven_hint"),
                &mut navigation.avoid_safe_haven,
            );
        }),
        _ => widgets::card(ui, icon::MAP_TRIFOLD, &t!("nav.position"), |ui| match &live {
            Some(live) => {
                if let Some(map) = &live.map {
                    widgets::field_row(ui, &t!("dashboard.map"), |ui| {
                        ui.label(format!("#{} · {}", map.id, map.kind));
                    });
                }
                if let Some(hero) = &live.hero {
                    widgets::field_row(ui, &t!("nav.ship"), |ui| {
                        ui.label(format!("{:.0}, {:.0}", hero.position.x, hero.position.y));
                    });
                    widgets::field_row(ui, &t!("nav.ranges"), |ui| {
                        ui.label(format!("{:.1} / {:.1}", hero.cannon_range, hero.harpoon_range));
                    });
                }
                widgets::field_row(ui, &t!("nav.waypoint"), |ui| {
                    ui.label(
                        live.waypoint
                            .map_or_else(|| "—".into(), |p| format!("{:.0}, {:.0}", p.x, p.y)),
                    );
                });
            }
            None => widgets::muted(ui, &t!("nav.offline")),
        }),
    });
    routes(ui, state, live.as_ref());
}

fn routes(ui: &mut Ui, state: &mut State, live: Option<&kraken_proto::Telemetry>) {
    let map_id = live.and_then(|t| t.map.as_ref()).map(|m| m.id);
    let position = live.and_then(|t| t.hero.as_ref()).map(|h| h.position.rounded());
    let routes = &mut state.profile.config.navigation.routes;
    widgets::card(ui, icon::MAP_TRIFOLD, &t!("nav.routes"), |ui| {
        if let Some(map_id) = map_id {
            ui.horizontal(|ui| {
                ui.label(RichText::new(t!("nav.current_map", id = map_id)).strong());
                let add = ui.add_enabled(
                    position.is_some(),
                    egui::Button::new(format!("{} {}", icon::PLUS, t!("nav.add_point"))),
                );
                if add.clicked()
                    && let Some(point) = position
                {
                    match routes.iter_mut().find(|r| r.map_id == map_id) {
                        Some(route) => route.points.push(point),
                        None => routes.push(Route {
                            map_id,
                            points: vec![point],
                        }),
                    }
                }
            });
        } else {
            widgets::muted(ui, &t!("nav.routes_offline"));
        }
        ui.add_space(theme::SPACE_XS);
        let mut drop_route = None;
        for (route_index, route) in routes.iter_mut().enumerate() {
            let current = Some(route.map_id) == map_id;
            let title = t!("nav.route_title", id = route.map_id, count = route.points.len());
            egui::CollapsingHeader::new(RichText::new(title).color(if current {
                theme::ACCENT
            } else {
                theme::TEXT
            }))
            .id_salt(("route", route.map_id))
            .default_open(current)
            .show(ui, |ui| {
                let mut drop_point = None;
                for (index, point) in route.points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:>2}", index + 1))
                                .monospace()
                                .color(theme::MUTED),
                        );
                        ui.add(egui::DragValue::new(&mut point.x).speed(1.0).prefix("x "));
                        ui.add(egui::DragValue::new(&mut point.y).speed(1.0).prefix("y "));
                        if widgets::icon_button(ui, icon::TRASH, &t!("target.remove"), theme::DANGER, true)
                            .clicked()
                        {
                            drop_point = Some(index);
                        }
                    });
                }
                if let Some(index) = drop_point {
                    route.points.remove(index);
                }
                if ui
                    .button(format!("{} {}", icon::TRASH, t!("nav.delete_route")))
                    .clicked()
                {
                    drop_route = Some(route_index);
                }
            });
        }
        if let Some(index) = drop_route {
            routes.remove(index);
        }
        if routes.is_empty() {
            widgets::muted(ui, &t!("nav.no_routes"));
        }
    });
}

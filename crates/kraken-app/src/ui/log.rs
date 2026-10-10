use eframe::egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::Level;
use rust_i18n::t;

use super::{LogEntry, LogKind, State, widgets};
use crate::{i18n, theme};

fn level_style(level: Level) -> (&'static str, eframe::egui::Color32) {
    match level {
        Level::Info => (icon::INFO, theme::ACCENT),
        Level::Success => (icon::CHECK_CIRCLE, theme::SUCCESS),
        Level::Warning => (icon::WARNING, theme::WARNING),
        Level::Error => (icon::X_CIRCLE, theme::DANGER),
    }
}

pub fn row(ui: &mut Ui, entry: &LogEntry) {
    let (glyph, tint) = level_style(entry.level);
    let text = match &entry.kind {
        LogKind::Agent(event) => i18n::event(event),
        LogKind::Link(link) => i18n::link(*link),
    };
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(&entry.stamp).monospace().color(theme::MUTED));
        ui.label(RichText::new(glyph).color(tint));
        ui.add(eframe::egui::Label::new(text).wrap());
    });
}

fn level_name(level: Option<Level>) -> String {
    match level {
        None => t!("log.all"),
        Some(Level::Info) => t!("log.info"),
        Some(Level::Success) => t!("log.success"),
        Some(Level::Warning) => t!("log.warning"),
        Some(Level::Error) => t!("log.error"),
    }
    .into_owned()
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::SCROLL, &t!("page.log"), &t!("log.subtitle"));
    ui.horizontal(|ui| {
        let options = [
            None,
            Some(Level::Info),
            Some(Level::Success),
            Some(Level::Warning),
            Some(Level::Error),
        ];
        widgets::segmented(ui, &mut state.search.log_filter, &options, level_name);
        ui.with_layout(
            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
            |ui| {
                if ui
                    .button(format!("{} {}", icon::TRASH, t!("log.clear")))
                    .clicked()
                {
                    state.log.clear();
                }
            },
        );
    });
    ui.add_space(theme::SPACE_S);
    let filter = state.search.log_filter;
    widgets::card(
        ui,
        icon::SCROLL,
        &t!("log.events", count = state.log.len()),
        |ui| {
            let mut shown = 0;
            for entry in state
                .log
                .iter()
                .rev()
                .filter(|e| filter.is_none_or(|f| f == e.level))
            {
                row(ui, entry);
                shown += 1;
            }
            if shown == 0 {
                widgets::muted(ui, &t!("log.empty"));
            }
        },
    );
}

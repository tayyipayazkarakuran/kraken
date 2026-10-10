use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;
use kraken_proto::{Event, EventCode, RunState, StockKind, Telemetry};
use rust_i18n::t;

use super::widgets::{Col, Table};
use super::{State, names, widgets};

const WALLET_COLUMNS: [Col; 4] = [Col::Fill, Col::Fixed(120.0), Col::Fixed(110.0), Col::Fixed(110.0)];
const COUNT_COLUMNS: [Col; 2] = [Col::Fill, Col::Fixed(60.0)];
use crate::{i18n, theme};

/// What the session earned since the bot was last started: wallet changes
/// plus kills and boxes by name.
#[derive(Default)]
pub struct Stats {
    pub since: Option<Instant>,
    base: HashMap<(u8, i16), i64>,
    now: HashMap<(u8, i16), i64>,
    kinds: HashMap<u8, StockKind>,
    pub kills: BTreeMap<String, u32>,
    pub boxes: BTreeMap<String, u32>,
    /// Finished El Dorado runs and the chests collected in them.
    pub eldorado: (u32, u32),
    last_run: RunState,
}

fn kind_key(kind: StockKind) -> u8 {
    match kind {
        StockKind::Ammo => 0,
        StockKind::Harpoon => 1,
        StockKind::Currency => 2,
        StockKind::Points => 3,
        StockKind::Token => 4,
        StockKind::BonusMap => 5,
    }
}

/// One wallet line: what changed since the start.
pub struct Gain {
    pub kind: StockKind,
    pub id: i16,
    pub base: i64,
    pub now: i64,
}

impl Gain {
    pub fn delta(&self) -> i64 {
        self.now - self.base
    }
}

impl Stats {
    pub fn reset(&mut self) {
        self.base = self.now.clone();
        self.kills.clear();
        self.boxes.clear();
        self.eldorado = (0, 0);
        self.since = Some(Instant::now());
    }

    pub fn observe(&mut self, telemetry: &Telemetry) {
        let wallet: Vec<_> = telemetry.stock.iter().filter(|s| s.kind.wallet()).collect();
        if wallet.is_empty() {
            return;
        }
        let first = self.now.is_empty();
        for stock in wallet {
            let key = (kind_key(stock.kind), stock.id);
            self.kinds.insert(key.0, stock.kind);
            self.now.insert(key, stock.amount);
            // Something new after the start was earned from zero.
            self.base
                .entry(key)
                .or_insert(if first { stock.amount } else { 0 });
        }
        if first {
            self.since = Some(Instant::now());
        }
        if telemetry.run == RunState::Running && self.last_run == RunState::Stopped {
            self.reset();
        }
        self.last_run = telemetry.run;
    }

    pub fn event(&mut self, event: &Event) {
        let Some(name) = event.args.first() else { return };
        if event.code == EventCode::ElDoradoLeft {
            self.eldorado.0 += 1;
            self.eldorado.1 += name.parse::<u32>().unwrap_or(0);
            return;
        }
        let table = match event.code {
            EventCode::TargetKilled => &mut self.kills,
            EventCode::BoxCollected => &mut self.boxes,
            _ => return,
        };
        *table.entry(name.clone()).or_default() += 1;
    }

    /// Every wallet entry with a change, biggest gain first.
    pub fn gains(&self) -> Vec<Gain> {
        let mut out: Vec<Gain> = self
            .now
            .iter()
            .filter_map(|(key, now)| {
                let base = *self.base.get(key).unwrap_or(now);
                Some(Gain {
                    kind: *self.kinds.get(&key.0)?,
                    id: key.1,
                    base,
                    now: *now,
                })
            })
            .collect();
        out.sort_by(|a, b| b.delta().cmp(&a.delta()).then(a.id.cmp(&b.id)));
        out
    }

    pub fn hours(&self) -> f64 {
        self.since.map_or(0.0, |t| t.elapsed().as_secs_f64() / 3600.0)
    }
}

pub fn gain_name(state: &State, gain: &Gain) -> String {
    names::name(state, names::stock_names(gain.kind), i32::from(gain.id), "")
}

pub fn signed(value: i64) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}

pub fn show(ui: &mut Ui, state: &mut State) {
    widgets::page_title(ui, icon::CHART_BAR, &t!("page.stats"), &t!("stats.subtitle"));
    ui.horizontal(|ui| {
        let since = state
            .stats
            .since
            .map_or_else(|| "—".to_owned(), |t| i18n::duration(t.elapsed().as_secs()));
        widgets::muted(ui, &t!("stats.since", time = since));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(format!("{} {}", icon::ARROW_COUNTER_CLOCKWISE, t!("stats.reset")))
                .clicked()
            {
                state.stats.reset();
            }
        });
    });
    ui.add_space(theme::SPACE_S);
    let gains = state.stats.gains();
    let hours = state.stats.hours();
    widgets::card(ui, icon::COINS, &t!("stats.wallet"), |ui| {
        let changed: Vec<&Gain> = gains.iter().filter(|g| g.delta() != 0).collect();
        if changed.is_empty() {
            widgets::muted(ui, &t!("stats.nothing"));
        }
        if changed.is_empty() {
            return;
        }
        let table = Table::new(ui, &WALLET_COLUMNS);
        table.header(
            ui,
            &[
                &t!("stats.item"),
                &t!("stats.now"),
                &t!("stats.gained"),
                &t!("stats.per_hour"),
            ],
        );
        for (index, gain) in changed.into_iter().enumerate() {
            table.row(ui, index, |cells| {
                cells.text(gain_name(state, gain));
                cells.right(|ui| ui.label(gain.now.to_string()));
                let tint = if gain.delta() > 0 {
                    theme::SUCCESS
                } else {
                    theme::DANGER
                };
                cells.right(|ui| ui.label(RichText::new(signed(gain.delta())).color(tint).strong()));
                let rate = if hours > 0.01 {
                    gain.delta() as f64 / hours
                } else {
                    0.0
                };
                cells.right(|ui| ui.label(RichText::new(format!("{rate:.0}")).color(theme::MUTED)));
            });
        }
    });
    let (runs, chests) = state.stats.eldorado;
    if runs > 0 {
        widgets::card(ui, icon::CROWN, &t!("eldorado.title"), |ui| {
            ui.label(t!("eldorado.runs", runs = runs, chests = chests));
        });
    }
    widgets::columns(ui, 2, |index, ui| {
        let (glyph, title, table) = match index {
            0 => (icon::CROSSHAIR, t!("stats.kills"), &state.stats.kills),
            _ => (icon::PACKAGE, t!("stats.boxes"), &state.stats.boxes),
        };
        let total: u32 = table.values().sum();
        widgets::card(ui, glyph, &format!("{title} ({total})"), |ui| {
            if table.is_empty() {
                widgets::muted(ui, &t!("stats.nothing"));
            }
            let mut rows: Vec<(&String, &u32)> = table.iter().collect();
            rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let layout = Table::new(ui, &COUNT_COLUMNS);
            for (index, (name, count)) in rows.into_iter().enumerate() {
                layout.row(ui, index, |cells| {
                    cells.text(name.as_str());
                    cells.right(|ui| ui.label(RichText::new(count.to_string()).strong()));
                });
            }
        });
    });
}

mod bonus;
mod collector;
mod dashboard;
pub mod hud;
mod items;
mod log;
mod monsters;
pub mod names;
mod navigation;
mod npc;
mod settings;
mod shell;
pub mod stats;
mod war;
pub mod widgets;
pub mod workmap;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use egui_phosphor::regular as icon;
use kraken_proto::{BindingReport, Catalog, Control, Event, Level, RunState, Telemetry};
use rust_i18n::t;

use crate::gamedata::Language;
use crate::process::GameProcess;
use crate::profile::Profile;
use crate::supervisor::Link;

pub use shell::shell;

const LOG_CAPACITY: usize = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Dashboard,
    Navigation,
    Workmap,
    Npc,
    Monsters,
    Collector,
    War,
    Items,
    Bonus,
    Stats,
    Settings,
    Log,
}

impl Page {
    pub const ALL: [Self; 12] = [
        Self::Dashboard,
        Self::Navigation,
        Self::Workmap,
        Self::Npc,
        Self::Monsters,
        Self::Collector,
        Self::War,
        Self::Items,
        Self::Bonus,
        Self::Stats,
        Self::Settings,
        Self::Log,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Self::Dashboard => icon::GAUGE,
            Self::Navigation => icon::COMPASS,
            Self::Workmap => icon::GLOBE,
            Self::Npc => icon::CROSSHAIR,
            Self::Monsters => icon::FISH,
            Self::Collector => icon::PACKAGE,
            Self::War => icon::SWORD,
            Self::Items => icon::FLASK,
            Self::Bonus => icon::MAP_TRIFOLD,
            Self::Stats => icon::CHART_BAR,
            Self::Settings => icon::GEAR,
            Self::Log => icon::SCROLL,
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Dashboard => t!("page.dashboard"),
            Self::Navigation => t!("page.navigation"),
            Self::Workmap => t!("page.workmap"),
            Self::Npc => t!("page.npc"),
            Self::Monsters => t!("page.monsters"),
            Self::Collector => t!("page.collector"),
            Self::War => t!("page.war"),
            Self::Items => t!("page.items"),
            Self::Bonus => t!("page.bonus"),
            Self::Stats => t!("page.stats"),
            Self::Settings => t!("page.settings"),
            Self::Log => t!("page.log"),
        }
        .into_owned()
    }
}

#[derive(Clone, Debug)]
pub enum LogKind {
    Agent(Event),
    Link(Link),
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub stamp: String,
    pub level: Level,
    pub kind: LogKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Control(Control),
    Attach(Option<u32>),
    Detach,
    /// Start a client of our own (the user's button).
    Launch,
    DumpTypes,
    RefreshCatalog,
    /// Show this game's session in the window.
    Select(u32),
    /// Copy another account's settings into this one.
    CopyFrom(i64),
    /// Stop the bots, take the agents out and close Kraken.
    Quit,
}

/// The game account a session belongs to (`PlayerInfoSystem.UserId`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub user_id: i64,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    pub pid: u32,
    pub label: String,
    pub connected: bool,
}

#[derive(Default)]
pub struct State {
    /// The game process this state belongs to; `None` while no game runs.
    pub pid: Option<u32>,
    pub account: Option<Account>,
    /// Every running game, for switching in the window.
    pub tabs: Vec<SessionTab>,
    pub profile: Profile,
    pub page: Page,
    pub link: Link,
    pub games: Vec<GameProcess>,
    pub telemetry: Option<Telemetry>,
    pub telemetry_at: Option<Instant>,
    pub catalog: Catalog,
    pub names: Arc<HashMap<String, Language>>,
    pub names_failed: Arc<HashMap<String, String>>,
    pub bindings: Option<BindingReport>,
    pub log: VecDeque<LogEntry>,
    pub intent: Option<Control>,
    pub requests: Vec<Request>,
    pub search: Search,
    pub stats: stats::Stats,
}

#[derive(Default)]
pub struct Search {
    pub npc: String,
    pub monster: String,
    pub boxes: String,
    pub war_items: String,
    pub friend: String,
    pub items: String,
    pub items_target: String,
    pub bonus: String,
    pub workmap: String,
    /// The NPC or monster target whose settings window is open.
    pub editing: Option<(kraken_proto::names::NameKind, i32)>,
    pub log_filter: Option<Level>,
}

impl State {
    pub fn connected(&self) -> bool {
        matches!(self.link, Link::Connected(_))
    }

    pub fn live(&self) -> Option<&Telemetry> {
        if !self.connected() {
            return None;
        }
        self.telemetry
            .as_ref()
            .filter(|_| self.telemetry_at.is_some_and(|at| at.elapsed().as_secs() < 5))
    }

    pub fn run_state(&self) -> RunState {
        self.live().map_or(RunState::Stopped, |t| t.run)
    }

    pub fn push_log(&mut self, level: Level, kind: LogKind) {
        if self.log.len() >= LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            stamp: crate::clock::stamp(),
            level,
            kind,
        });
    }
}

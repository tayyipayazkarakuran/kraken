use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use eframe::egui;
use kraken_proto::{AgentMessage, AppMessage, Config, Control, Level};
use parking_lot::Mutex;

use crate::gamedata::{self, Update};
use crate::profile::{Profile, Store, account_path, data_dir};
use crate::remote::Remote;
use crate::supervisor::{Fault, Link, Mode, Note, Supervisor};
use crate::ui::{self, Account, LogKind, Request, State};
use crate::{i18n, theme};

const CONFIG_DELAY: Duration = Duration::from_millis(300);
/// How often the core runs when nothing arrives: in-game frames, config
/// pushes and saves.
const CORE_TICK: Duration = Duration::from_millis(33);
/// Kraken quits this long after the last game closed…
const QUIT_AFTER_GAMES: Duration = Duration::from_secs(30);
/// …or when no game showed up at all for this long. A client that never
/// starts (fault, or the game refusing to run) must not leave Kraken
/// sitting hidden for ten minutes.
const QUIT_WITHOUT_GAME: Duration = Duration::from_secs(150);

/// One game client: its UI state, its in-game interface and, once the hero
/// is known, the account's own settings file.
pub struct Session {
    pub state: State,
    remote: Remote,
    account: Option<Store>,
    sent_config: Option<Config>,
    config_changed: Option<Instant>,
    /// Input arrived: build a frame now instead of on the next tick.
    urgent: bool,
}

impl Session {
    fn new(pid: u32, global: &Profile, shared: &Shared) -> Self {
        let mut state = State {
            profile: global.clone(),
            pid: Some(pid),
            ..State::default()
        };
        shared.apply(&mut state);
        Self {
            state,
            remote: Remote::new(),
            account: None,
            sent_config: None,
            config_changed: None,
            urgent: false,
        }
    }

    fn pid(&self) -> u32 {
        self.state.pid.unwrap_or(0)
    }
}

/// Game data shared by every session.
#[derive(Default)]
struct Shared {
    names: Arc<HashMap<String, gamedata::Language>>,
    failed: Arc<HashMap<String, String>>,
    games: Vec<crate::process::GameProcess>,
}

impl Shared {
    fn apply(&self, state: &mut State) {
        state.names = self.names.clone();
        state.names_failed = self.failed.clone();
        state.games = self.games.clone();
    }
}

/// Everything except the window: sessions, settings and the link to the
/// agents. Runs on its own thread so the in-game interface stays responsive
/// while the window is minimised.
pub struct Core {
    /// Global settings (language, auto-attach, overlay) and the template for
    /// new accounts.
    global: Profile,
    store: Store,
    sessions: Vec<Session>,
    /// Shown in the window when no game is running.
    idle: State,
    selected: Option<u32>,
    shared: Shared,
    supervisor: Supervisor,
    names: Receiver<Update>,
    auto_attach: bool,
    mode: Mode,
    ctx: egui::Context,
    started: Instant,
    /// A game was attached at some point.
    seen_game: bool,
    /// Since when no game has been running.
    empty_since: Option<Instant>,
    /// Kraken should close (button, or no game left).
    quit: bool,
    /// Something the user must see went wrong: show the window.
    show_window: bool,
    hide_check: Option<Instant>,
}

impl Core {
    fn session(&mut self, pid: u32) -> &mut Session {
        if let Some(index) = self.sessions.iter().position(|s| s.pid() == pid) {
            return &mut self.sessions[index];
        }
        self.sessions.push(Session::new(pid, &self.global, &self.shared));
        self.sessions.last_mut().expect("just pushed")
    }

    fn note(&mut self, note: Note) {
        match note {
            Note::Link(pid, link) => self.on_link(pid, link),
            Note::Status(link) => {
                if self.idle.link != link {
                    self.idle.link = link;
                    self.idle.push_log(level_of(link), LogKind::Link(link));
                }
            }
            Note::Games(games) => {
                self.shared.games = games;
                for game in self.shared.games.clone() {
                    self.session(game.pid);
                }
                self.share();
            }
            Note::Agent(pid, message) => self.on_agent(pid, message),
            Note::Gone(pid) => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.pid() == pid)
                    && let Some(store) = session.account.as_mut()
                {
                    store.flush(&session.state.profile);
                }
                self.sessions.retain(|s| s.pid() != pid);
                if self.selected == Some(pid) {
                    self.selected = self.sessions.first().map(|s| s.pid());
                }
            }
        }
    }

    fn drain_names(&mut self) {
        let mut changed = false;
        while let Ok(update) = self.names.try_recv() {
            changed = true;
            match update {
                Update::Loaded(lang, language) => {
                    Arc::make_mut(&mut self.shared.failed).remove(&lang);
                    Arc::make_mut(&mut self.shared.names).insert(lang, language);
                }
                Update::Failed(lang, error) => {
                    Arc::make_mut(&mut self.shared.failed).insert(lang, error);
                }
            }
        }
        if changed {
            self.share();
        }
    }

    fn share(&mut self) {
        for session in &mut self.sessions {
            self.shared.apply(&mut session.state);
        }
        self.shared.apply(&mut self.idle);
    }

    fn on_link(&mut self, pid: u32, link: Link) {
        let session = self.session(pid);
        if session.state.link == link {
            return;
        }
        session.state.link = link;
        session.state.push_log(level_of(link), LogKind::Link(link));
        if let Link::Connected(_) = link {
            session.sent_config = None;
            // A (re)started agent has no textures yet.
            session.remote.reset();
            let intent = session.state.intent;
            self.push_config(pid);
            if let Some(intent) = intent {
                self.supervisor.send(pid, AppMessage::Control(intent));
            }
        } else {
            session.state.telemetry = None;
            session.state.telemetry_at = None;
        }
    }

    fn on_agent(&mut self, pid: u32, message: AgentMessage) {
        let session = self.session(pid);
        match message {
            AgentMessage::Hello { .. } => {}
            AgentMessage::Bindings(report) => session.state.bindings = Some(report),
            AgentMessage::Telemetry(telemetry) => {
                let hero = telemetry.hero.as_ref().map(|h| (h.user_id, h.name.clone()));
                session.state.stats.observe(&telemetry);
                session.state.telemetry = Some(*telemetry);
                session.state.telemetry_at = Some(Instant::now());
                if let Some((user_id, name)) = hero
                    && user_id > 0
                {
                    self.adopt_account(pid, user_id, name);
                }
            }
            AgentMessage::Catalog(catalog) => {
                session.state.catalog = *catalog;
                let owned = session
                    .state
                    .catalog
                    .items
                    .iter()
                    .map(|i| (i.id, i.name.as_str()));
                session.state.profile.config.war.add_defaults(owned);
            }
            AgentMessage::Event(event) => {
                session.state.stats.event(&event);
                session.state.push_log(event.level, LogKind::Agent(event));
            }
            AgentMessage::Input(input) => {
                let scale = session.state.profile.overlay.scale;
                if session.remote.input(input, scale) {
                    session.urgent = true;
                }
            }
        }
    }

    /// Loads (or creates from the current settings) the account's profile the
    /// first time its hero shows up.
    fn adopt_account(&mut self, pid: u32, user_id: i64, name: String) {
        let global = self.global.clone();
        let session = self.session(pid);
        if session
            .state
            .account
            .as_ref()
            .is_some_and(|a| a.user_id == user_id)
        {
            if let Some(account) = session.state.account.as_mut() {
                account.name = name;
            }
            return;
        }
        let path = account_path(user_id);
        let existed = path.is_file();
        let (mut store, mut profile) = Store::open(path);
        if !existed {
            profile = Profile {
                config: session.state.profile.config.clone(),
                ..global.clone()
            };
        }
        profile.locale = global.locale.clone();
        profile.auto_attach = global.auto_attach;
        profile.overlay = global.overlay;
        profile.account.clone_from(&name);
        store.flush(&profile);
        tracing::info!(pid, user_id, %name, existed, "account profile");
        session.state.profile = profile;
        session.state.account = Some(Account { user_id, name });
        session.account = Some(store);
        session.sent_config = None;
        self.push_config(pid);
    }

    fn push_config(&mut self, pid: u32) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.pid() == pid) else {
            return;
        };
        if !session.state.connected() {
            return;
        }
        let config = session.state.profile.config.clone();
        self.supervisor
            .send(pid, AppMessage::Configure(Box::new(config.clone())));
        session.sent_config = Some(config);
        session.config_changed = None;
    }

    /// Global fields edited in any session apply everywhere.
    fn sync_globals(&mut self, from: &Profile) {
        let changed = from.locale != self.global.locale
            || from.auto_attach != self.global.auto_attach
            || from.launch != self.global.launch
            || from.parent_spoof != self.global.parent_spoof
            || from.overlay != self.global.overlay;
        if !changed {
            return;
        }
        if from.locale != self.global.locale {
            i18n::apply(&from.locale);
        }
        self.global.locale.clone_from(&from.locale);
        self.global.auto_attach = from.auto_attach;
        self.global.launch = from.launch;
        self.global.parent_spoof = from.parent_spoof;
        self.global.overlay = from.overlay.clone();
        for state in self
            .sessions
            .iter_mut()
            .map(|s| &mut s.state)
            .chain(std::iter::once(&mut self.idle))
        {
            state.profile.locale.clone_from(&from.locale);
            state.profile.auto_attach = from.auto_attach;
            state.profile.overlay = from.overlay.clone();
        }
    }

    fn sync(&mut self, now: Instant) {
        let due: Vec<u32> = self
            .sessions
            .iter_mut()
            .filter(|s| s.state.connected() && s.sent_config.as_ref() != Some(&s.state.profile.config))
            .filter_map(|s| {
                let changed = *s.config_changed.get_or_insert(now);
                (now.duration_since(changed) >= CONFIG_DELAY).then(|| s.pid())
            })
            .collect();
        for pid in due {
            self.push_config(pid);
        }
        if self.auto_attach != self.global.auto_attach {
            self.auto_attach = self.global.auto_attach;
            self.supervisor.set_auto(self.auto_attach);
        }
        let mode = Mode {
            launch: self.global.launch,
            parent_spoof: self.global.parent_spoof,
        };
        if self.mode != mode {
            self.mode = mode;
            self.supervisor.set_mode(mode);
        }
        for session in &mut self.sessions {
            match session.account.as_mut() {
                Some(store) => {
                    store.track(&session.state.profile, now);
                }
                // Before the account is known, edits go to the template.
                None => self.global.config = session.state.profile.config.clone(),
            }
        }
        if self.sessions.is_empty() {
            self.global.config = self.idle.profile.config.clone();
        }
        self.store.track(&self.global, now);
    }

    fn handle(&mut self, pid: Option<u32>, requests: Vec<Request>) {
        for request in requests {
            match request {
                Request::Select(target) => self.selected = Some(target),
                Request::Quit => self.quit = true,
                Request::Launch => self.supervisor.launch(),
                Request::Attach(target) => {
                    if let Some(target) = target.or(pid) {
                        self.supervisor.attach(target);
                    }
                }
                _ => {
                    let Some(pid) = pid else { continue };
                    self.session_request(pid, request);
                }
            }
        }
    }

    fn session_request(&mut self, pid: u32, request: Request) {
        let session = self.session(pid);
        match request {
            Request::Control(control) => {
                session.state.intent = match control {
                    Control::Start | Control::Pause => Some(control),
                    Control::Stop => None,
                };
                if control == Control::Start {
                    self.push_config(pid);
                }
                self.supervisor.send(pid, AppMessage::Control(control));
            }
            Request::Detach => {
                session.state.intent = None;
                self.supervisor.send(pid, AppMessage::Control(Control::Stop));
                self.supervisor.detach(pid);
            }
            Request::DumpTypes => self.supervisor.send(pid, AppMessage::DumpTypes),
            Request::RefreshCatalog => self.supervisor.send(pid, AppMessage::RequestCatalog),
            Request::CopyFrom(user_id) => {
                if let Some(config) = crate::profile::account_config(user_id) {
                    session.state.profile.config = config;
                }
            }
            Request::Select(_) | Request::Attach(_) | Request::Launch | Request::Quit => {}
        }
    }

    fn tabs(&self) -> Vec<ui::SessionTab> {
        self.sessions
            .iter()
            .map(|s| ui::SessionTab {
                pid: s.pid(),
                label: s
                    .state
                    .account
                    .as_ref()
                    .map_or_else(|| format!("PID {}", s.pid()), |a| a.name.clone()),
                connected: s.state.connected(),
            })
            .collect()
    }

    /// The session shown in the window: the chosen one, else the first.
    fn current(&mut self) -> Option<usize> {
        let index = self
            .selected
            .and_then(|pid| self.sessions.iter().position(|s| s.pid() == pid))
            .or_else(|| (!self.sessions.is_empty()).then_some(0));
        self.selected = index.map(|i| self.sessions[i].pid());
        index
    }

    /// Builds and sends the in-game interface of every attached game.
    fn remote_frames(&mut self) {
        let tabs = self.tabs();
        let mut changes = Vec::new();
        for index in 0..self.sessions.len() {
            let session = &mut self.sessions[index];
            if !session.state.connected() || !session.remote.has_size() {
                continue;
            }
            let pid = session.pid();
            let urgent = std::mem::take(&mut session.urgent);
            session.state.tabs.clone_from(&tabs);
            let frame = session.remote.frame(&mut session.state, urgent);
            let requests = std::mem::take(&mut session.state.requests);
            let profile = session.state.profile.clone();
            if let Some(frame) = frame {
                self.supervisor.send(pid, AppMessage::Frame(Box::new(frame)));
            }
            changes.push((pid, requests, profile));
        }
        for (pid, requests, profile) in changes {
            self.sync_globals(&profile);
            self.handle(Some(pid), requests);
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.drain_names();
        self.remote_frames();
        self.sync(now);
        self.lifetime(now);
    }

    /// Quits on request, or once no game is left (see `QUIT_*`).
    fn lifetime(&mut self, now: Instant) {
        let running = !self.shared.games.is_empty();
        if self.sessions.iter().any(|s| s.state.connected()) {
            self.seen_game = true;
        }
        if running {
            self.empty_since = None;
        } else {
            let since = *self.empty_since.get_or_insert(now);
            let limit = if self.seen_game {
                QUIT_AFTER_GAMES
            } else {
                QUIT_WITHOUT_GAME
            };
            if !self.quit && now.duration_since(since) >= limit && now.duration_since(self.started) >= limit {
                tracing::info!(seen_game = self.seen_game, "no game left, closing");
                self.quit = true;
            }
        }
        // eframe shows its window after the first frame; keep it hidden
        // unless there is a fault to show.
        if !self.show_window
            && self
                .hide_check
                .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(250))
        {
            self.hide_check = Some(now);
            crate::process::show_own_windows(false);
        }
        // Faults the in-game panel cannot show (it needs a working agent).
        // A client that would not start belongs here: its game never gets an
        // agent to tell the user.
        let fault = self.sessions.iter().any(|s| {
            matches!(
                s.state.link,
                Link::Failed(
                    Fault::MissingFiles | Fault::AccessDenied | Fault::LaunchFailed(_) | Fault::Protocol(_)
                )
            )
        }) || matches!(
            self.idle.link,
            Link::Failed(Fault::MissingFiles | Fault::LaunchFailed(_))
        );
        if fault && !self.show_window {
            tracing::warn!("showing the window for a fault");
            self.show_window = true;
            self.ctx.request_repaint();
        }
        if self.quit {
            self.ctx.request_repaint();
        }
    }

    /// Stops every bot and takes the agents out of the games.
    fn shutdown(&mut self) {
        for session in &mut self.sessions {
            if let Some(store) = session.account.as_mut() {
                store.flush(&session.state.profile);
            }
            if session.state.connected() {
                let pid = session.pid();
                self.supervisor.send(pid, AppMessage::Control(Control::Stop));
                self.supervisor.send(pid, AppMessage::Unload);
            }
        }
        self.store.flush(&self.global);
    }

    /// Only shown for faults the in-game panel cannot show.
    fn window(&mut self, ui: &mut egui::Ui) {
        let tabs = self.tabs();
        let current = self.current();
        let (pid, requests, profile) = {
            let state = match current {
                Some(index) => &mut self.sessions[index].state,
                None => &mut self.idle,
            };
            state.tabs = tabs;
            ui::shell(ui, state);
            (
                state.pid,
                std::mem::take(&mut state.requests),
                state.profile.clone(),
            )
        };
        self.sync_globals(&profile);
        self.handle(pid, requests);
    }

    fn exit(&mut self) {
        self.store.flush(&self.global);
        let mut stop = false;
        for session in &mut self.sessions {
            if let Some(store) = session.account.as_mut() {
                store.flush(&session.state.profile);
            }
            if session.state.connected() && session.state.intent.is_some() {
                self.supervisor
                    .send(session.pid(), AppMessage::Control(Control::Stop));
                stop = true;
            }
        }
        if stop {
            std::thread::sleep(Duration::from_millis(150));
        }
    }
}

fn level_of(link: Link) -> Level {
    match link {
        Link::Connected(_) => Level::Success,
        Link::Failed(_) => Level::Error,
        _ => Level::Info,
    }
}

fn run_core(core: &Mutex<Core>, notes: &Receiver<Note>) {
    loop {
        let first = match notes.recv_timeout(CORE_TICK) {
            Ok(note) => Some(note),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut core = core.lock();
        if let Some(note) = first {
            core.note(note);
        }
        while let Ok(note) = notes.try_recv() {
            core.note(note);
        }
        core.tick();
    }
}

pub struct KrakenApp {
    core: Arc<Mutex<Core>>,
    closing: bool,
    shown: bool,
}

impl KrakenApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let (store, global) = Store::open(data_dir().join("profile.toml"));
        i18n::apply(&global.locale);
        let mode = Mode {
            launch: global.launch,
            parent_spoof: global.parent_spoof,
        };
        let (supervisor, notes) = Supervisor::spawn(cc.egui_ctx.clone(), global.auto_attach, mode);
        let names = gamedata::spawn(cc.egui_ctx.clone());
        let idle = State {
            profile: global.clone(),
            link: Link::Searching,
            ..State::default()
        };
        let core = Arc::new(Mutex::new(Core {
            auto_attach: global.auto_attach,
            mode,
            global,
            store,
            sessions: Vec::new(),
            idle,
            selected: None,
            shared: Shared::default(),
            supervisor,
            names,
            ctx: cc.egui_ctx.clone(),
            started: Instant::now(),
            seen_game: false,
            empty_since: None,
            quit: false,
            show_window: false,
            hide_check: None,
        }));
        let worker = core.clone();
        std::thread::Builder::new()
            .name("kraken-core".into())
            .spawn(move || run_core(&worker, &notes))
            .expect("core thread");
        Self {
            core,
            closing: false,
            shown: false,
        }
    }
}

impl eframe::App for KrakenApp {
    /// Runs while the window is hidden too: quitting and showing faults.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut core = self.core.lock();
        if core.quit && !self.closing {
            self.closing = true;
            core.shutdown();
            drop(core);
            // Give the pipes a moment to deliver Stop and Unload.
            std::thread::sleep(Duration::from_millis(300));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if core.show_window && !self.shown {
            self.shown = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut core = self.core.lock();
        core.window(ui);
        let connected = core.sessions.iter().any(|s| s.state.connected());
        let wait = if connected {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(500)
        };
        let wait = core.store.wants_repaint().map_or(wait, |d| d.min(wait));
        ui.ctx().request_repaint_after(wait);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.core.lock().exit();
    }
}

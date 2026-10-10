use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use kraken_proto::{AgentMessage, AppMessage, PROTO_VERSION, pipe_names, read_frame, write_frame};

use crate::process::{self, GameProcess};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TICK: Duration = Duration::from_millis(250);
const SCAN_EVERY: Duration = Duration::from_secs(1);
const CONNECT_WINDOW: Duration = Duration::from_secs(20);
const AGENT_NAMES: [&str; 2] = ["kraken-agent.dll", "kraken_agent.dll"];
const INJECTOR_NAME: &str = "kraken-injector.exe";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    MissingFiles,
    AccessDenied,
    NotGame,
    InjectFailed(i32),
    LaunchFailed(i32),
    Protocol(u32),
}

/// How Kraken's own client arrives: started by Kraken (`launch`) or picked
/// from what the user already opened, and whether the parent process is
/// hidden behind explorer.exe while it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub launch: bool,
    pub parent_spoof: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    #[default]
    Idle,
    Searching,
    /// A client is being started; the game does not exist yet.
    Launching,
    Injecting(u32),
    Connecting(u32),
    Connected(u32),
    Failed(Fault),
}

impl Link {
    pub fn pid(self) -> Option<u32> {
        match self {
            Self::Injecting(pid) | Self::Connecting(pid) | Self::Connected(pid) => Some(pid),
            _ => None,
        }
    }
}

pub enum Note {
    /// What the Kraken instance is doing with no game of its own (the
    /// window shows this while a client starts).
    Status(Link),
    Link(u32, Link),
    Games(Vec<GameProcess>),
    Agent(u32, AgentMessage),
    /// The game process is gone; its session can be dropped.
    Gone(u32),
}

enum Order {
    Attach(u32),
    Detach(u32),
    Auto(bool),
    Mode(Mode),
    /// Open the Seafight launcher (the button).
    Launch,
    Send(u32, AppMessage),
}

/// Talks to every running game client at once: each gets its own agent,
/// pipes and link state.
pub struct Supervisor {
    orders: Sender<Order>,
}

impl Supervisor {
    pub fn spawn(ctx: egui::Context, auto: bool, mode: Mode) -> (Self, Receiver<Note>) {
        let (orders, inbox) = channel();
        let (notes, outbox) = channel();
        let worker = Worker::new(ctx, notes, inbox, auto, mode);
        std::thread::Builder::new()
            .name("kraken-supervisor".into())
            .spawn(move || worker.run())
            .expect("supervisor thread");
        (Self { orders }, outbox)
    }

    pub fn attach(&self, pid: u32) {
        let _ = self.orders.send(Order::Attach(pid));
    }

    pub fn detach(&self, pid: u32) {
        let _ = self.orders.send(Order::Detach(pid));
    }

    pub fn set_auto(&self, auto: bool) {
        let _ = self.orders.send(Order::Auto(auto));
    }

    pub fn set_mode(&self, mode: Mode) {
        let _ = self.orders.send(Order::Mode(mode));
    }

    pub fn launch(&self) {
        let _ = self.orders.send(Order::Launch);
    }

    pub fn send(&self, pid: u32, message: AppMessage) {
        let _ = self.orders.send(Order::Send(pid, message));
    }
}

pub struct Paths {
    pub injector: PathBuf,
    pub agent: PathBuf,
}

fn search_dirs() -> Vec<PathBuf> {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return Vec::new();
    };
    let mut dirs = vec![dir.clone()];
    if let Some(target) = dir.parent() {
        let profile = dir.file_name().map(PathBuf::from).unwrap_or_default();
        dirs.push(target.join("i686-pc-windows-msvc").join(&profile));
        if let Some(root) = target.parent() {
            dirs.push(root.join("i686-pc-windows-msvc").join(&profile));
        }
    }
    dirs
}

const EMBEDDED_AGENT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agent.bin"));
const EMBEDDED_INJECTOR: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/injector.bin"));

/// The parts built into kraken.exe, written out once per build to
/// `%LOCALAPPDATA%\Kraken\bin\<hash>`.
fn unpack_embedded() -> Option<Paths> {
    if EMBEDDED_AGENT.is_empty() || EMBEDDED_INJECTOR.is_empty() {
        return None;
    }
    let hash = EMBEDDED_AGENT
        .iter()
        .chain(EMBEDDED_INJECTOR)
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
    let dir = crate::profile::data_dir()
        .join("bin")
        .join(format!("{hash:016x}"));
    std::fs::create_dir_all(&dir).ok()?;
    let write = |name: &str, bytes: &[u8]| -> Option<PathBuf> {
        let path = dir.join(name);
        if std::fs::metadata(&path)
            .ok()
            .is_none_or(|m| m.len() != bytes.len() as u64)
        {
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, bytes).ok()?;
            std::fs::rename(&temp, &path).ok()?;
        }
        Some(path)
    };
    Some(Paths {
        agent: write(AGENT_NAMES[0], EMBEDDED_AGENT)?,
        injector: write(INJECTOR_NAME, EMBEDDED_INJECTOR)?,
    })
}

pub fn locate() -> Option<Paths> {
    if let Some(paths) = unpack_embedded() {
        return Some(paths);
    }
    search_dirs().into_iter().find_map(|dir| {
        let injector = dir.join(INJECTOR_NAME);
        let agent = AGENT_NAMES
            .iter()
            .map(|name| dir.join(name))
            .find(|p| p.is_file())?;
        injector.is_file().then_some(Paths { injector, agent })
    })
}

fn staged_agent(source: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(source).ok()?;
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    let dir = crate::profile::data_dir().join("agents");
    std::fs::create_dir_all(&dir).ok()?;
    let staged = dir.join(format!("kraken-agent-{hash:016x}.dll"));
    if !staged.is_file() {
        let temp = staged.with_extension("tmp");
        std::fs::write(&temp, &bytes).ok()?;
        std::fs::rename(&temp, &staged).ok()?;
    }
    // Hidden and read-only: nothing inside a waiting session can swap the
    // agent for another DLL, and Explorer never lists it.
    set_hidden(&staged);
    Some(staged)
}

fn set_hidden(path: &Path) {
    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let attributes = windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN
        | windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
    unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(), attributes);
    }
}

enum Phase {
    /// Not wanted (auto-attach off, or detached by the user).
    Idle,
    Injecting {
        at: Instant,
    },
    Connecting {
        until: Instant,
    },
    Connected {
        writer: BufWriter<File>,
        closed: Arc<AtomicBool>,
    },
}

struct Session {
    game: GameProcess,
    phase: Phase,
    link: Link,
    /// The user detached this game; auto-attach leaves it alone.
    detached: bool,
}

struct Worker {
    ctx: egui::Context,
    notes: Sender<Note>,
    orders: Receiver<Order>,
    auto: bool,
    mode: Mode,
    /// The launcher is due at this point: once at start in launch mode, and
    /// again when the user presses the button.
    launch_at: Option<Instant>,
    sessions: Vec<Session>,
    games: Vec<GameProcess>,
    scanned: Option<Instant>,
}

impl Worker {
    fn new(ctx: egui::Context, notes: Sender<Note>, orders: Receiver<Order>, auto: bool, mode: Mode) -> Self {
        Self {
            ctx,
            notes,
            orders,
            auto,
            mode,
            launch_at: (auto && mode.launch).then(Instant::now),
            sessions: Vec::new(),
            games: Vec::new(),
            scanned: None,
        }
    }

    fn note(&self, note: Note) {
        let _ = self.notes.send(note);
        self.ctx.request_repaint();
    }

    fn set_link(&self, session: &mut Session, link: Link) {
        if session.link != link {
            session.link = link;
            self.note(Note::Link(session.game.pid, link));
        }
    }

    fn set_status(&self, link: Link) {
        self.note(Note::Status(link));
    }

    fn run(mut self) {
        loop {
            match self.orders.recv_timeout(TICK) {
                Ok(order) => self.order(order),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            while let Ok(order) = self.orders.try_recv() {
                self.order(order);
            }
            self.step(Instant::now());
        }
    }

    fn order(&mut self, order: Order) {
        match order {
            Order::Attach(pid) => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.game.pid == pid) {
                    session.detached = false;
                    if matches!(session.phase, Phase::Idle) {
                        session.phase = Phase::Injecting { at: Instant::now() };
                    }
                }
            }
            Order::Detach(pid) => {
                let mut sessions = std::mem::take(&mut self.sessions);
                if let Some(session) = sessions.iter_mut().find(|s| s.game.pid == pid) {
                    session.detached = true;
                    session.phase = Phase::Idle;
                    self.set_link(session, Link::Idle);
                }
                self.sessions = sessions;
            }
            Order::Auto(auto) => self.auto = auto,
            Order::Mode(mode) => self.mode = mode,
            Order::Launch => self.launch_at = Some(Instant::now()),
            Order::Send(pid, message) => {
                if let Some(session) = self.sessions.iter_mut().find(|s| s.game.pid == pid)
                    && let Phase::Connected { writer, closed } = &mut session.phase
                    && write_frame(writer, &message).is_err()
                {
                    closed.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    fn scan(&mut self, now: Instant) {
        if self.scanned.is_some_and(|at| now.duration_since(at) < SCAN_EVERY) {
            return;
        }
        self.scanned = Some(now);
        let games = process::games();
        if games != self.games {
            self.games = games.clone();
            self.note(Note::Games(games));
        }
        for session in &mut self.sessions {
            // A client we started may not have reported its start time yet;
            // with it, the next scan finds the same session again.
            if session.game.started == 0
                && let Some(started) = process::started_of(session.game.pid)
            {
                session.game.started = started;
            }
        }
        for game in &self.games {
            if !self.sessions.iter().any(|s| s.game == *game) {
                self.sessions.push(Session {
                    game: *game,
                    phase: Phase::Idle,
                    link: Link::Idle,
                    detached: false,
                });
            }
        }
        let gone: Vec<u32> = self
            .sessions
            .iter()
            .filter(|s| !process::alive(s.game))
            .map(|s| s.game.pid)
            .collect();
        for pid in gone {
            tracing::info!(pid, "game process exited");
            self.sessions.retain(|s| s.game.pid != pid);
            self.note(Note::Gone(pid));
        }
    }

    fn step(&mut self, now: Instant) {
        self.scan(now);
        let mut sessions = std::mem::take(&mut self.sessions);
        for session in &mut sessions {
            if self.auto && !session.detached && matches!(session.phase, Phase::Idle) {
                session.phase = Phase::Injecting { at: now };
            }
            let phase = std::mem::replace(&mut session.phase, Phase::Idle);
            session.phase = match phase {
                Phase::Idle => {
                    if !matches!(session.link, Link::Failed(_)) {
                        self.set_link(session, Link::Idle);
                    }
                    Phase::Idle
                }
                Phase::Injecting { at } => self.inject(session, at, now),
                Phase::Connecting { until } => self.connect(session, until, now),
                Phase::Connected { writer, closed } => {
                    if closed.load(Ordering::SeqCst) {
                        let pid = session.game.pid;
                        tracing::info!(pid, "agent pipe closed");
                        self.set_link(session, Link::Connecting(pid));
                        Phase::Connecting {
                            until: now + CONNECT_WINDOW,
                        }
                    } else {
                        Phase::Connected { writer, closed }
                    }
                }
            };
        }
        self.sessions = sessions;
        if let Some(at) = self.launch_at
            && now >= at
        {
            self.launch_at = None;
            self.start_launcher();
        }
    }

    /// Opens the Seafight login window unless one is already open. The
    /// user signs in and presses Play; the launcher starts the game client,
    /// and the next scan finds it and injects the agent into it.
    fn start_launcher(&self) {
        if process::launcher_running() {
            tracing::info!("launcher already open");
            self.set_status(Link::Searching);
            return;
        }
        let Some(launcher) = process::launcher_path() else {
            tracing::warn!("Seafight launcher not found");
            self.set_status(Link::Failed(Fault::LaunchFailed(-1)));
            return;
        };
        self.set_status(Link::Launching);
        let spoof = self.mode.parent_spoof;
        match process::start_launcher(&launcher, spoof) {
            Ok(()) => {
                tracing::info!(spoof, "launcher started");
                self.set_status(Link::Searching);
            }
            Err(error) => {
                tracing::warn!(%error, "launcher start failed");
                self.set_status(Link::Failed(Fault::LaunchFailed(
                    error.raw_os_error().unwrap_or(-1),
                )));
            }
        }
    }

    fn inject(&self, session: &mut Session, at: Instant, now: Instant) -> Phase {
        if now < at {
            return Phase::Injecting { at };
        }
        let pid = session.game.pid;
        let Some(paths) = locate() else {
            self.set_link(session, Link::Failed(Fault::MissingFiles));
            return Phase::Injecting {
                at: now + Duration::from_secs(5),
            };
        };
        self.set_link(session, Link::Injecting(pid));
        let agent = staged_agent(&paths.agent).unwrap_or_else(|| paths.agent.clone());
        let status = Command::new(&paths.injector)
            .arg(pid.to_string())
            .arg(&agent)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        let code = status.map_or(-1, |s| s.code().unwrap_or(-1));
        tracing::info!(pid, code, "injector finished");
        let retry = |secs: u64| Phase::Injecting {
            at: now + Duration::from_secs(secs),
        };
        match code {
            0 => {
                self.set_link(session, Link::Connecting(pid));
                Phase::Connecting {
                    until: now + CONNECT_WINDOW,
                }
            }
            4 => retry(2),
            2 => {
                self.set_link(session, Link::Failed(Fault::AccessDenied));
                retry(10)
            }
            3 => {
                self.set_link(session, Link::Failed(Fault::NotGame));
                retry(30)
            }
            other => {
                self.set_link(session, Link::Failed(Fault::InjectFailed(other)));
                retry(5)
            }
        }
    }

    fn connect(&mut self, session: &mut Session, until: Instant, now: Instant) -> Phase {
        let pid = session.game.pid;
        match open_pipes(pid) {
            Some((writer, reader)) => {
                let closed = Arc::new(AtomicBool::new(false));
                let mut writer = BufWriter::new(writer);
                if write_frame(
                    &mut writer,
                    &AppMessage::Hello {
                        version: PROTO_VERSION,
                    },
                )
                .is_err()
                {
                    return Phase::Connecting { until };
                }
                spawn_reader(pid, reader, closed.clone(), self.notes.clone(), self.ctx.clone());
                tracing::info!(pid, "agent connected");
                self.set_link(session, Link::Connected(pid));
                Phase::Connected { writer, closed }
            }
            None if now >= until => {
                tracing::warn!(pid, "agent pipe not available, injecting again");
                Phase::Injecting { at: now }
            }
            None => Phase::Connecting { until },
        }
    }
}

fn open_pipes(pid: u32) -> Option<(File, File)> {
    let (down, up) = pipe_names(pid);
    let Ok(writer) = OpenOptions::new().write(true).open(&down) else {
        let _ = OpenOptions::new().read(true).open(&up);
        return None;
    };
    let started = Instant::now();
    loop {
        if let Ok(reader) = OpenOptions::new().read(true).open(&up) {
            return Some((writer, reader));
        }
        if started.elapsed() > Duration::from_secs(3) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn spawn_reader(pid: u32, reader: File, closed: Arc<AtomicBool>, notes: Sender<Note>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let mut stream = BufReader::new(reader);
        loop {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            match read_frame::<AgentMessage, _>(&mut stream) {
                Ok(message) => {
                    if let AgentMessage::Hello { version, .. } = &message
                        && *version != PROTO_VERSION
                    {
                        let _ = notes.send(Note::Link(pid, Link::Failed(Fault::Protocol(*version))));
                    }
                    if notes.send(Note::Agent(pid, message)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                Err(error) => {
                    tracing::info!(pid, %error, "agent stream ended");
                    break;
                }
            }
        }
        closed.store(true, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

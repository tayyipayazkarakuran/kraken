use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Instant;

use kraken_proto::names::{NameKind, NameTable};
use kraken_proto::{
    ActionItemEntry, AgentMessage, AppMessage, AtlasEntry, BindingReport, BonusMapEntry, Capabilities,
    Catalog, CatalogEntry, EntityKind, Event, EventCode, Hero, ItemEntry, Level, MapInfo, MapRegion,
    NearbyIds, PROTO_VERSION, RunState, ScreenMarkers, SessionState, StockKind, Telemetry, VisibleCounts,
};
use parking_lot::Mutex;
use retour::RawDetour;

use crate::engine::Engine;
use crate::engine::world::{Action, Feedback};
use crate::game::messages::MessageMap;
use crate::game::{Game, Scan};
use crate::il2cpp::{Method, Ptr};

const SCAN_MS: u64 = 100;
/// War mode reacts to the fight, so it scans about every other frame.
const WAR_SCAN_MS: u64 = 33;
const TELEMETRY_MS: u64 = 150;
const CATALOG_MS: u64 = 2_000;
const FAULT_REPEAT_MS: u64 = 10_000;
const DIAG_MS: u64 = 30_000;
const LOCA_RETRY_MS: u64 = 30_000;
const TRACE_MS: u64 = 60_000;

type TickFn = unsafe extern "C" fn(Ptr, Ptr);
type DispatchFn = unsafe extern "C" fn(Ptr, Ptr, Ptr);

static AGENT: Mutex<Option<Agent>> = Mutex::new(None);
static FEEDBACK: Mutex<Vec<Feedback>> = Mutex::new(Vec::new());
static MESSAGES: OnceLock<MessageMap> = OnceLock::new();
static TICK: OnceLock<TickFn> = OnceLock::new();
static DISPATCH: OnceLock<DispatchFn> = OnceLock::new();
static DETOURS: Mutex<Vec<Detour>> = Mutex::new(Vec::new());
static DORMANT: AtomicBool = AtomicBool::new(false);
/// After a skill trigger, every server message is logged until then, to see
/// how the server answers (or refuses).
static WATCH_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
const WATCH_SECS: u64 = 3;

struct Detour(RawDetour);

unsafe impl Send for Detour {}

#[derive(Default)]
struct Book {
    language: String,
    names: NameTable,
    loca_at: Option<u64>,
    npcs: BTreeMap<i32, String>,
    monsters: BTreeMap<i32, String>,
    boxes: BTreeMap<i32, String>,
    ammo: BTreeMap<i16, String>,
    harpoons: BTreeMap<i16, String>,
    torpedoes: BTreeMap<i16, String>,
    items: Vec<ActionItemEntry>,
    bonus_maps: Vec<BonusMapEntry>,
    atlas: Vec<AtlasEntry>,
    dirty: bool,
}

fn name_kind(kind: EntityKind) -> Option<NameKind> {
    match kind {
        EntityKind::Npc => Some(NameKind::Npc),
        EntityKind::Monster => Some(NameKind::Monster),
        EntityKind::Box => Some(NameKind::Box),
        _ => None,
    }
}

impl Book {
    fn load(&mut self, now: u64, game: &mut Game) {
        if !self.names.is_empty()
            || self
                .loca_at
                .is_some_and(|t| now.saturating_sub(t) < LOCA_RETRY_MS)
        {
            return;
        }
        self.loca_at = Some(now);
        if let Some(loca) = game.loca() {
            self.language = loca.language;
            self.names = loca.names;
            self.dirty = true;
        }
    }

    /// Replaces runtime display names with the game's localised type names so
    /// events and the catalog agree with the in-game labels.
    fn rename(&self, scan: &mut Scan) {
        for actor in &mut scan.world.actors {
            if let Some(name) = name_kind(actor.kind).and_then(|k| self.names.get(k, actor.type_id)) {
                name.clone_into(&mut actor.name);
            }
        }
    }

    fn learn(&mut self, scan: &Scan) {
        for actor in &scan.world.actors {
            let table = match actor.kind {
                EntityKind::Npc => &mut self.npcs,
                EntityKind::Monster => &mut self.monsters,
                EntityKind::Box => &mut self.boxes,
                _ => continue,
            };
            if table.get(&actor.type_id) != Some(&actor.name) {
                table.insert(actor.type_id, actor.name.clone());
                self.dirty = true;
            }
        }
        for stock in &scan.world.stock {
            let (table, kind) = match stock.kind {
                StockKind::Ammo => (&mut self.ammo, NameKind::Ammo),
                StockKind::Harpoon => (&mut self.harpoons, NameKind::Harpoon),
                StockKind::Torpedo => (&mut self.torpedoes, NameKind::Torpedo),
                // Wallet names come from the app's game data.
                StockKind::Currency | StockKind::Points | StockKind::Token | StockKind::BonusMap => continue,
            };
            let name = self
                .names
                .get(kind, i32::from(stock.id))
                .map_or_else(|| format!("#{}", stock.id), str::to_owned);
            if table.get(&stock.id) != Some(&name) {
                table.insert(stock.id, name);
                self.dirty = true;
            }
        }
    }

    fn learn_items(&mut self, scan: &Scan) {
        let items: Vec<ActionItemEntry> = scan
            .world
            .items
            .iter()
            .map(|item| ActionItemEntry {
                id: item.id,
                name: self
                    .names
                    .get(NameKind::ActionItem, i32::from(item.id))
                    .map_or_else(|| format!("#{}", item.id), str::to_owned),
                amount: item.amount,
                active: item.active,
            })
            .collect();
        if items != self.items {
            self.items = items;
            self.dirty = true;
        }
        let maps: Vec<BonusMapEntry> = scan
            .world
            .bonus
            .iter()
            .map(|map| BonusMapEntry {
                id: map.id,
                name: self
                    .names
                    .get(NameKind::Map, i32::from(map.id))
                    .map_or_else(|| format!("#{}", map.id), str::to_owned),
                amount: map.amount,
                waves: map.waves,
            })
            .collect();
        if maps != self.bonus_maps {
            self.bonus_maps = maps;
            self.dirty = true;
        }
        if scan.world.atlas.len() != self.atlas.len()
            || scan
                .world
                .atlas
                .iter()
                .zip(&self.atlas)
                .any(|(c, e)| c.id != e.id)
            || scan
                .world
                .atlas
                .iter()
                .zip(&self.atlas)
                .any(|(c, e)| c.label != e.label)
        {
            let atlas: Vec<AtlasEntry> = scan
                .world
                .atlas
                .iter()
                .map(|c| AtlasEntry {
                    id: c.id,
                    label: c.label.clone(),
                    // Map ids of the world map overlap the bonus maps' loca
                    // labels; only safe havens have a name of their own.
                    name: if c.safe_haven {
                        self.names
                            .get(NameKind::Map, i32::from(c.id))
                            .unwrap_or_default()
                            .to_owned()
                    } else {
                        String::new()
                    },
                    region: if c.world == 1 {
                        MapRegion::Atlantis
                    } else if c.label.to_uppercase().starts_with("RÖK")
                        || c.label.to_uppercase().starts_with("ROK")
                    {
                        MapRegion::Ragnarok
                    } else {
                        MapRegion::Nautica
                    },
                    world: c.world,
                    row: c.row,
                    col: c.col,
                    safe_haven: c.safe_haven,
                    min_level: c.min_level,
                })
                .collect();
            if atlas != self.atlas {
                self.atlas = atlas;
                self.dirty = true;
            }
        }
    }

    fn catalog(&self) -> Catalog {
        let entries = |seen: &BTreeMap<i32, String>, kind: NameKind| {
            let mut all = self.names.table(kind).clone();
            for (id, name) in seen {
                all.entry(*id).or_insert_with(|| name.clone());
            }
            all.into_iter()
                .map(|(type_id, name)| CatalogEntry { type_id, name })
                .collect()
        };
        let items = |seen: &BTreeMap<i16, String>, kind: NameKind| {
            let mut all: BTreeMap<i16, String> = self
                .names
                .table(kind)
                .iter()
                .filter_map(|(id, name)| Some((i16::try_from(*id).ok()?, name.clone())))
                .collect();
            for (id, name) in seen {
                all.entry(*id).or_insert_with(|| name.clone());
            }
            all.into_iter().map(|(id, name)| ItemEntry { id, name }).collect()
        };
        Catalog {
            language: self.language.clone(),
            npcs: entries(&self.npcs, NameKind::Npc),
            monsters: entries(&self.monsters, NameKind::Monster),
            boxes: entries(&self.boxes, NameKind::Box),
            ammo: items(&self.ammo, NameKind::Ammo),
            harpoons: items(&self.harpoons, NameKind::Harpoon),
            torpedoes: items(&self.torpedoes, NameKind::Torpedo),
            items: self.items.clone(),
            bonus_maps: self.bonus_maps.clone(),
            atlas: self.atlas.clone(),
        }
    }
}

pub struct Agent {
    game: Game,
    engine: Engine,
    inbox: Receiver<AppMessage>,
    outbox: SyncSender<AgentMessage>,
    network: Ptr,
    clock: Instant,
    last_scan: u64,
    last_telemetry: u64,
    last_catalog: u64,
    scan: Scan,
    report: BindingReport,
    book: Book,
    frame_us: u32,
    scan_us: u32,
    faults: BTreeMap<String, u64>,
    last_diag: Vec<String>,
    last_diag_at: u64,
    seen: BTreeMap<&'static str, u32>,
    seen_at: u64,
    /// Work map edges from earlier sessions were handed to the engine.
    edges_seeded: bool,
}

unsafe impl Send for Agent {}

impl Agent {
    pub fn new(game: Game, inbox: Receiver<AppMessage>, outbox: SyncSender<AgentMessage>) -> Self {
        let seed = std::process::id() as u64 ^ 0x9e37_79b9_7f4a_7c15;
        Self {
            game,
            engine: Engine::new(seed),
            inbox,
            outbox,
            network: std::ptr::null_mut(),
            clock: Instant::now(),
            last_scan: 0,
            last_telemetry: 0,
            last_catalog: 0,
            scan: Scan::default(),
            report: BindingReport::default(),
            book: Book::default(),
            frame_us: 0,
            scan_us: 0,
            faults: BTreeMap::new(),
            last_diag: Vec::new(),
            last_diag_at: 0,
            seen: BTreeMap::new(),
            seen_at: 0,
            edges_seeded: false,
        }
    }

    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    fn send(&self, message: AgentMessage) {
        let _ = self.outbox.try_send(message);
    }

    fn event(&self, event: Event) {
        self.send(AgentMessage::Event(event));
    }

    fn caps(&self) -> Capabilities {
        self.report.capabilities
    }

    fn adopt(&mut self, network: Ptr) -> bool {
        if network == self.network {
            return true;
        }
        let replace =
            self.network.is_null() || (!self.game.connected(self.network) && self.game.connected(network));
        if replace {
            self.network = network;
            self.report = self.game.report(network);
            self.send(AgentMessage::Bindings(self.report.clone()));
            for missing in &self.report.missing {
                self.event(Event::new(Level::Warning, EventCode::BindingMissing).arg(missing));
            }
            self.event(Event::new(Level::Success, EventCode::AgentReady));
        }
        replace
    }

    fn frame(&mut self, network: Ptr) {
        if !self.adopt(network) {
            return;
        }
        let started = Instant::now();
        let now = self.now();
        while let Ok(message) = self.inbox.try_recv() {
            self.command(now, message);
        }
        let feedback = std::mem::take(&mut *FEEDBACK.lock());
        let war = self.engine.config().mode.war();
        // El Dorado is timed and wave maps need quick fire: scan as often as in a fight.
        let eldorado = self
            .scan
            .world
            .map
            .as_ref()
            .is_some_and(|m| m.id == kraken_proto::ELDORADO_MAP || m.kind_name == "WAVE")
            || self.engine.fast_map();
        // A sunk or lost war target is replaced in this very frame.
        let urgent = (war
            && feedback.iter().any(|f| match f {
                Feedback::Removed(id) => self.engine.war_target().is_some_and(|t| t.matches(*id)),
                Feedback::AttackEnded | Feedback::AttackAborted | Feedback::OwnDeath => true,
                Feedback::Effect { .. } => self.engine.holds_hero(f),
                _ => false,
            }))
            // A collected chest: head for the next one in this very frame.
            || (eldorado
                && feedback
                    .iter()
                    .any(|f| matches!(f, Feedback::Removed(_) | Feedback::MapChanged | Feedback::Hit { .. })));
        for item in feedback {
            *self.seen.entry(feedback_label(&item)).or_default() += 1;
            self.engine.feedback(now, item);
        }
        if now.saturating_sub(self.seen_at) >= TRACE_MS {
            self.seen_at = now;
            let action_panel = self.game.view("ActionMenuPanel").is_some();
            let death_window = self.game.death_window().is_some();
            let hero = self.scan.world.hero.as_ref();
            tracing::info!(
                feedback = ?self.seen,
                repair_block = ?hero.and_then(|h| h.repair_block),
                action_panel,
                death_window,
                run = ?self.engine.run_state(),
                activity = ?self.engine.activity(),
                hp = hero.map_or(0.0, |h| h.hp),
                hp_max = hero.map_or(0.0, |h| h.hp_max),
                "minute"
            );
            self.seen.clear();
        }
        let interval = if war || eldorado { WAR_SCAN_MS } else { SCAN_MS };
        if urgent || now.saturating_sub(self.last_scan) >= interval {
            self.last_scan = now;
            let scanning = Instant::now();
            crate::il2cpp::reset_guard();
            self.scan = self.game.scan(self.network);
            if self.scan.world.ready {
                self.book.load(now, &mut self.game);
            }
            self.book.rename(&mut self.scan);
            let shape = |d: &[String]| {
                d.iter()
                    .map(|s| s.contains("=false") || s.contains("none"))
                    .collect::<Vec<_>>()
            };
            let changed = shape(&self.scan.diag) != shape(&self.last_diag);
            if changed
                || (self.scan.diag != self.last_diag && now.saturating_sub(self.last_diag_at) >= DIAG_MS)
            {
                tracing::info!(stages = %self.scan.diag.join(" | "), "scan");
                self.last_diag.clone_from(&self.scan.diag);
                self.last_diag_at = now;
            }
            self.scan_us = scanning.elapsed().as_micros() as u32;
            let caps = self.caps();
            let actions = {
                let terrain = self.game.terrain();
                self.engine.step(now, &self.scan.world, &caps, &terrain)
            };
            self.perform(now, actions);
            if !self.edges_seeded && !self.scan.world.atlas.is_empty() {
                self.edges_seeded = true;
                let edges = crate::game::load_edges();
                tracing::info!(edges = edges.len(), "work map edges loaded");
                self.engine.seed_edges(&edges, &self.scan.world.atlas);
            }
            if let Some(edges) = self.engine.take_learned_edges() {
                crate::game::save_edges(&edges);
            }
            for event in self.engine.drain_events() {
                tracing::info!(code = ?event.code, args = ?event.args, "event");
                self.event(event);
            }
            self.book.learn(&self.scan);
            self.book.learn_items(&self.scan);
        }
        if now.saturating_sub(self.last_telemetry) >= TELEMETRY_MS {
            self.last_telemetry = now;
            let telemetry = self.telemetry(now);
            self.send(AgentMessage::Telemetry(Box::new(telemetry)));
        }
        if self.book.dirty && now.saturating_sub(self.last_catalog) >= CATALOG_MS {
            self.last_catalog = now;
            self.book.dirty = false;
            self.send(AgentMessage::Catalog(Box::new(self.book.catalog())));
        }
        self.frame_us = started.elapsed().as_micros() as u32;
    }

    fn command(&mut self, now: u64, message: AppMessage) {
        match message {
            AppMessage::Hello { version } => {
                self.send(AgentMessage::Hello {
                    version: PROTO_VERSION,
                    pid: std::process::id(),
                });
                if version != PROTO_VERSION {
                    tracing::warn!(version, "controller protocol mismatch");
                }
                self.send(AgentMessage::Bindings(self.report.clone()));
                self.send(AgentMessage::Catalog(Box::new(self.book.catalog())));
            }
            AppMessage::Configure(config) => self.engine.configure(*config),
            AppMessage::Control(control) => {
                let actions = self.engine.control(now, control, &self.scan.world);
                self.perform(now, actions);
            }
            AppMessage::RequestCatalog => self.send(AgentMessage::Catalog(Box::new(self.book.catalog()))),
            AppMessage::Frame(frame) => crate::overlay::submit(*frame),
            AppMessage::Unload => {
                self.event(Event::new(Level::Warning, EventCode::Unloading));
                std::thread::spawn(crate::unload);
            }
            AppMessage::DumpTypes => match crate::dump::write(&self.game.registry) {
                Ok(path) => {
                    self.event(Event::new(Level::Success, EventCode::TypesDumped).arg(path.display()));
                }
                Err(error) => self.event(Event::new(Level::Error, EventCode::Fault).arg(error)),
            },
        }
    }

    fn perform(&mut self, now: u64, actions: Vec<Action>) {
        for action in actions {
            if !matches!(action, Action::Move(_)) {
                tracing::info!(?action, "action");
            }
            if let Action::TriggerSkill(id) = action {
                let hero = self.scan.world.hero.as_ref();
                tracing::info!(
                    id,
                    skills = ?self.scan.world.skills,
                    selected = ?hero.and_then(|h| h.selected),
                    moving = ?hero.and_then(|h| h.moving),
                    "skill trigger: state"
                );
                *WATCH_UNTIL.lock() = Some(Instant::now() + std::time::Duration::from_secs(WATCH_SECS));
            }
            if let Err(error) = self.game.execute(self.network, action) {
                tracing::warn!(?action, %error, "action failed");
                let due = self
                    .faults
                    .get(&error)
                    .is_none_or(|t| now.saturating_sub(*t) >= FAULT_REPEAT_MS);
                if due {
                    self.faults.insert(error.clone(), now);
                    self.event(Event::new(Level::Error, EventCode::Fault).arg(error));
                }
            }
        }
    }

    fn telemetry(&self, now: u64) -> Telemetry {
        let world = &self.scan.world;
        let run = self.engine.run_state();
        let session = if !world.connected {
            if run == RunState::Running && self.engine.config().safety.reconnect {
                SessionState::Reconnecting
            } else {
                SessionState::Disconnected
            }
        } else if world.ready {
            SessionState::Ready
        } else {
            SessionState::Loading
        };
        let mut visible = VisibleCounts::default();
        let mut nearby = NearbyIds::default();
        for actor in world.actors.iter().filter(|a| a.visible) {
            let ids = match actor.kind {
                EntityKind::Player => {
                    visible.players += 1;
                    continue;
                }
                EntityKind::Npc => {
                    visible.npcs += 1;
                    &mut nearby.npcs
                }
                EntityKind::Monster => {
                    visible.monsters += 1;
                    &mut nearby.monsters
                }
                EntityKind::Box => {
                    visible.boxes += 1;
                    &mut nearby.boxes
                }
                _ => continue,
            };
            if !ids.contains(&actor.type_id) {
                ids.push(actor.type_id);
            }
        }
        Telemetry {
            run,
            session,
            activity: self.engine.activity(),
            map: world.map.as_ref().map(|m| MapInfo {
                id: m.id,
                kind: m.kind_name.clone(),
                width: m.width as i32,
                height: m.height as i32,
            }),
            hero: world.hero.as_ref().map(|h| Hero {
                user_id: h.id.id,
                name: h.name.clone(),
                position: h.pos,
                hp: h.hp,
                hp_max: h.hp_max,
                moving: h.moving.unwrap_or(false),
                cannon_range: h.cannon_range,
                harpoon_range: h.harpoon_range,
                vp: h.vp,
                vp_max: h.vp_max,
            }),
            target: self.engine.target(world),
            waypoint: self.engine.waypoint(),
            counters: self.engine.counters(now),
            visible,
            nearby,
            stock: world.stock.clone(),
            screen: self.screen(),
            war: self.engine.war_info(world),
            eldorado: self.engine.eldorado_info(now, world),
            bonus: self.engine.bonus_info(now, world),
            workmap: self.engine.workmap_info(world),
            frame_us: self.frame_us,
            scan_us: self.scan_us,
        }
    }

    fn screen(&self) -> Option<ScreenMarkers> {
        if !self.caps().screen {
            return None;
        }
        let (width, height) = self.game.screen_size()?;
        let flip = |p: kraken_proto::Point| kraken_proto::Point::new(p.x, height - p.y);
        let hero_world = self.scan.hero_world?;
        let hero = self.game.screen_point(hero_world).map(flip);
        let target = self
            .engine
            .target_id()
            .and_then(|id| {
                self.scan
                    .actor_world
                    .iter()
                    .find(|(k, _)| k.matches(id))
                    .map(|(_, w)| *w)
            })
            .and_then(|w| self.game.screen_point(w))
            .map(flip);
        let waypoint = self
            .engine
            .waypoint()
            .zip(self.scan.map_to_world)
            .map(|(p, fit)| fit.apply(p))
            .and_then(|w| {
                self.game.screen_point(crate::game::Vec3 {
                    x: w.x,
                    y: w.y,
                    z: hero_world.z,
                })
            })
            .map(flip);
        let range_ring = self.range_ring(hero_world.z).into_iter().map(flip).collect();
        Some(ScreenMarkers {
            width,
            height,
            hero,
            target,
            waypoint,
            range_ring,
        })
    }

    /// The hero's cannon range as a ring of screen points; empty unless every
    /// point projects (a partial ring would be drawn wrong).
    fn range_ring(&self, z: f32) -> Vec<kraken_proto::Point> {
        const SEGMENTS: usize = 32;
        let (Some(hero), Some(fit)) = (self.scan.world.hero.as_ref(), self.scan.map_to_world) else {
            return Vec::new();
        };
        let radius = hero.cannon_range;
        if radius <= 0.0 {
            return Vec::new();
        }
        (0..SEGMENTS)
            .map(|i| {
                let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let point = kraken_proto::Point::new(
                    hero.pos.x + radius * angle.cos(),
                    hero.pos.y + radius * angle.sin(),
                );
                let w = fit.apply(point);
                self.game.screen_point(crate::game::Vec3 { x: w.x, y: w.y, z })
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default()
    }
}

fn feedback_label(feedback: &Feedback) -> &'static str {
    match feedback {
        Feedback::Hit { .. } => "hit",
        Feedback::Removed(_) => "removed",
        Feedback::AttackStarted => "attack_started",
        Feedback::AttackEnded => "attack_ended",
        Feedback::AttackAborted => "attack_aborted",
        Feedback::OwnDeath => "death",
        Feedback::MapChanged => "map_changed",
        Feedback::Spawned => "spawned",
        Feedback::Disconnected(_) => "disconnected",
        Feedback::Effect { .. } => "effect",
        Feedback::SkillTriggered { .. } => "skill_triggered",
        Feedback::SkillCharge { .. } => "skill_charge",
        Feedback::MapDelay { .. } => "map_delay",
        Feedback::MapAsk { .. } => "map_ask",
        Feedback::MapCancel => "map_cancel",
    }
}

fn on_tick(network: Ptr) {
    let Some(mut guard) = AGENT.try_lock() else { return };
    if let Some(agent) = guard.as_mut() {
        agent.frame(network);
    }
}

fn on_message(message: Ptr) {
    let watching = WATCH_UNTIL.lock().is_some_and(|t| Instant::now() < t);
    if watching && let Some(class) = crate::il2cpp::Class::of(message) {
        tracing::info!(message = class.name(), "after skill trigger");
    }
    let Some(map) = MESSAGES.get() else { return };
    if let Some(feedback) = map.translate(message) {
        FEEDBACK.lock().push(feedback);
    }
}

unsafe extern "C" fn tick_detour(this: Ptr, method: Ptr) {
    if let Some(original) = TICK.get() {
        unsafe { original(this, method) };
    }
    if !DORMANT.load(Ordering::Acquire) {
        let _ = catch_unwind(AssertUnwindSafe(|| on_tick(this)));
    }
}

unsafe extern "C" fn dispatch_detour(this: Ptr, message: Ptr, method: Ptr) {
    if let Some(original) = DISPATCH.get() {
        unsafe { original(this, message, method) };
    }
    if !DORMANT.load(Ordering::Acquire) {
        let _ = catch_unwind(AssertUnwindSafe(|| on_message(message)));
    }
}

fn hook(target: Method, detour: *const (), store: impl FnOnce(*const ())) -> Result<(), String> {
    let raw = unsafe { RawDetour::new(target.code(), detour) }.map_err(|e| e.to_string())?;
    store(raw.trampoline() as *const ());
    unsafe { raw.enable() }.map_err(|e| e.to_string())?;
    DETOURS.lock().push(Detour(raw));
    Ok(())
}

pub fn shutdown() {
    DORMANT.store(true, Ordering::Release);
    drop(AGENT.lock().take());
    for detour in DETOURS.lock().drain(..) {
        if let Err(error) = unsafe { detour.0.disable() } {
            tracing::warn!(%error, "detour disable failed");
        }
        std::mem::forget(detour);
    }
}

pub fn install(
    agent: Agent,
    messages: MessageMap,
    tick: Method,
    dispatch: Option<Method>,
) -> Result<(), String> {
    let _ = MESSAGES.set(messages);
    *AGENT.lock() = Some(agent);
    if let Some(dispatch) = dispatch {
        let stored = hook(dispatch, dispatch_detour as *const (), |original| {
            let _ = DISPATCH.set(unsafe { std::mem::transmute::<*const (), DispatchFn>(original) });
        });
        if let Err(error) = stored {
            tracing::warn!(%error, "dispatch hook unavailable");
        }
    }
    hook(tick, tick_detour as *const (), |original| {
        let _ = TICK.set(unsafe { std::mem::transmute::<*const (), TickFn>(original) });
    })
}

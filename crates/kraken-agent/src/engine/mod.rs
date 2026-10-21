mod bonus;
mod eldorado;
pub mod geometry;
mod items;
pub mod nav;
mod retreat;
mod targets;
mod war;
mod workmap;
pub mod world;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use kraken_proto::{
    Activity, Capabilities, Config, Control, Counters, EntityKind, Event, EventCode, Level, MixedPriority,
    Point, RangeStyle, RoamMode, RunState, StockKind, TargetInfo, WorkMode,
};

use geometry::{Geometry, Rng, ray};
use nav::{Navigator, Owner};
use targets::{Choice, Rejection};
use world::{Action, Actor, DisconnectKind, Eid, Feedback, HeroState, MapKind, RepairBlock, Terrain, World};

const ROAM_MARGIN: f32 = 0.08;
const SKIP_TARGET_MS: u64 = 60_000;
const SKIP_BOX_MS: u64 = 30_000;
const NO_HIT_SKIP_MS: u64 = 20_000;
const KILL_WINDOW_MS: u64 = 15_000;
const BOX_SETTLE_MS: u64 = 2_000;
const BOX_TIMEOUT_MS: u64 = 45_000;
const COLLECT_RADIUS: f32 = 1.5;
const COLLECT_CONFIRM_RADIUS: f32 = 4.0;
const DAMAGE_QUIET_MS: u64 = 4_000;
const REPAIR_RETRY_MS: u64 = 3_000;
const REPAIR_TRIES: u32 = 4;
const REPAIR_BACKOFF_MS: u64 = 10_000;
const REPAIR_OFF_MS: u64 = 60_000;
const REPAIR_STALL_MS: u64 = 20_000;
/// A ship that hit the hero this recently is where the danger is.
const ATTACKER_RECENT_MS: u64 = 6_000;
const ATTACKER_MEMORY_MS: u64 = 8_000;
const RESPAWN_DELAY_MS: u64 = 3_000;
const RESPAWN_RETRY_MS: u64 = 8_000;
const SIGHT_MIN: f32 = 60.0;
const SIGHT_FACTOR: f32 = 3.0;
const KEEP_SIGHT: f32 = 1.5;
const DETOUR_REACH: f32 = 0.85;
const DETOUR_ABANDON: f32 = 0.95;
const ATTACK_INTERVAL_MS: u64 = 300;
const REFIRE_UNCONFIRMED_MS: u64 = 2_500;
const REFIRE_SILENT_MS: u64 = 6_000;
/// Between two torpedo orders (the launchers' own cooldown decides the rest).
const TORPEDO_GAP_MS: u64 = 900;
/// A load request not reflected in the preset by then is sent again.
const TORPEDO_LOAD_RETRY_MS: u64 = 3_000;
const WARN_REPEAT_MS: u64 = 60_000;
/// After this many failed fast retries (~2.5 min of the game being down), a
/// generic outage is treated as a long one and waits the configured delay.
const RETRY_ESCALATE: u32 = 6;

#[derive(Clone, Debug)]
struct Engagement {
    id: Eid,
    kind: EntityKind,
    type_id: i32,
    name: String,
    weapon: i16,
    /// Fought with torpedoes (Atlantis); `weapon` is the torpedo, 0 = any.
    torpedo: bool,
    last_fire: u64,
    last_hit: u64,
    hits: u32,
    last_hp: Option<f32>,
    last_progress: u64,
    in_range_since: Option<u64>,
    removed: bool,
    last_pos: Point,
    /// No shot before this time: a human's reaction delay on a new target
    /// (0 when humanization is off).
    ready_at: u64,
    /// The jittered gap this engagement waits before its next shot; refreshed
    /// after every shot so the cadence never repeats exactly.
    refire_gap: u64,
}

#[derive(Clone, Debug)]
struct Collecting {
    id: Eid,
    name: String,
    type_id: i32,
    pos: Point,
    since: u64,
    arrived_at: Option<u64>,
    removed: bool,
    detour_until: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Repair {
    Idle,
    /// Fleeing to `corner`, chosen while the enemy was at `dir` (its bearing
    /// to the hero); kept until the enemy comes round or the way is blocked.
    Retreat {
        corner: Option<Point>,
        dir: f32,
    },
    Request {
        tries: u32,
        last: u64,
        hp: f64,
    },
    Active {
        last_rise: u64,
        hp: f64,
    },
}

impl Repair {
    const RETREAT: Self = Self::Retreat {
        corner: None,
        dir: 0.0,
    };
}

#[derive(Clone, Copy, Debug)]
struct Death {
    since: u64,
    tries: u32,
    last: Option<u64>,
    spawned: bool,
}

#[derive(Default)]
struct Recovery {
    offline_since: Option<u64>,
    reason: Option<String>,
    attempts: u32,
    next_at: u64,
    blocked: bool,
}

#[derive(Default)]
struct Roam {
    goal: Option<Point>,
    seq: u32,
    cursor: usize,
    map: Option<i32>,
}

pub struct Engine {
    cfg: Config,
    run: RunState,
    activity: Activity,
    nav: Navigator,
    rng: Rng,
    /// A separate stream for humanization jitter, so pulling random pacing
    /// values never shifts the navigation/roam RNG (and its tests).
    human: Rng,
    /// The last game clock seen, to catch the clock jumping backwards.
    last_now: Option<u64>,
    /// Paused because the clock went backwards; cleared once it recovers.
    clock_paused: bool,
    roam: Roam,
    engage: Option<Engagement>,
    collect: Option<Collecting>,
    skips: HashMap<Eid, u64>,
    repair: Repair,
    repair_after_spawn: bool,
    repair_off_until: Option<u64>,
    /// Who hit the hero last, and when.
    attackers: HashMap<Eid, u64>,
    /// Where the last target was when a repair began.
    last_target_pos: Option<Point>,
    /// Flight corners found unreachable during this repair.
    repair_skip: Vec<Point>,
    death: Option<Death>,
    recovery: Recovery,
    attack_active: bool,
    last_damage: Option<u64>,
    last_attack_cmd: Option<u64>,
    torpedo_load_at: Option<u64>,
    hero_id: Eid,
    /// Last seen hero position, for feedback that arrives between scans.
    hero_pos: Option<Point>,
    map_id: Option<i32>,
    counters: Counters,
    run_since: Option<u64>,
    run_ms: u64,
    warned: HashMap<(EventCode, i64), u64>,
    events: Vec<Event>,
    war: war::War,
    items: items::ItemRunner,
    eldorado: eldorado::ElDorado,
    bonus: bonus::Bonus,
    workmap: workmap::Workmap,
}

impl Engine {
    pub fn new(seed: u64) -> Self {
        Self {
            cfg: Config::default(),
            run: RunState::Stopped,
            activity: Activity::Idle,
            nav: Navigator::default(),
            rng: Rng::new(seed),
            human: Rng::new(seed ^ 0x2545_f491_4f6c_dd1d),
            last_now: None,
            clock_paused: false,
            roam: Roam::default(),
            engage: None,
            collect: None,
            skips: HashMap::new(),
            repair: Repair::Idle,
            repair_after_spawn: false,
            repair_off_until: None,
            attackers: HashMap::new(),
            last_target_pos: None,
            repair_skip: Vec::new(),
            death: None,
            recovery: Recovery::default(),
            attack_active: false,
            last_damage: None,
            last_attack_cmd: None,
            torpedo_load_at: None,
            hero_id: Eid::default(),
            hero_pos: None,
            map_id: None,
            counters: Counters::default(),
            run_since: None,
            run_ms: 0,
            warned: HashMap::new(),
            events: Vec::new(),
            war: war::War::default(),
            items: items::ItemRunner::default(),
            eldorado: eldorado::ElDorado::default(),
            bonus: bonus::Bonus::default(),
            workmap: workmap::Workmap::default(),
        }
    }

    pub fn configure(&mut self, cfg: Config) {
        // Rule state is keyed by position; edited lists start over.
        let npc_rules = |c: &Config| c.npc.targets.iter().map(|t| t.items.clone()).collect::<Vec<_>>();
        let monster_rules = |c: &Config| {
            c.monster
                .targets
                .iter()
                .map(|t| t.items.clone())
                .collect::<Vec<_>>()
        };
        if cfg.items != self.cfg.items
            || npc_rules(&cfg) != npc_rules(&self.cfg)
            || monster_rules(&cfg) != monster_rules(&self.cfg)
        {
            self.items.clear();
        }
        if cfg.navigation != self.cfg.navigation {
            self.roam = Roam::default();
        }
        if cfg.mode != self.cfg.mode {
            self.war.reset();
        }
        self.cfg = cfg;
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn run_state(&self) -> RunState {
        self.run
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn control(&mut self, now: u64, control: Control, world: &World) -> Vec<Action> {
        let mut out = Vec::new();
        match control {
            Control::Start => {
                if self.run != RunState::Running {
                    self.run_since = Some(now);
                }
                if self.run == RunState::Stopped {
                    self.bonus.restart();
                    self.counters = Counters::default();
                    self.run_ms = 0;
                    self.skips.clear();
                }
                self.recovery.blocked = false;
                self.run = RunState::Running;
            }
            Control::Pause => self.halt(now, world, RunState::Paused, &mut out),
            Control::Stop => self.halt(now, world, RunState::Stopped, &mut out),
        }
        out
    }

    fn halt(&mut self, now: u64, world: &World, state: RunState, out: &mut Vec<Action>) {
        if self.run == RunState::Running {
            self.run_ms += self.run_since.map_or(0, |t| now.saturating_sub(t));
        }
        self.run_since = None;
        self.run = state;
        self.eldorado.reset();
        self.bonus.reset();
        self.workmap.reset();
        if self.cfg.mode.war() {
            // The player keeps the helm and the guns; nothing to undo.
            self.war.reset();
            self.activity = Activity::Idle;
            return;
        }
        if self.engage.is_some() || self.attack_active {
            out.push(Action::AbortAttack);
        }
        if let Some(hero) = &world.hero {
            out.extend(self.nav.stop(now, hero));
        }
        self.reset_work();
        self.repair = Repair::Idle;
        self.repair_after_spawn = false;
        self.death = None;
        self.activity = Activity::Idle;
    }

    fn reset_work(&mut self) {
        self.engage = None;
        self.collect = None;
        self.nav.clear();
        self.roam.goal = None;
    }

    pub fn feedback(&mut self, now: u64, feedback: Feedback) {
        match feedback {
            Feedback::Hit { source, target } => {
                if self.hero_id.is_set() && target.matches(self.hero_id) {
                    self.last_damage = Some(now);
                    if source.is_set() && !source.matches(self.hero_id) {
                        self.attackers.insert(source, now);
                    }
                }
                if self.hero_id.is_set() && source.matches(self.hero_id) {
                    self.bonus_hit(target);
                }
                if self.hero_id.is_set()
                    && source.matches(self.hero_id)
                    && let Some(e) = self.engage.as_mut().filter(|e| e.id.matches(target))
                {
                    e.last_hit = now;
                    e.hits += 1;
                }
            }
            Feedback::Removed(id) => {
                self.war.removed(id);
                self.eldorado_removed(id, self.hero_pos);
                self.bonus_removed(id);
                if let Some(e) = self.engage.as_mut().filter(|e| e.id.matches(id)) {
                    e.removed = true;
                }
                if let Some(c) = self.collect.as_mut().filter(|c| c.id.matches(id)) {
                    c.removed = true;
                }
            }
            Feedback::AttackStarted => self.attack_active = true,
            Feedback::AttackEnded | Feedback::AttackAborted => {
                self.attack_active = false;
                if let Some(e) = self.engage.as_mut() {
                    e.last_fire = e.last_fire.min(now.saturating_sub(REFIRE_UNCONFIRMED_MS));
                }
            }
            Feedback::OwnDeath => self.enter_death(now),
            Feedback::MapChanged | Feedback::Spawned => {
                self.reset_work();
                self.attackers.clear();
                self.war.reset();
                self.attack_active = false;
                self.repair = Repair::Idle;
                if let Some(d) = self.death.as_mut() {
                    d.spawned = true;
                }
            }
            Feedback::Disconnected(reason) => self.recovery.reason = Some(reason),
            Feedback::MapDelay { map, delay_ms } => self.bonus_delay(now, map, delay_ms),
            Feedback::MapAsk { map, confirm } => self.workmap_ask(now, map, confirm),
            Feedback::MapCancel => self.workmap_cancel(),
            Feedback::SkillCharge { id, charge } => {
                tracing::info!(id, charge, "master skill charge");
                self.eldorado_charge(id, charge);
            }
            Feedback::SkillTriggered { id, remaining_ms } => {
                tracing::info!(id, remaining_ms, "master skill triggered");
                self.eldorado_triggered(now, id, remaining_ms);
            }
            Feedback::Effect {
                name,
                target,
                active,
                millis,
            } => {
                if self.hero_id.is_set() && target.matches(self.hero_id) {
                    tracing::info!(effect = %name, active, ?millis, "hero effect");
                    self.war.effect(now, &name, active, millis);
                }
            }
        }
    }

    fn enter_death(&mut self, now: u64) {
        if self.death.is_none() {
            self.death = Some(Death {
                since: now,
                tries: 0,
                last: None,
                spawned: false,
            });
            self.counters.deaths += 1;
            self.reset_work();
            if self.eldorado.phase != kraken_proto::ElDoradoPhase::Run {
                self.eldorado.reset();
            }
            self.attack_active = false;
            self.repair = Repair::Idle;
            self.emit(Level::Error, EventCode::Died, &[]);
        }
    }

    fn emit(&mut self, level: Level, code: EventCode, args: &[String]) {
        self.events.push(Event {
            level,
            code,
            args: args.to_vec(),
        });
    }

    fn warn_once(&mut self, now: u64, code: EventCode, key: i64, args: &[String]) {
        let due = self
            .warned
            .get(&(code, key))
            .is_none_or(|t| now.saturating_sub(*t) >= WARN_REPEAT_MS);
        if due {
            self.warned.insert((code, key), now);
            self.emit(Level::Warning, code, args);
        }
    }

    fn pause(&mut self, now: u64, world: &World, out: &mut Vec<Action>) {
        self.halt(now, world, RunState::Paused, out);
    }

    /// A human's reaction delay before the first shot at a new target.
    fn reaction_delay(&mut self) -> u64 {
        let h = self.cfg.humanize;
        if !h.enabled || h.reaction_ms == 0 {
            return 0;
        }
        self.human.range_u64(0, u64::from(h.reaction_ms))
    }

    /// A jittered amount added to the refire thresholds, so repeated shots
    /// never land on an exact machine cadence. Zero when humanization is off.
    fn refire_jitter(&mut self) -> u64 {
        let h = self.cfg.humanize;
        if !h.enabled || h.jitter_pct == 0 {
            return 0;
        }
        let span = REFIRE_UNCONFIRMED_MS * u64::from(h.jitter_pct) / 100;
        self.human.range_u64(0, span)
    }

    /// True once (and stays) when the game clock has run backwards; the step
    /// loop then holds every action until the clock is sane again.
    fn clock_anomaly(&mut self, now: u64) -> bool {
        if !self.cfg.humanize.enabled || !self.cfg.humanize.clock_guard {
            self.last_now = Some(now);
            return false;
        }
        let back = self.last_now.is_some_and(|last| now + 2_000 < last);
        if back && !self.clock_paused {
            self.clock_paused = true;
            self.emit(Level::Warning, EventCode::ClockAnomaly, &[]);
        } else if !back && self.clock_paused && self.last_now.is_some_and(|last| now >= last) {
            self.clock_paused = false;
        }
        self.last_now = Some(now);
        self.clock_paused
    }

    pub fn step(
        &mut self,
        now: u64,
        world: &World,
        caps: &Capabilities,
        terrain: &dyn Terrain,
    ) -> Vec<Action> {
        if self.clock_anomaly(now) {
            self.activity = Activity::Recovering;
            return Vec::new();
        }
        let mut out = self.step_work(now, world, caps, terrain);
        if world.playable() {
            self.item_rules(now, world, caps, &mut out);
        }
        out
    }

    fn step_work(
        &mut self,
        now: u64,
        world: &World,
        caps: &Capabilities,
        terrain: &dyn Terrain,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        self.skips.retain(|_, until| *until > now);

        if !world.connected {
            self.offline(now, world, caps, &mut out);
            return out;
        }
        if self.recovery.offline_since.is_some() {
            if !world.playable() {
                self.activity = Activity::Recovering;
                return out;
            }
            self.recovery = Recovery::default();
            self.emit(Level::Success, EventCode::Reconnected, &[]);
        }
        let (Some(hero), Some(map)) = (world.hero.as_ref(), world.map.as_ref()) else {
            self.activity = Activity::Syncing;
            return out;
        };
        if !world.ready {
            self.activity = Activity::Syncing;
            return out;
        }
        self.hero_id = hero.id;
        self.hero_pos = Some(hero.pos);
        self.nav
            .set_fast(Self::on_eldorado(map) || self.on_bonus(world, map));
        if self.map_id != Some(map.id) {
            let moved = self.map_id.is_some();
            let from = self.map_id;
            if moved {
                self.emit(Level::Info, EventCode::MapChanged, &[map.id.to_string()]);
            }
            self.map_id = Some(map.id);
            self.reset_work();
            self.eldorado_map(now, from, map.id);
            self.bonus_map(world, map, moved);
            self.workmap_map_changed(from, map.id, world);
            // Leaving the death map is the respawn, whether or not its
            // network message was seen.
            if moved
                && map.kind != MapKind::Death
                && let Some(d) = self.death.as_mut()
            {
                d.spawned = true;
            }
        }
        if self.run != RunState::Running {
            self.activity = Activity::Idle;
            return out;
        }
        if self.cfg.mode.war() {
            self.death = None;
            return self.war_step(now, world, hero, caps);
        }

        if hero.dead() || map.kind == MapKind::Death {
            self.enter_death(now);
        }
        if self.death.is_some() {
            self.dead(now, world, hero, map.kind, caps, &mut out);
            if self.death.is_some() {
                return out;
            }
        }
        let geometry = Geometry::new(map.width, map.height);
        // The bonus map is timed: no repairs, only chests.
        if Self::on_eldorado(map) && self.cfg.eldorado.enabled && caps.movement {
            if self.eldorado.phase != kraken_proto::ElDoradoPhase::Run {
                self.eldorado_map(now, None, map.id);
            }
            self.eldorado_run(now, world, hero, geometry, terrain, &mut out);
            return out;
        }
        if self.run == RunState::Running && self.bonus.inside.is_none() {
            self.bonus_map(world, map, false);
        }
        if self.cfg.bonus.enabled && self.bonus.inside == Some(map.id) && caps.cannon {
            self.bonus_fight(now, world, hero, map, geometry, terrain, &mut out);
            return out;
        }
        if self.repairing(now, world, hero, map.width, map.height, caps, terrain, &mut out) {
            return out;
        }
        if map.kind == MapKind::SafeHaven && self.cfg.navigation.avoid_safe_haven {
            self.activity = Activity::Idle;
            out.extend(self.nav.stop(now, hero));
            self.reset_work();
            self.warn_once(
                now,
                EventCode::SafeHaven,
                i64::from(map.id),
                &[map.id.to_string()],
            );
            return out;
        }
        if caps.movement && self.bonus_prepare(now, world, hero, map, geometry, terrain, &mut out) {
            return out;
        }
        if caps.movement && self.eldorado_prepare(now, world, hero, map, geometry, terrain, &mut out) {
            return out;
        }
        if caps.movement && self.workmap_step(now, world, hero, map, geometry, terrain, &mut out) {
            return out;
        }
        self.work(now, world, hero, geometry, caps, terrain, &mut out);
        out
    }

    fn offline(&mut self, now: u64, world: &World, caps: &Capabilities, out: &mut Vec<Action>) {
        self.activity = Activity::Recovering;
        if self.recovery.offline_since.is_none() {
            let reason = self.recovery.reason.clone().unwrap_or_default();
            let kind = DisconnectKind::classify(&reason);
            self.recovery.offline_since = Some(now);
            self.recovery.attempts = 0;
            self.recovery.next_at = now + self.backoff(kind, 0);
            self.reset_work();
            self.attack_active = false;
            self.repair = Repair::Idle;
            self.emit(Level::Warning, EventCode::Disconnected, &[reason]);
        }
        if self.run != RunState::Running || self.recovery.blocked {
            return;
        }
        if !self.cfg.safety.reconnect || !caps.reconnect {
            return;
        }
        let reason = self.recovery.reason.clone().unwrap_or_default();
        let kind = DisconnectKind::classify(&reason);
        if kind == DisconnectKind::Fatal {
            self.recovery.blocked = true;
            self.emit(Level::Error, EventCode::ReconnectBlocked, &[reason]);
            self.pause(now, world, out);
            self.activity = Activity::Recovering;
            return;
        }
        if now >= self.recovery.next_at {
            self.recovery.attempts += 1;
            let wait = self.backoff(kind, self.recovery.attempts);
            self.recovery.next_at = now + wait;
            out.push(Action::Reconnect);
            self.emit(
                Level::Info,
                EventCode::Reconnecting,
                &[self.recovery.attempts.to_string()],
            );
            tracing::info!(
                attempt = self.recovery.attempts,
                next_in_secs = wait / 1_000,
                ?kind,
                "reconnect scheduled"
            );
        }
    }

    /// How long to wait before the next reconnect try. A server reset or
    /// maintenance (`Wait`) waits the full configured delay before the first
    /// try; a generic outage retries fast at first, then — once enough tries
    /// have failed, i.e. the game has been down a while — also settles into
    /// the long wait instead of hammering the server.
    fn backoff(&self, kind: DisconnectKind, attempt: u32) -> u64 {
        let long = u64::from(self.cfg.safety.reconnect_wait_mins.max(1)) * 60_000;
        match kind {
            DisconnectKind::Wait => long,
            _ if attempt >= RETRY_ESCALATE => long,
            _ => (10_000 * u64::from(attempt + 1)).min(30_000),
        }
    }

    fn dead(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        map: MapKind,
        caps: &Capabilities,
        out: &mut Vec<Action>,
    ) {
        self.activity = Activity::Dead;
        let Some(mut death) = self.death else { return };
        let settled = now.saturating_sub(death.since) >= RESPAWN_DELAY_MS || death.tries > 0;
        if death.spawned && settled && map != MapKind::Death && hero.alive() {
            self.death = None;
            self.repair_after_spawn = true;
            self.repair_off_until = None;
            self.repair = Repair::Idle;
            self.nav.clear();
            out.push(Action::CloseDeathWindow);
            return;
        }
        if !self.cfg.safety.respawn || !caps.respawn {
            return;
        }
        let waiting = death
            .last
            .is_some_and(|t| now.saturating_sub(t) < RESPAWN_RETRY_MS);
        if now.saturating_sub(death.since) < RESPAWN_DELAY_MS || waiting {
            return;
        }
        if death.tries >= 3 {
            self.emit(Level::Error, EventCode::RespawnFailed, &[]);
            self.pause(now, world, out);
            return;
        }
        death.tries += 1;
        death.last = Some(now);
        self.death = Some(death);
        out.push(Action::Respawn);
        self.emit(Level::Info, EventCode::Respawning, &[death.tries.to_string()]);
    }

    /// Where the danger is while repairing: the ships that hit the hero
    /// lately (nearer ones weigh more), else the last target, else the
    /// nearest hostile ship in sight.
    fn repair_threat(&mut self, now: u64, world: &World, hero: &HeroState) -> Option<Point> {
        self.attackers
            .retain(|_, at| now.saturating_sub(*at) < ATTACKER_MEMORY_MS);
        let (mut x, mut y, mut weight) = (0.0, 0.0, 0.0);
        for (id, at) in &self.attackers {
            if now.saturating_sub(*at) >= ATTACKER_RECENT_MS {
                continue;
            }
            if let Some(a) = world.actor(*id).filter(|a| a.visible) {
                let w = 1.0 / (1.0 + a.pos.distance(hero.pos));
                x += a.pos.x * w;
                y += a.pos.y * w;
                weight += w;
            }
        }
        if weight > 0.0 {
            return Some(Point::new(x / weight, y / weight));
        }
        if let Some(p) = self.last_target_pos {
            return Some(p);
        }
        world
            .actors
            .iter()
            .filter(|a| matches!(a.kind, EntityKind::Npc | EntityKind::Monster | EntityKind::Player))
            .filter(|a| a.visible && !a.id.matches(hero.id) && !a.relation.friendly())
            .min_by(|a, b| a.pos.distance(hero.pos).total_cmp(&b.pos.distance(hero.pos)))
            .map(|a| a.pos)
    }

    #[allow(clippy::too_many_arguments)]
    fn repairing(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        width: f32,
        height: f32,
        caps: &Capabilities,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) -> bool {
        let safety = &self.cfg.safety;
        if !safety.repair || !caps.repair || !hero.hp_known || hero.hp_max <= 0.0 {
            self.repair = Repair::Idle;
            return false;
        }
        let pct = hero.hp_percent();
        let (start, end) = (
            f32::from(safety.repair_start),
            f32::from(safety.repair_end.max(safety.repair_start)),
        );
        if self.repair == Repair::Idle {
            let off = self.repair_off_until.is_some_and(|t| now < t);
            if off || hero.repair_block == Some(RepairBlock::HealthFull) {
                return false;
            }
            let after_spawn = std::mem::take(&mut self.repair_after_spawn);
            if pct >= start && !(after_spawn && pct < end) {
                return false;
            }
            if self.engage.is_some() || self.attack_active {
                out.push(Action::AbortAttack);
            }
            self.last_target_pos = self.engage.as_ref().map(|e| e.last_pos);
            self.repair_skip.clear();
            self.reset_work();
            self.repair = Repair::RETREAT;
            self.emit(Level::Warning, EventCode::RepairStarted, &[format!("{pct:.0}")]);
        }
        if pct >= end {
            out.push(Action::Repair(false));
            self.repair = Repair::Idle;
            self.nav.clear();
            self.emit(Level::Success, EventCode::RepairFinished, &[format!("{pct:.0}")]);
            return false;
        }
        self.activity = Activity::Repairing;
        let recently_hit = self
            .last_damage
            .is_some_and(|t| now.saturating_sub(t) < DAMAGE_QUIET_MS);
        match self.repair {
            Repair::Idle => {}
            Repair::Retreat { corner, dir } => {
                match self.repair_threat(now, world, hero).filter(|_| recently_hit) {
                    Some(threat) => {
                        let geometry = Geometry::new(width, height);
                        let (corner, dir) = match (corner, geometry) {
                            (Some(c), _) if !retreat::turned(dir, hero.pos, threat) => (Some(c), dir),
                            (_, Some(g)) => {
                                let picked =
                                    retreat::flee_corner(hero.pos, threat, g, terrain, &self.repair_skip);
                                tracing::info!(pos = ?hero.pos, ?threat, corner = ?picked, "repair flee corner");
                                (picked, geometry::angle(threat, hero.pos))
                            }
                            (_, None) => (None, dir),
                        };
                        self.repair = Repair::Retreat { corner, dir };
                        // No corner to go to: straight away from the enemy.
                        let goal = corner.unwrap_or_else(|| {
                            let away = ray(threat, hero.pos, threat.distance(hero.pos) + 30.0);
                            geometry.map_or(away, |g| g.clamp(away, ROAM_MARGIN))
                        });
                        let step = self.nav.drive(now, hero, goal, Owner::Repair, geometry, terrain);
                        if corner.is_some() && (step.blocked || self.nav.stuck_count() >= 3) {
                            self.repair_skip.push(goal);
                            self.nav.clear();
                            self.repair = Repair::RETREAT;
                        } else if !step.arrived && hero.pos.distance(goal) > retreat::CORNER_REACHED {
                            out.extend(step.action);
                        }
                    }
                    None if recently_hit => {}
                    None => {
                        out.extend(self.nav.stop(now, hero));
                        out.push(Action::CloseDeathWindow);
                        self.repair = Repair::Request {
                            tries: 0,
                            last: 0,
                            hp: hero.hp,
                        };
                    }
                }
            }
            Repair::Request { tries, last, hp } => {
                if recently_hit {
                    self.repair = Repair::RETREAT;
                } else if hero.repairing == Some(true) || hero.hp > hp + 0.5 {
                    self.repair = Repair::Active {
                        last_rise: now,
                        hp: hero.hp,
                    };
                } else {
                    match hero.repair_block {
                        Some(RepairBlock::HealthFull) => {
                            self.repair = Repair::Idle;
                            return false;
                        }
                        Some(RepairBlock::NoCarpenter) => {
                            self.warn_once(now, EventCode::RepairStalled, 1, &["no_carpenter".into()]);
                            self.repair = Repair::Idle;
                            self.repair_off_until = Some(now + REPAIR_OFF_MS);
                            return false;
                        }
                        Some(RepairBlock::Attacking) => {
                            if now.saturating_sub(last) >= REPAIR_RETRY_MS {
                                out.push(Action::AbortAttack);
                                self.repair = Repair::Request { tries, last: now, hp };
                            }
                        }
                        Some(block @ (RepairBlock::Breach | RepairBlock::Cooldown)) => {
                            let reason = if block == RepairBlock::Breach {
                                "breach"
                            } else {
                                "cooldown"
                            };
                            self.warn_once(now, EventCode::RepairStalled, 2, &[reason.into()]);
                        }
                        None if now.saturating_sub(last) >= REPAIR_RETRY_MS => {
                            if tries >= REPAIR_TRIES {
                                self.warn_once(now, EventCode::RepairStalled, 0, &[]);
                                self.repair = Repair::Request {
                                    tries: 0,
                                    last: now + REPAIR_BACKOFF_MS,
                                    hp,
                                };
                            } else {
                                out.push(Action::Repair(true));
                                self.repair = Repair::Request {
                                    tries: tries + 1,
                                    last: now,
                                    hp,
                                };
                            }
                        }
                        None => {}
                    }
                }
            }
            Repair::Active { last_rise, hp } => {
                if self.last_damage.is_some_and(|t| now.saturating_sub(t) < 1_000) {
                    self.repair = Repair::RETREAT;
                } else if hero.hp > hp {
                    self.repair = Repair::Active {
                        last_rise: now,
                        hp: hero.hp,
                    };
                } else if now.saturating_sub(last_rise) > REPAIR_STALL_MS {
                    self.warn_once(now, EventCode::RepairStalled, 0, &[]);
                    self.repair = Repair::Request {
                        tries: 0,
                        last: now,
                        hp: hero.hp,
                    };
                }
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn work(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        geometry: Option<Geometry>,
        caps: &Capabilities,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        self.settle_engagement(now, world, hero, out);
        self.settle_collecting(now, world, hero);

        let mode = self.cfg.mode;
        let collect_first = mode == WorkMode::Mixed && self.cfg.mixed_priority == MixedPriority::Collect;
        let can_fight = mode.combat() && (caps.cannon || caps.harpoon);
        let can_collect = mode.collect() && caps.movement && self.cfg.collector.enabled;

        let combat = if can_fight {
            self.combat_choice(now, world, hero, caps)
        } else {
            None
        };
        let gather = if can_collect {
            self.box_choice(world, hero)
        } else {
            None
        };

        // A fight in progress is never dropped for a box; boxes met while
        // fighting are only taken as short detours that keep the target in range.
        let engaged = combat
            .as_ref()
            .is_some_and(|c| self.engage.as_ref().is_some_and(|e| e.id.matches(c.id)));
        let fight = match (combat.is_some(), gather.is_some()) {
            (true, true) => engaged || !collect_first,
            (true, false) => true,
            _ => false,
        };

        if fight {
            let choice = combat.expect("combat choice");
            if self.collect.as_ref().is_some_and(|c| c.detour_until.is_none()) {
                self.collect = None;
            }
            self.fight(now, world, hero, choice, geometry, terrain, out);
            return;
        }
        if let Some(choice) = gather {
            if self.engage.take().is_some() || self.attack_active {
                out.push(Action::AbortAttack);
                self.attack_active = false;
            }
            self.gather(now, world, hero, choice, geometry, terrain, out);
            return;
        }
        if self.engage.take().is_some() {
            out.push(Action::AbortAttack);
        }
        self.collect = None;
        self.wander(now, hero, geometry, caps, terrain, out);
    }

    fn settle_engagement(&mut self, now: u64, world: &World, hero: &HeroState, out: &mut Vec<Action>) {
        let Some(e) = self.engage.clone() else { return };
        let actor = world.actor(e.id);
        if e.removed || actor.is_none() {
            self.engage = None;
            let near = hero.pos.distance(e.last_pos) < 80.0;
            let last_effect = e.last_hit.max(e.last_progress);
            let effective = e.hits > 0 || e.last_progress > 0;
            if effective && now.saturating_sub(last_effect) < KILL_WINDOW_MS && near {
                match e.kind {
                    EntityKind::Monster => self.counters.monster_kills += 1,
                    _ => self.counters.npc_kills += 1,
                }
                self.emit(Level::Success, EventCode::TargetKilled, &[e.name]);
            }
            return;
        }
        let stocked = match e.kind {
            EntityKind::Monster => world.stock(StockKind::Harpoon, e.weapon),
            _ if e.torpedo && e.weapon == 0 => None,
            _ if e.torpedo => world.stock(StockKind::Torpedo, e.weapon),
            _ => world.stock(StockKind::Ammo, e.weapon),
        };
        if stocked.is_some_and(|n| n <= 0) {
            self.engage = None;
            out.push(Action::AbortAttack);
            self.warn_once(
                now,
                EventCode::NoAmmo,
                i64::from(e.weapon),
                &[e.weapon.to_string()],
            );
        }
    }

    fn settle_collecting(&mut self, now: u64, world: &World, hero: &HeroState) {
        let Some(c) = self.collect.clone() else { return };
        if c.removed {
            self.collect = None;
            if hero.pos.distance(c.pos) <= COLLECT_CONFIRM_RADIUS {
                self.counters.boxes += 1;
                self.emit(Level::Success, EventCode::BoxCollected, &[c.name]);
            }
            return;
        }
        if world.actor(c.id).is_none() {
            self.collect = None;
        } else if c.detour_until.is_some_and(|t| now > t) {
            self.collect = None;
            self.skips.insert(c.id, now + SKIP_BOX_MS);
        }
    }

    fn combat_choice(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        caps: &Capabilities,
    ) -> Option<Choice> {
        let sight = Self::sight(hero);
        let current = self.engage.as_ref().map(|e| (e.id, e.last_fire));
        let current = current.and_then(|(id, fired)| {
            let actor = world
                .actor(id)
                .filter(|a| hero.pos.distance(a.pos) <= sight * KEEP_SIGHT)?;
            self.evaluate(now, world, hero, caps, actor).map(|c| (c, fired))
        });
        if current.is_none() {
            self.engage = None;
        }
        let mut candidates = Vec::new();
        let pool: Vec<&Actor> = world
            .actors
            .iter()
            .filter(|a| a.visible && !self.skips.contains_key(&a.id))
            .filter(|a| hero.pos.distance(a.pos) <= sight)
            .collect();
        for actor in pool {
            candidates.extend(self.evaluate(now, world, hero, caps, actor));
        }
        let best = targets::rank(candidates.into_iter());
        if self.eldorado_holds_targets() || self.bonus.phase == kraken_proto::BonusPhase::Finishing {
            return current.map(|(c, _)| c);
        }
        match (current, best) {
            // Only an untouched target gives way, and only to a higher priority.
            (Some((current, 0)), Some(best)) if best.priority < current.priority => Some(best),
            (Some((current, _)), _) => Some(current),
            (None, best) => best,
        }
    }

    fn sight(hero: &HeroState) -> f32 {
        (hero.cannon_range.max(hero.harpoon_range) * SIGHT_FACTOR).max(SIGHT_MIN)
    }

    fn evaluate(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        caps: &Capabilities,
        actor: &Actor,
    ) -> Option<Choice> {
        let distance = hero.pos.distance(actor.pos);
        let result = match actor.kind {
            EntityKind::Npc if caps.cannon => {
                targets::npc(&self.cfg.npc, world, actor)?.map(|(priority, weapon, style)| Choice {
                    id: actor.id,
                    kind: actor.kind,
                    priority,
                    distance,
                    weapon,
                    style,
                })
            }
            EntityKind::Monster if caps.harpoon => {
                let wanted = targets::monster(&self.cfg.monster, world, actor)?;
                if hero.harpoon_range <= 0.0 {
                    self.warn_once(now, EventCode::NoHarpoonRange, 0, &[]);
                    return None;
                }
                wanted.map(|(priority, weapon)| Choice {
                    id: actor.id,
                    kind: actor.kind,
                    priority,
                    distance,
                    weapon,
                    style: RangeStyle::Approach,
                })
            }
            _ => return None,
        };
        match result {
            Ok(choice) => Some(choice),
            Err(Rejection::NoStock(_, weapon)) => {
                self.warn_once(now, EventCode::NoAmmo, i64::from(weapon), &[weapon.to_string()]);
                None
            }
            Err(Rejection::NoWeapon) => None,
        }
    }

    fn box_choice(&self, world: &World, hero: &HeroState) -> Option<Choice> {
        if let Some(c) = self.collect.as_ref().filter(|c| c.detour_until.is_none())
            && world.actor(c.id).is_some()
        {
            return Some(Choice {
                id: c.id,
                kind: EntityKind::Box,
                priority: 0,
                distance: hero.pos.distance(c.pos),
                weapon: 0,
                style: RangeStyle::Default,
            });
        }
        let sight = Self::sight(hero);
        let candidates = world
            .actors
            .iter()
            .filter(|a| a.visible && !self.skips.contains_key(&a.id))
            .filter(|a| hero.pos.distance(a.pos) <= sight)
            .filter_map(|a| {
                targets::boxed(&self.cfg.collector, a).map(|priority| Choice {
                    id: a.id,
                    kind: EntityKind::Box,
                    priority,
                    distance: hero.pos.distance(a.pos),
                    weapon: 0,
                    style: RangeStyle::Default,
                })
            });
        targets::rank(candidates)
    }

    #[allow(clippy::too_many_arguments)]
    fn fight(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        choice: Choice,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        let Some(target) = world.actor(choice.id).cloned() else {
            return;
        };
        let switching = self.engage.as_ref().is_none_or(|e| !e.id.matches(choice.id));
        if switching {
            if self.engage.is_some() && self.attack_active {
                self.attack_active = false;
            }
            let ready_at = now.saturating_add(self.reaction_delay());
            self.engage = Some(Engagement {
                id: target.id,
                kind: target.kind,
                type_id: target.type_id,
                name: target.name.clone(),
                weapon: choice.weapon,
                torpedo: target.kind == EntityKind::Npc && world.map.as_ref().is_some_and(|m| m.atlantis),
                last_fire: 0,
                last_hit: 0,
                hits: 0,
                last_hp: target.hp_percent,
                last_progress: 0,
                in_range_since: None,
                removed: false,
                last_pos: target.pos,
                ready_at,
                refire_gap: 0,
            });
            if self.engage.as_ref().is_some_and(|e| e.torpedo) {
                tracing::info!(
                    target = %target.name,
                    torpedo = choice.weapon,
                    range = hero.cannon_range,
                    launchers = ?world.torpedo,
                    "torpedo engagement"
                );
            }
            self.emit(
                Level::Info,
                EventCode::TargetEngaged,
                std::slice::from_ref(&target.name),
            );
        }
        let monster = target.kind == EntityKind::Monster;
        self.activity = if monster { Activity::Monster } else { Activity::Npc };
        let range = if monster {
            hero.harpoon_range
        } else {
            hero.cannon_range
        };
        let reach = if range > 0.0 { range } else { 8.0 };
        let distance = hero.pos.distance(target.pos);

        let style = match choice.style {
            RangeStyle::Default if self.cfg.npc.keep_range && !monster => RangeStyle::Keep,
            RangeStyle::Default => RangeStyle::Approach,
            other => other,
        };
        let desired = match style {
            RangeStyle::Keep => reach * f32::from(self.cfg.npc.range_percent.clamp(60, 95)) / 100.0,
            _ => reach * if monster { 0.7 } else { 0.6 },
        };

        // A detour box must stay inside the target's firing range; the
        // target may have moved since the box was picked.
        if let Some(c) = self.collect.as_ref().filter(|c| c.detour_until.is_some())
            && (distance > reach * DETOUR_ABANDON || c.pos.distance(target.pos) > reach * DETOUR_ABANDON)
        {
            self.skips.insert(c.id, now + SKIP_BOX_MS);
            self.collect = None;
            self.nav.clear();
        }
        let detour = self.detour_box(now, world, hero, &target, reach);
        let owner = if monster {
            Owner::Monster(target.id)
        } else {
            Owner::Npc(target.id)
        };
        if let Some((box_id, box_pos)) = detour {
            out.extend(
                self.nav
                    .drive(now, hero, box_pos, Owner::Box(box_id), geometry, terrain)
                    .action,
            );
        } else if distance > reach * 0.95 || (style == RangeStyle::Keep && distance < desired - 4.0) {
            let goal = ray(target.pos, hero.pos, desired);
            let goal = geometry.map_or(goal, |g| g.clamp(goal, 0.01));
            let step = self.nav.drive(now, hero, goal, owner, geometry, terrain);
            out.extend(step.action);
            if step.stuck && self.nav.stuck_count() >= 3 {
                self.skip(target.id, now, SKIP_TARGET_MS, out, &target.name);
                return;
            }
        }

        // Drawn before borrowing the engagement (jitter needs `self`); only
        // committed onto the engagement when a shot actually goes out.
        let next_gap = self.refire_jitter();
        let Some(e) = self.engage.as_mut() else { return };
        e.last_pos = target.pos;
        if let Some(hp) = target.hp_percent {
            if e.last_hp.is_some_and(|before| hp < before - 0.01) {
                e.last_progress = now;
            }
            e.last_hp = Some(hp);
        }
        if distance <= reach {
            let since = *e.in_range_since.get_or_insert(now);
            let silent = now.saturating_sub(since.max(e.last_hit).max(e.last_progress));
            if silent > NO_HIT_SKIP_MS {
                let name = e.name.clone();
                self.skip(target.id, now, SKIP_TARGET_MS, out, &name);
                return;
            }
            if e.torpedo {
                let (weapon, ready_at) = (e.weapon, e.ready_at);
                if now >= ready_at {
                    self.torpedo_fire(now, world, target.id, weapon, out);
                }
                return;
            }
            let gap = e.refire_gap;
            let since_fire = now.saturating_sub(e.last_fire);
            let confirmed = e.last_hit >= e.last_fire && e.last_fire > 0;
            let due = if confirmed && self.attack_active {
                now.saturating_sub(e.last_hit) > REFIRE_SILENT_MS + gap && since_fire > REFIRE_SILENT_MS + gap
            } else {
                e.last_fire == 0 || since_fire >= REFIRE_UNCONFIRMED_MS + gap
            };
            // A human does not fire the instant a target appears.
            let ready = now >= e.ready_at;
            let paced = self
                .last_attack_cmd
                .is_none_or(|t| now.saturating_sub(t) >= ATTACK_INTERVAL_MS);
            if due && ready && paced {
                e.last_fire = now;
                e.refire_gap = next_gap;
                self.last_attack_cmd = Some(now);
                out.push(if monster {
                    Action::Harpoon {
                        target: target.id,
                        harpoon: e.weapon,
                    }
                } else {
                    Action::Cannon {
                        target: target.id,
                        ammo: e.weapon,
                    }
                });
            }
        } else {
            e.in_range_since = None;
        }
    }

    /// Atlantis: load the wanted torpedo into every launcher that holds
    /// something else, and fire once a launcher holds it and is ready.
    fn torpedo_fire(&mut self, now: u64, world: &World, target: Eid, want: i16, out: &mut Vec<Action>) {
        let state = &world.torpedo;
        if want > 0
            && let Some(preset) = state.preset
            && self
                .torpedo_load_at
                .is_none_or(|t| now.saturating_sub(t) >= TORPEDO_LOAD_RETRY_MS)
        {
            let wrong: Vec<i16> = state
                .slots
                .iter()
                .filter(|(_, t)| *t != want)
                .map(|(s, _)| *s)
                .collect();
            if !wrong.is_empty() {
                tracing::info!(preset, ?wrong, torpedo = want, "loading torpedoes");
                self.torpedo_load_at = Some(now);
                out.extend(wrong.into_iter().map(|slot| Action::LoadTorpedo {
                    preset,
                    slot,
                    torpedo: want,
                }));
            }
        }
        let loaded = if want > 0 {
            state.slots.iter().any(|(_, t)| *t == want)
        } else {
            state.slots.is_empty() || state.slots.iter().any(|(_, t)| *t > 0)
        };
        let paced = self
            .last_attack_cmd
            .is_none_or(|t| now.saturating_sub(t) >= TORPEDO_GAP_MS);
        if loaded && !state.cooling && paced {
            self.last_attack_cmd = Some(now);
            if let Some(e) = self.engage.as_mut() {
                e.last_fire = now;
            }
            out.push(Action::Torpedo { target });
        }
    }

    fn detour_box(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        target: &Actor,
        reach: f32,
    ) -> Option<(Eid, Point)> {
        let collector = &self.cfg.collector;
        if !collector.enabled || !collector.during_combat || !self.cfg.mode.collect() {
            return None;
        }
        if let Some(c) = self.collect.as_ref().filter(|c| c.detour_until.is_some()) {
            return Some((c.id, c.pos));
        }
        // Only from inside firing range: the straight path from here to a box
        // that is itself within range of the target never leaves that range.
        if hero.pos.distance(target.pos) > reach {
            return None;
        }
        // By default every box inside the hero's own range circle qualifies.
        let radius = if collector.detour_radius > 0 {
            f32::from(collector.detour_radius)
        } else {
            reach
        };
        let pick = world
            .actors
            .iter()
            .filter(|a| a.visible && !self.skips.contains_key(&a.id))
            .filter(|a| targets::boxed(collector, a).is_some())
            .filter(|a| a.pos.distance(hero.pos) <= radius)
            .filter(|a| a.pos.distance(target.pos) <= reach * DETOUR_REACH)
            .min_by(|a, b| a.pos.distance(hero.pos).total_cmp(&b.pos.distance(hero.pos)))?;
        let until = now + u64::from(collector.detour_secs.max(1)) * 1_000;
        self.collect = Some(Collecting {
            id: pick.id,
            name: pick.name.clone(),
            type_id: pick.type_id,
            pos: pick.pos,
            since: now,
            arrived_at: None,
            removed: false,
            detour_until: Some(until),
        });
        Some((pick.id, pick.pos))
    }

    fn skip(&mut self, id: Eid, now: u64, ms: u64, out: &mut Vec<Action>, name: &str) {
        self.skips.insert(id, now + ms);
        if self.engage.as_ref().is_some_and(|e| e.id.matches(id)) {
            self.engage = None;
            out.push(Action::AbortAttack);
            self.emit(Level::Warning, EventCode::TargetSkipped, &[name.to_owned()]);
        } else if self.collect.as_ref().is_some_and(|c| c.id.matches(id)) {
            self.collect = None;
            self.emit(Level::Warning, EventCode::BoxSkipped, &[name.to_owned()]);
        }
        self.nav.clear();
    }

    #[allow(clippy::too_many_arguments)]
    fn gather(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        choice: Choice,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        let Some(item) = world.actor(choice.id).cloned() else {
            return;
        };
        if self.collect.as_ref().is_none_or(|c| !c.id.matches(item.id)) {
            self.collect = Some(Collecting {
                id: item.id,
                name: item.name.clone(),
                type_id: item.type_id,
                pos: item.pos,
                since: now,
                arrived_at: None,
                removed: false,
                detour_until: None,
            });
        }
        self.activity = Activity::Collecting;
        let distance = hero.pos.distance(item.pos);
        let Some(c) = self.collect.as_mut() else { return };
        c.pos = item.pos;
        if distance <= COLLECT_RADIUS {
            let arrived = *c.arrived_at.get_or_insert(now);
            if now.saturating_sub(arrived) > BOX_SETTLE_MS {
                self.skip(item.id, now, SKIP_BOX_MS, out, &item.name);
            }
            return;
        }
        c.arrived_at = None;
        if now.saturating_sub(c.since) > BOX_TIMEOUT_MS {
            self.skip(item.id, now, SKIP_BOX_MS, out, &item.name);
            return;
        }
        let step = self
            .nav
            .drive(now, hero, item.pos, Owner::Box(item.id), geometry, terrain);
        out.extend(step.action);
        if (step.stuck && self.nav.stuck_count() >= 3) || step.blocked {
            self.skip(item.id, now, SKIP_BOX_MS, out, &item.name);
        }
    }

    fn wander(
        &mut self,
        now: u64,
        hero: &HeroState,
        geometry: Option<Geometry>,
        caps: &Capabilities,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        let roam = self.cfg.navigation.roam;
        if roam == RoamMode::Idle || !caps.movement {
            self.activity = Activity::Idle;
            out.extend(self.nav.stop(now, hero));
            return;
        }
        self.activity = Activity::Roaming;
        if self.roam.map != self.map_id {
            self.roam = Roam {
                map: self.map_id,
                ..Roam::default()
            };
        }
        let exhausted = self.nav.stuck_count() >= 3;
        if self.roam.goal.is_none() || exhausted {
            self.roam.goal = self.next_roam_goal(now, hero, geometry);
            self.roam.seq = self.roam.seq.wrapping_add(1);
            if self.roam.goal.is_none() {
                self.activity = Activity::Idle;
                return;
            }
        }
        let goal = self.roam.goal.expect("roam goal");
        let step = self
            .nav
            .drive(now, hero, goal, Owner::Roam(self.roam.seq), geometry, terrain);
        out.extend(step.action);
        if step.stuck {
            self.emit(
                Level::Warning,
                EventCode::Stuck,
                &[format!("{:.0},{:.0}", hero.pos.x, hero.pos.y)],
            );
        }
        if step.arrived || step.blocked {
            self.roam.goal = None;
        }
    }

    fn next_roam_goal(&mut self, now: u64, hero: &HeroState, geometry: Option<Geometry>) -> Option<Point> {
        let roam = self.cfg.navigation.roam;
        if roam == RoamMode::Route {
            let map = self.map_id?;
            let points = self.cfg.navigation.route_for(map)?.points.clone();
            for _ in 0..points.len() {
                let point = points[self.roam.cursor % points.len()];
                self.roam.cursor = (self.roam.cursor + 1) % points.len();
                if geometry.is_none_or(|g| g.contains(point, 0.0)) {
                    return Some(point);
                }
                self.warn_once(
                    now,
                    EventCode::RouteInvalid,
                    i64::from(map),
                    &[format!("{:.0},{:.0}", point.x, point.y)],
                );
            }
            return None;
        }
        let geometry = geometry?;
        match roam {
            RoamMode::Random => {
                let mut best = geometry.random(&mut self.rng, ROAM_MARGIN);
                for _ in 0..4 {
                    if best.distance(hero.pos) >= 60.0 {
                        break;
                    }
                    best = geometry.random(&mut self.rng, ROAM_MARGIN);
                }
                Some(best)
            }
            RoamMode::Corners => {
                let corners = geometry.corners(ROAM_MARGIN);
                if self.roam.seq == 0 {
                    self.roam.cursor = (0..4)
                        .min_by(|a, b| {
                            corners[*a]
                                .distance(hero.pos)
                                .total_cmp(&corners[*b].distance(hero.pos))
                        })
                        .unwrap_or(0);
                }
                let point = corners[self.roam.cursor % 4];
                self.roam.cursor = (self.roam.cursor + 1) % 4;
                Some(point)
            }
            RoamMode::Lanes => {
                let lanes = geometry.lanes(ROAM_MARGIN, 7);
                if self.roam.seq == 0 {
                    self.roam.cursor = (0..lanes.len())
                        .min_by(|a, b| {
                            lanes[*a]
                                .distance(hero.pos)
                                .total_cmp(&lanes[*b].distance(hero.pos))
                        })
                        .unwrap_or(0);
                }
                let point = lanes[self.roam.cursor % lanes.len()];
                self.roam.cursor = (self.roam.cursor + 1) % lanes.len();
                Some(point)
            }
            RoamMode::Route | RoamMode::Idle => None,
        }
    }

    pub fn activity(&self) -> Activity {
        self.activity
    }

    pub fn waypoint(&self) -> Option<Point> {
        self.nav.waypoint()
    }

    pub fn counters(&self, now: u64) -> Counters {
        let live = self.run_since.map_or(0, |t| now.saturating_sub(t));
        Counters {
            run_secs: (self.run_ms + live) / 1_000,
            ..self.counters
        }
    }

    pub fn target(&self, world: &World) -> Option<TargetInfo> {
        let hero = world.hero.as_ref()?;
        if let Some(id) = self.war_target() {
            let actor = world.actor(id)?;
            return Some(TargetInfo {
                kind: actor.kind,
                type_id: actor.type_id,
                name: actor.name.clone(),
                distance: actor.pos.distance(hero.pos),
                hp_percent: actor.hp_percent,
            });
        }
        let (id, kind) = match (&self.collect, &self.engage) {
            _ if let Some(id) = self.eldorado_chest() => (id, EntityKind::Box),
            (Some(c), _) if self.activity == Activity::Collecting || c.detour_until.is_some() => {
                (c.id, EntityKind::Box)
            }
            (_, Some(e)) => (e.id, e.kind),
            (Some(c), None) => (c.id, EntityKind::Box),
            (None, None) => return None,
        };
        let actor = world.actor(id)?;
        Some(TargetInfo {
            kind,
            type_id: actor.type_id,
            name: actor.name.clone(),
            distance: actor.pos.distance(hero.pos),
            hp_percent: actor.hp_percent,
        })
    }

    pub fn target_id(&self) -> Option<Eid> {
        if self.cfg.mode.war() {
            return self.war_target();
        }
        self.engage
            .as_ref()
            .map(|e| e.id)
            .or_else(|| self.collect.as_ref().map(|c| c.id))
            .or_else(|| self.eldorado_chest())
    }

    pub fn engaged_type(&self) -> Option<i32> {
        self.engage
            .as_ref()
            .map(|e| e.type_id)
            .or_else(|| self.collect.as_ref().map(|c| c.type_id))
    }
}

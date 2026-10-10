//! Bonus maps: play the queued maps one after the other. Entering works like
//! El Dorado (finish the fight, corner, request, hold still for the delay);
//! inside, the waves spawn around the ship, so it stays put and spreads its
//! fire, switching to the next NPC after every hit.

use std::collections::{BTreeMap, HashMap};

use kraken_proto::{Activity, BonusInfo, BonusPhase, EntityKind, EventCode, Level, Point, StockKind};

use super::Engine;
use super::eldorado::{FINISH_MS, RETREAT_MS};
use super::geometry::Geometry;
use super::nav::Owner;
use super::retreat::{CornerRetreat, Sail};
use super::world::{Action, Eid, HeroState, MapKind, MapState, Terrain, World};

/// No answer (delay or map change) to an entry request by then: try again.
const CONFIRM_MS: u64 = 3_000;
const ENTER_TRIES: u32 = 2;
/// After the announced delay, the jump may take this much longer.
const JUMP_GRACE_MS: u64 = 10_000;
/// A map that could not be entered is left alone this long.
const FAILED_SKIP_MS: u64 = 300_000;
/// Next target after this long without a hit on the current one.
const ROTATE_MS: u64 = 1_200;
/// Never faster than this between two cannon orders.
const SHOT_GAP_MS: u64 = 250;
/// Out-of-range NPCs are approached to this share of the cannon range.
const APPROACH: f32 = 0.85;
/// With no NPC around, the next wave gets this long to show up.
const WAVE_WAIT_MS: u64 = 4_000;

#[derive(Debug, Default)]
pub(super) struct Bonus {
    pub(super) phase: BonusPhase,
    since: u64,
    corner: CornerRetreat,
    /// The map being entered or played.
    map: Option<i16>,
    tries: u32,
    last_request: Option<u64>,
    /// Set by the delay message: the jump is expected before this.
    jump_by: Option<u64>,
    jump_at: Option<u64>,
    /// On the bonus map (its map id).
    pub(super) inside: Option<i32>,
    /// Maps finished since the bot started.
    done: BTreeMap<i16, u32>,
    skip_until: HashMap<i16, u64>,
    /// Something was queued; announces the end of the queue once.
    had_entry: bool,
    // Fighting.
    target: Option<Eid>,
    last_shot: Option<u64>,
    hit: bool,
    empty_since: Option<u64>,
    wave: u8,
}

impl Bonus {
    fn enter(&mut self, phase: BonusPhase, now: u64) {
        self.phase = phase;
        self.since = now;
    }

    /// Back to waiting; keeps the counts and skips.
    pub(super) fn reset(&mut self) {
        *self = Self {
            done: std::mem::take(&mut self.done),
            skip_until: std::mem::take(&mut self.skip_until),
            had_entry: self.had_entry,
            inside: self.inside,
            ..Self::default()
        };
    }

    /// A fresh start of the bot.
    pub(super) fn restart(&mut self) {
        *self = Self::default();
    }
}

impl Engine {
    /// The bonus map the ship is on, if any.
    pub(super) fn on_bonus(&self, world: &World, map: &MapState) -> bool {
        if Self::on_eldorado(map) || map.kind == MapKind::Death {
            return false;
        }
        map.kind_name == "WAVE"
            || self.bonus.inside == Some(map.id)
            || world.bonus.iter().any(|b| i32::from(b.id) == map.id)
    }

    /// The next queued map that can be played.
    fn bonus_next(&self, now: u64, world: &World) -> Option<(i16, String)> {
        if !self.cfg.bonus.enabled {
            return None;
        }
        self.cfg.bonus.queue.iter().find_map(|entry| {
            let owned = world
                .bonus
                .iter()
                .find(|b| b.id == entry.map)
                .map(|b| b.amount)
                .or_else(|| world.stock(StockKind::BonusMap, entry.map))
                .unwrap_or(0);
            let done = self.bonus.done.get(&entry.map).copied().unwrap_or(0);
            let wanted = entry.count.is_none_or(|n| done < u32::from(n));
            let skipped = self.bonus.skip_until.get(&entry.map).is_some_and(|t| now < *t);
            (owned > 0 && wanted && !skipped).then(|| (entry.map, entry.name.clone()))
        })
    }

    /// The queue still has work: El Dorado waits for it.
    pub(super) fn bonus_pending(&self, now: u64, world: &World) -> bool {
        self.bonus.phase != BonusPhase::Waiting || self.bonus_next(now, world).is_some()
    }

    /// Map changes: the jump in, and the game sending the ship back out.
    pub(super) fn bonus_map(&mut self, world: &World, map: &MapState, moved: bool) {
        // Whatever map the requested jump leads to is the bonus map.
        if moved
            && self.bonus.phase == BonusPhase::Countdown
            && map.kind != MapKind::Death
            && self.bonus.inside.is_none()
        {
            self.bonus.inside = Some(map.id);
            let map_id = self.bonus.map;
            self.bonus.reset();
            self.bonus.map = map_id;
            self.bonus.enter(BonusPhase::Fighting, 0);
            self.emit(Level::Success, EventCode::BonusEntered, &[map.id.to_string()]);
            return;
        }
        if let Some(inside) = self.bonus.inside
            && inside != map.id
        {
            let played = self.bonus.map.unwrap_or(inside as i16);
            self.bonus.inside = None;
            self.bonus.reset();
            if map.kind != MapKind::Death {
                *self.bonus.done.entry(played).or_default() += 1;
                self.emit(Level::Success, EventCode::BonusFinished, &[played.to_string()]);
            }
        }
        if self.on_bonus(world, map) && self.bonus.inside.is_none() {
            let map_id = self.bonus.map;
            self.bonus.reset();
            self.bonus.map = map_id.or(Some(map.id as i16));
            self.bonus.inside = Some(map.id);
            self.bonus.enter(BonusPhase::Fighting, 0);
            self.emit(Level::Success, EventCode::BonusEntered, &[map.id.to_string()]);
        }
    }

    pub(super) fn bonus_delay(&mut self, now: u64, map: i16, delay_ms: u64) {
        if self.bonus.phase == BonusPhase::Countdown {
            // Seconds or milliseconds: a delay under 1000 is in seconds.
            let delay = if delay_ms < 1_000 {
                delay_ms * 1_000
            } else {
                delay_ms
            };
            tracing::info!(map, delay, "bonus map delay");
            self.bonus.jump_at = Some(now + delay);
            self.bonus.jump_by = Some(now + delay + JUMP_GRACE_MS);
        }
    }

    pub(super) fn bonus_hit(&mut self, target: Eid) {
        if self.bonus.target.is_some_and(|t| t.matches(target)) {
            self.bonus.hit = true;
        }
    }

    pub(super) fn bonus_removed(&mut self, id: Eid) {
        if self.bonus.target.is_some_and(|t| t.matches(id)) {
            self.bonus.target = None;
            self.bonus.hit = true;
        }
    }

    fn bonus_failed(&mut self, now: u64) {
        if let Some(map) = self.bonus.map {
            self.bonus.skip_until.insert(map, now + FAILED_SKIP_MS);
            self.emit(Level::Warning, EventCode::BonusFailed, &[map.to_string()]);
        }
        self.bonus.reset();
    }

    /// Finishing, corner and entry request. Returns true when it took the step.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn bonus_prepare(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        map: &MapState,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) -> bool {
        if !self.cfg.bonus.enabled || map.kind == MapKind::Death || self.on_bonus(world, map) {
            if !matches!(self.bonus.phase, BonusPhase::Waiting | BonusPhase::Fighting) {
                self.bonus.reset();
            }
            return false;
        }
        loop {
            match self.bonus.phase {
                BonusPhase::Waiting | BonusPhase::Fighting => {
                    let Some((id, name)) = self.bonus_next(now, world) else {
                        if self.bonus.had_entry {
                            self.bonus.had_entry = false;
                            self.emit(Level::Success, EventCode::BonusQueueDone, &[]);
                        }
                        return false;
                    };
                    self.bonus.reset();
                    self.bonus.had_entry = true;
                    self.bonus.map = Some(id);
                    self.emit(Level::Info, EventCode::BonusEntering, &[name]);
                    let next = if self.engage.is_some() {
                        BonusPhase::Finishing
                    } else {
                        BonusPhase::Retreat
                    };
                    self.bonus.enter(next, now);
                }
                BonusPhase::Finishing => {
                    let fighting = self
                        .engage
                        .as_ref()
                        .is_some_and(|e| !e.removed && world.actor(e.id).is_some());
                    if fighting && now.saturating_sub(self.bonus.since) < FINISH_MS {
                        return false;
                    }
                    self.bonus.enter(BonusPhase::Retreat, now);
                }
                BonusPhase::Retreat => {
                    self.activity = Activity::Bonus;
                    if self.engage.take().is_some() || self.attack_active {
                        out.push(Action::AbortAttack);
                        self.attack_active = false;
                    }
                    self.collect = None;
                    let late = now.saturating_sub(self.bonus.since) > RETREAT_MS;
                    let sail = match (self.cfg.bonus.retreat, geometry) {
                        (true, Some(g)) if !late => {
                            self.bonus.corner.sail(&mut self.nav, now, hero, g, terrain)
                        }
                        _ => Sail::Done,
                    };
                    match sail {
                        Sail::Sailing(action) => {
                            out.extend(action);
                            return true;
                        }
                        Sail::Done => {
                            self.bonus.enter(BonusPhase::Countdown, now);
                            continue;
                        }
                    }
                }
                BonusPhase::Countdown => {
                    self.activity = Activity::Bonus;
                    let Some(id) = self.bonus.map else {
                        self.bonus.reset();
                        return false;
                    };
                    if let Some(by) = self.bonus.jump_by {
                        if now >= by {
                            self.bonus_failed(now);
                            return false;
                        }
                        return true;
                    }
                    let answered_late = self
                        .bonus
                        .last_request
                        .is_some_and(|t| now.saturating_sub(t) >= CONFIRM_MS);
                    if self.bonus.last_request.is_none() || (answered_late && self.bonus.tries < ENTER_TRIES)
                    {
                        out.extend(self.nav.stop(now, hero));
                        out.push(Action::EnterMap(id));
                        self.bonus.tries += 1;
                        self.bonus.last_request = Some(now);
                    } else if answered_late {
                        self.bonus_failed(now);
                        return false;
                    }
                    return true;
                }
            }
        }
    }

    /// On the bonus map: stay put and spread the fire over every NPC in range.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn bonus_fight(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        map: &MapState,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        self.activity = Activity::Npc;
        self.engage = None;
        self.collect = None;
        if let Some(status) = self
            .bonus
            .map
            .and_then(|id| world.bonus.iter().find(|b| b.id == id))
            && status.wave != self.bonus.wave
        {
            self.bonus.wave = status.wave;
            let waves = status.waves.to_string();
            self.emit(
                Level::Info,
                EventCode::BonusWave,
                &[status.wave.to_string(), waves],
            );
        }
        let range = hero.cannon_range.max(1.0);
        let mut npcs: Vec<(Eid, Point, f32)> = world
            .actors
            .iter()
            .filter(|a| a.kind == EntityKind::Npc && a.visible)
            .map(|a| (a.id, a.pos, hero.pos.distance(a.pos)))
            .collect();
        npcs.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.id.cmp(&b.0.id)));
        let in_range: Vec<Eid> = npcs.iter().filter(|n| n.2 <= range).map(|n| n.0).collect();

        if !in_range.is_empty() {
            self.bonus.empty_since = None;
            out.extend(self.nav.stop(now, hero));
            let current = self
                .bonus
                .target
                .filter(|t| in_range.iter().any(|i| i.matches(*t)));
            let last_shot = self.bonus.last_shot;
            let since = |gap: u64| last_shot.is_none_or(|t| now.saturating_sub(t) >= gap);
            let due = current.is_none() || self.bonus.hit || since(ROTATE_MS);
            if !due || !since(SHOT_GAP_MS) {
                return;
            }
            let next = match current.and_then(|c| in_range.iter().position(|i| i.matches(c))) {
                Some(i) => in_range[(i + 1) % in_range.len()],
                None => in_range[0],
            };
            let Some(ammo) = self.bonus_ammo(world) else {
                self.warn_once(now, EventCode::NoAmmo, 0, &[]);
                return;
            };
            out.push(Action::Cannon { target: next, ammo });
            self.bonus.target = Some(next);
            self.bonus.last_shot = Some(now);
            self.bonus.hit = false;
            return;
        }
        self.bonus.target = None;
        if let Some((id, pos, _)) = npcs.first().copied() {
            // A wave out of reach: close in until it is in range.
            self.bonus.empty_since = None;
            let goal = pos.toward(hero.pos, range * APPROACH).rounded();
            let step = self.nav.drive(now, hero, goal, Owner::Npc(id), geometry, terrain);
            out.extend(step.action);
            return;
        }
        let since = *self.bonus.empty_since.get_or_insert(now);
        if now.saturating_sub(since) < WAVE_WAIT_MS {
            out.extend(self.nav.stop(now, hero));
            return;
        }
        // Nothing for a while: look around the middle of the map.
        let centre = Point::new(map.width / 2.0, 0.0);
        if hero.pos.distance(centre) > 10.0 {
            let step = self
                .nav
                .drive(now, hero, centre, Owner::Corner, geometry, terrain);
            out.extend(step.action);
        }
    }

    fn bonus_ammo(&self, world: &World) -> Option<i16> {
        let stocked = |id: i16| world.stock(StockKind::Ammo, id).is_none_or(|n| n > 0);
        self.cfg
            .bonus
            .ammo
            .filter(|id| stocked(*id))
            .or(self.cfg.npc.default_ammo.filter(|id| stocked(*id)))
            .or_else(|| {
                world
                    .stock
                    .iter()
                    .filter(|s| s.kind == StockKind::Ammo && s.amount > 0)
                    .map(|s| s.id)
                    .min()
            })
    }

    /// On a bonus map: the pump scans at fight speed.
    pub fn fast_map(&self) -> bool {
        self.bonus.inside.is_some()
    }

    pub fn bonus_info(&self, now: u64, world: &World) -> Option<BonusInfo> {
        if !self.cfg.bonus.enabled {
            return None;
        }
        let b = &self.bonus;
        let status = b.map.and_then(|id| world.bonus.iter().find(|m| m.id == id));
        Some(BonusInfo {
            phase: b.phase,
            map: b.map,
            wave: status.map_or(0, |s| s.wave),
            waves: status.map_or(0, |s| s.waves),
            done: b.done.iter().map(|(k, v)| (*k, *v)).collect(),
            delay_ms: b.jump_at.map_or(0, |t| t.saturating_sub(now)),
        })
    }
}

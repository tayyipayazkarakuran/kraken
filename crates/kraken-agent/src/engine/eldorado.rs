//! El Dorado: once the master skill is charged, finish the fight, sail to a
//! map corner, trigger the jump and, on the bonus map, collect every chest
//! as fast as possible (nearest first, the next one the moment one is taken).

use kraken_proto::{
    Activity, ELDORADO_MAP, ElDoradoInfo, ElDoradoPhase, EntityKind, EventCode, Level, Point,
};

use super::geometry::Geometry;
use super::nav::Owner;
use super::retreat::{CornerRetreat, Sail};
use super::world::{Action, Eid, HeroState, MapKind, MapState, Terrain, World};
use super::{COLLECT_CONFIRM_RADIUS, COLLECT_RADIUS, Engine};

/// Longest wait for the current fight before retreating anyway.
pub(super) const FINISH_MS: u64 = 30_000;
/// Longest sail to the corner.
pub(super) const RETREAT_MS: u64 = 45_000;
/// The server answers a trigger within a fraction of a second
/// (`MasterSkillTriggerMessage`); no answer by then means it refused.
const CONFIRM_MS: u64 = 3_000;
const TRIGGER_TRIES: u32 = 2;
/// Accepted, but no jump this long after: give up.
const JUMP_MS: u64 = 20_000;
/// The game can keep showing 100% after a run (or a refused trigger) until
/// it resets the charge. Wait for a lower charge, or this long.
const RECHARGE_FALLBACK_MS: u64 = 600_000;
/// Box types taken on the bonus map: treasure chests only, no glitter
/// (`seafight.labels.BoxType`: 5 treasure chest, 97 Ragnarök treasure chest).
const CHEST_TYPES: [i32; 2] = [5, 97];
/// Sitting on a chest this long without it going away: no key, skip it.
const DWELL_MS: u64 = 1_500;
/// Give up on reaching a chest after this long.
const CHEST_TIMEOUT_MS: u64 = 20_000;
const CHEST_SKIP_MS: u64 = 20_000;

#[derive(Clone, Debug)]
struct Chest {
    id: Eid,
    name: String,
    pos: Point,
    since: u64,
    arrived_at: Option<u64>,
}

#[derive(Debug, Default)]
pub(super) struct ElDorado {
    pub(super) phase: ElDoradoPhase,
    since: u64,
    corner: CornerRetreat,
    tries: u32,
    last_trigger: Option<u64>,
    /// The server accepted the trigger.
    confirmed: bool,
    /// Since the last jump; cleared once the charge is seen below 100.
    recharging_since: Option<u64>,
    /// When the bonus map ends, if the game told us.
    ends_at: Option<u64>,
    chests: u32,
    runs: u32,
    /// The El Dorado skill id and its full charge (points), from the last scan.
    skill: Option<i32>,
    max_charge: i64,
    current: Option<Chest>,
}

impl ElDorado {
    fn enter(&mut self, phase: ElDoradoPhase, now: u64) {
        self.phase = phase;
        self.since = now;
    }

    /// Back to charging; keeps the run counter.
    pub(super) fn reset(&mut self) {
        *self = Self {
            runs: self.runs,
            skill: self.skill,
            max_charge: self.max_charge,
            recharging_since: self.recharging_since,
            ..Self::default()
        };
    }
}

impl Engine {
    pub(super) fn on_eldorado(map: &MapState) -> bool {
        map.id == ELDORADO_MAP
    }

    /// Map changes in and out of the bonus map.
    pub(super) fn eldorado_map(&mut self, now: u64, from: Option<i32>, to: i32) {
        if to == ELDORADO_MAP {
            if self.eldorado.phase != ElDoradoPhase::Run {
                let ends_at = self.eldorado.ends_at;
                self.eldorado.reset();
                self.eldorado.ends_at = ends_at;
                self.eldorado.enter(ElDoradoPhase::Run, now);
                self.eldorado.recharging_since = Some(now);
                self.emit(Level::Success, EventCode::ElDoradoEntered, &[]);
            }
        } else if from == Some(ELDORADO_MAP) {
            let chests = self.eldorado.chests;
            self.eldorado.runs += 1;
            self.eldorado.reset();
            self.emit(Level::Success, EventCode::ElDoradoLeft, &[chests.to_string()]);
        } else if self.eldorado.phase != ElDoradoPhase::Waiting {
            // Moved elsewhere (death, portal): start over.
            self.eldorado.reset();
        }
    }

    pub(super) fn eldorado_triggered(&mut self, now: u64, id: i32, remaining_ms: u64) {
        if self.eldorado.skill != Some(id) {
            return;
        }
        self.eldorado.confirmed = true;
        if remaining_ms > 0 {
            self.eldorado.ends_at = Some(now + remaining_ms);
        }
    }

    /// A removed chest of the bonus map counts as collected.
    /// The server's charge (points): below full means the next 100% is a real one.
    pub(super) fn eldorado_charge(&mut self, id: i32, points: i64) {
        let max = self.eldorado.max_charge;
        if self.eldorado.skill == Some(id) && max > 0 && points < max {
            self.eldorado.recharging_since = None;
        }
    }

    /// The chest being sailed to went away: ours if the ship was on it.
    pub(super) fn eldorado_removed(&mut self, id: Eid, hero: Option<Point>) {
        if self.eldorado.phase != ElDoradoPhase::Run
            || !self.eldorado.current.as_ref().is_some_and(|c| c.id.matches(id))
        {
            return;
        }
        let Some(chest) = self.eldorado.current.take() else {
            return;
        };
        self.nav.clear();
        if hero.is_some_and(|h| h.distance(chest.pos) <= COLLECT_CONFIRM_RADIUS) {
            self.eldorado.chests += 1;
            self.counters.boxes += 1;
            self.emit(Level::Success, EventCode::BoxCollected, &[chest.name]);
        }
    }

    /// Only the fight in progress is kept while the skill waits for it.
    pub(super) fn eldorado_holds_targets(&self) -> bool {
        self.eldorado.phase == ElDoradoPhase::Finishing
    }

    /// Charge, retreat and countdown. Returns true when it took the step.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn eldorado_prepare(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        map: &MapState,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) -> bool {
        let Some(skill) = world.eldorado().copied() else {
            return false;
        };
        self.eldorado.skill = Some(skill.id);
        self.eldorado.max_charge = skill.max_charge;
        if !self.cfg.eldorado.enabled || map.kind == MapKind::Death || Self::on_eldorado(map) {
            if !matches!(self.eldorado.phase, ElDoradoPhase::Waiting | ElDoradoPhase::Run) {
                self.eldorado.reset();
            }
            return false;
        }
        loop {
            match self.eldorado.phase {
                ElDoradoPhase::Waiting | ElDoradoPhase::Run => {
                    // Queued bonus maps go first.
                    if self.bonus_pending(now, world) {
                        return false;
                    }
                    if let Some(since) = self.eldorado.recharging_since {
                        if skill.charge < 100 || now.saturating_sub(since) >= RECHARGE_FALLBACK_MS {
                            self.eldorado.recharging_since = None;
                        } else {
                            return false;
                        }
                    }
                    if !skill.active || skill.charge < 100 {
                        return false;
                    }
                    self.eldorado.reset();
                    self.emit(Level::Info, EventCode::ElDoradoReady, &[]);
                    let next = if self.cfg.eldorado.finish_target && self.engage.is_some() {
                        ElDoradoPhase::Finishing
                    } else {
                        ElDoradoPhase::Retreat
                    };
                    self.eldorado.enter(next, now);
                }
                ElDoradoPhase::Finishing => {
                    let fighting = self
                        .engage
                        .as_ref()
                        .is_some_and(|e| !e.removed && world.actor(e.id).is_some());
                    if fighting && now.saturating_sub(self.eldorado.since) < FINISH_MS {
                        // The regular step finishes the fight (no new targets).
                        return false;
                    }
                    self.eldorado.enter(ElDoradoPhase::Retreat, now);
                }
                ElDoradoPhase::Retreat => {
                    self.activity = Activity::ElDorado;
                    if self.engage.take().is_some() || self.attack_active {
                        out.push(Action::AbortAttack);
                        self.attack_active = false;
                    }
                    self.collect = None;
                    let late = now.saturating_sub(self.eldorado.since) > RETREAT_MS;
                    let sail = match (self.cfg.eldorado.retreat, geometry) {
                        (true, Some(g)) if !late => {
                            self.eldorado.corner.sail(&mut self.nav, now, hero, g, terrain)
                        }
                        _ => Sail::Done,
                    };
                    match sail {
                        Sail::Sailing(action) => {
                            out.extend(action);
                            return true;
                        }
                        Sail::Done => {
                            self.eldorado.enter(ElDoradoPhase::Countdown, now);
                            continue;
                        }
                    }
                }
                ElDoradoPhase::Countdown => {
                    self.activity = Activity::ElDorado;
                    let ed = &self.eldorado;
                    let failed = match ed.last_trigger {
                        None => false,
                        Some(t) if ed.confirmed => now.saturating_sub(t) >= JUMP_MS,
                        Some(t) => now.saturating_sub(t) >= CONFIRM_MS && ed.tries >= TRIGGER_TRIES,
                    };
                    if failed {
                        // Refused: the shown charge is stale; wait for a real one.
                        self.eldorado.reset();
                        self.eldorado.recharging_since = Some(now);
                        self.emit(Level::Warning, EventCode::ElDoradoFailed, &[]);
                        return false;
                    }
                    let due = !ed.confirmed
                        && ed
                            .last_trigger
                            .is_none_or(|t| now.saturating_sub(t) >= CONFIRM_MS);
                    if due {
                        out.extend(self.nav.stop(now, hero));
                        out.push(Action::TriggerSkill(skill.id));
                        self.eldorado.tries += 1;
                        self.eldorado.last_trigger = Some(now);
                        self.emit(Level::Info, EventCode::ElDoradoTriggered, &[]);
                    }
                    // Hold still until the game moves the ship.
                    return true;
                }
            }
        }
    }

    /// The bonus map: nearest chest first, next one as soon as it is reached.
    pub(super) fn eldorado_run(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) {
        self.activity = Activity::Collecting;
        if self.engage.take().is_some() || self.attack_active {
            out.push(Action::AbortAttack);
            self.attack_active = false;
        }
        self.collect = None;
        let ed = &mut self.eldorado;
        if ed.current.as_ref().is_some_and(|c| world.actor(c.id).is_none()) {
            ed.current = None;
        }
        // On the chest: keep still until the game takes it (the removal
        // triggers an immediate step to the next one); no key, move on.
        if let Some(c) = ed.current.as_mut() {
            let skip = if hero.pos.distance(c.pos) <= COLLECT_RADIUS {
                let arrived = *c.arrived_at.get_or_insert(now);
                if now.saturating_sub(arrived) < DWELL_MS {
                    return;
                }
                true
            } else {
                c.arrived_at = None;
                now.saturating_sub(c.since) > CHEST_TIMEOUT_MS
            };
            if skip {
                let c = ed.current.take().expect("current chest");
                self.skips.insert(c.id, now + CHEST_SKIP_MS);
                self.emit(Level::Info, EventCode::BoxSkipped, &[c.name]);
            }
        }

        if self.eldorado.current.is_none() {
            let chests: Vec<(Eid, &str, Point)> = world
                .actors
                .iter()
                .filter(|a| a.kind == EntityKind::Box && CHEST_TYPES.contains(&a.type_id))
                .filter(|a| !self.skips.contains_key(&a.id))
                .map(|a| (a.id, a.name.as_str(), a.pos))
                .collect();
            // Nearest, with a look at what lies beyond it.
            let score = |pos: Point| {
                let next = chests
                    .iter()
                    .map(|(_, _, p)| pos.distance(*p))
                    .filter(|d| *d > 0.1)
                    .fold(f32::INFINITY, f32::min);
                hero.pos.distance(pos) + if next.is_finite() { next } else { 0.0 }
            };
            let best = chests.iter().min_by(|a, b| score(a.2).total_cmp(&score(b.2)));
            let Some(&(id, name, pos)) = best else {
                out.extend(self.nav.stop(now, hero));
                return;
            };
            // Kept until taken: switching half-way loses more than it wins.
            self.eldorado.current = Some(Chest {
                id,
                name: name.to_owned(),
                pos,
                since: now,
                arrived_at: None,
            });
        }
        let Some(chest) = self.eldorado.current.as_mut() else {
            return;
        };
        if let Some(actor) = world.actor(chest.id) {
            chest.pos = actor.pos;
        }
        let (id, pos, name) = (chest.id, chest.pos, chest.name.clone());
        let step = self.nav.drive(now, hero, pos, Owner::Box(id), geometry, terrain);
        out.extend(step.action);
        if step.blocked || (step.stuck && self.nav.stuck_count() >= 3) {
            self.eldorado.current = None;
            self.skips.insert(id, now + CHEST_SKIP_MS);
            self.emit(Level::Info, EventCode::BoxSkipped, &[name]);
        }
    }

    pub fn eldorado_info(&self, now: u64, world: &World) -> Option<ElDoradoInfo> {
        let skill = world.eldorado()?;
        let ed = &self.eldorado;
        let remaining_ms = match (ed.phase, ed.ends_at) {
            (ElDoradoPhase::Run, Some(end)) => end.saturating_sub(now),
            _ => skill.remaining_ms,
        };
        Some(ElDoradoInfo {
            available: skill.active,
            charge: skill.charge,
            phase: ed.phase,
            remaining_ms,
            chests: ed.chests,
            runs: ed.runs,
        })
    }

    /// The chest being sailed to, for the HUD and the target marker.
    pub(super) fn eldorado_chest(&self) -> Option<Eid> {
        (self.eldorado.phase == ElDoradoPhase::Run)
            .then(|| self.eldorado.current.as_ref().map(|c| c.id))
            .flatten()
    }
}

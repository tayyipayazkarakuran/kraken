//! Work maps: work on each listed map for the configured time, then travel
//! to the next one. A teleport goes first when one is in stock; otherwise the
//! ship sails from map edge to map edge along the world map grid.
//!
//! At an edge the server offers the neighbouring map (`MapAsk`); the bot
//! takes the offer only when it is the next hop. Which diamond edge leads to
//! which grid neighbour is learnt from those offers (and seeded from earlier
//! sessions), so a wrong edge costs one detour at most.

use std::collections::{HashMap, HashSet, VecDeque};

use kraken_proto::{Activity, EventCode, Level, Point, WorkmapInfo, WorkmapPhase};

use super::Engine;
use super::geometry::Geometry;
use super::nav::Owner;
use super::world::{Action, HeroState, MapCell, MapKind, MapState, Terrain, World};

/// The four sides of the diamond, by the sign of (u, v).
pub const EDGES: usize = 4;
/// Grid steps on the world map: up, down, left, right.
const STEPS: [(i8, i8); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
/// Inset of the edge goal from the map border (share of the half size).
const EDGE_INSET: f32 = 0.004;
/// Close enough to the edge goal.
const EDGE_REACHED: f32 = 4.0;
/// At the edge without an offer this long: back off and come in again.
const ASK_WAIT_MS: u64 = 6_000;
/// Offers older than this are stale.
const ASK_FRESH_MS: u64 = 20_000;
const EDGE_APPROACHES: u8 = 2;
/// The jump after an accepted offer or a teleport must come by then.
const JUMP_MS: u64 = 45_000;
/// No hits for this long before a teleport.
const QUIET_MS: u64 = 4_000;
const FAIL_LIMIT: u32 = 3;
const BLOCK_MS: u64 = 300_000;
const NO_ATLAS_MS: u64 = 60_000;
/// Longest gap between two scans that still counts as work time.
const TICK_CAP_MS: u64 = 2_000;

#[derive(Debug, Default)]
pub(super) struct Workmap {
    phase: WorkmapPhase,
    index: usize,
    worked_ms: u64,
    last_tick: Option<u64>,
    /// The next map on the way and the edge chosen for it.
    hop: Option<i16>,
    edge: Option<usize>,
    approaches: u8,
    at_edge_since: Option<u64>,
    backing_off: Option<Point>,
    tried: Vec<usize>,
    ask: Option<(i16, bool, u64)>,
    jump_since: Option<u64>,
    teleport_failed: HashSet<i16>,
    failures: u32,
    blocked_until: Option<u64>,
    /// (map, edge) -> the map the server offered there.
    learned: HashMap<(i16, usize), i16>,
    /// Grid step -> edge, the same on every map.
    step_edge: [Option<usize>; 4],
    learned_dirty: bool,
}

impl Workmap {
    fn clear_travel(&mut self) {
        self.hop = None;
        self.edge = None;
        self.approaches = 0;
        self.at_edge_since = None;
        self.backing_off = None;
        self.tried.clear();
        self.ask = None;
        self.jump_since = None;
    }

    /// Stop/start: the rotation starts over; learnt edges stay.
    pub(super) fn reset(&mut self) {
        *self = Self {
            learned: std::mem::take(&mut self.learned),
            step_edge: self.step_edge,
            learned_dirty: self.learned_dirty,
            index: self.index,
            ..Self::default()
        };
    }
}

/// The edge goal: the middle of the side, just inside the border.
fn edge_goal(edge: usize, geometry: Geometry) -> Point {
    let k = 0.5 * (1.0 - EDGE_INSET);
    let (u, v) = match edge {
        0 => (k, k),
        1 => (-k, k),
        2 => (-k, -k),
        _ => (k, -k),
    };
    geometry.uv_point(u, v)
}

/// The side of the diamond `p` is nearest to.
fn edge_of(p: Point, geometry: Geometry) -> usize {
    let (u, v) = geometry.to_uv(p);
    match (u >= 0.0, v >= 0.0) {
        (true, true) => 0,
        (false, true) => 1,
        (false, false) => 2,
        (true, false) => 3,
    }
}

fn cell(atlas: &[MapCell], id: i16) -> Option<&MapCell> {
    atlas.iter().find(|c| c.id == id)
}

fn step_between(a: &MapCell, b: &MapCell) -> Option<usize> {
    if a.world != b.world {
        return None;
    }
    let d = (
        i16::from(b.row) - i16::from(a.row),
        i16::from(b.col) - i16::from(a.col),
    );
    STEPS
        .iter()
        .position(|(r, c)| i16::from(*r) == d.0 && i16::from(*c) == d.1)
}

impl Engine {
    pub fn workmap_info(&self, world: &World) -> Option<WorkmapInfo> {
        let cfg = &self.cfg.workmap;
        if !cfg.enabled || cfg.maps.is_empty() {
            return None;
        }
        let w = &self.workmap;
        let total = u64::from(cfg.minutes.max(1)) * 60_000;
        Some(WorkmapInfo {
            phase: w.phase,
            target: cfg.maps.get(w.index % cfg.maps.len()).map(|e| e.map),
            next_hop: w.hop,
            remaining_secs: total.saturating_sub(w.worked_ms) / 1_000,
            atlas: !world.atlas.is_empty(),
        })
    }

    /// Edges learnt this session, to keep for the next one.
    pub fn take_learned_edges(&mut self) -> Option<Vec<(i16, usize, i16)>> {
        if !std::mem::take(&mut self.workmap.learned_dirty) {
            return None;
        }
        Some(
            self.workmap
                .learned
                .iter()
                .map(|((from, edge), to)| (*from, *edge, *to))
                .collect(),
        )
    }

    /// Edges learnt in earlier sessions.
    pub fn seed_edges(&mut self, edges: &[(i16, usize, i16)], atlas: &[MapCell]) {
        for &(from, edge, to) in edges {
            if edge < EDGES {
                self.workmap.learned.insert((from, edge), to);
                self.learn_step(from, edge, to, atlas);
            }
        }
    }

    fn learn_step(&mut self, from: i16, edge: usize, to: i16, atlas: &[MapCell]) {
        if let (Some(a), Some(b)) = (cell(atlas, from), cell(atlas, to))
            && let Some(step) = step_between(a, b)
        {
            self.workmap.step_edge[step] = Some(edge);
        }
    }

    pub(super) fn workmap_ask(&mut self, now: u64, map: i16, confirm: bool) {
        tracing::info!(map, confirm, "map change offered");
        self.workmap.ask = Some((map, confirm, now));
    }

    pub(super) fn workmap_cancel(&mut self) {
        tracing::info!("map change cancelled");
        self.workmap.ask = None;
        if self.workmap.phase == WorkmapPhase::Crossing {
            self.workmap.jump_since = None;
            self.workmap.phase = WorkmapPhase::Sailing;
        }
    }

    /// The ship is on a new map.
    pub(super) fn workmap_map_changed(&mut self, from: Option<i32>, to: i32, world: &World) {
        let w = &mut self.workmap;
        // An offer taken at an edge: remember where that edge leads.
        if let (Some(from), Some(edge), WorkmapPhase::Crossing) = (from, w.edge, w.phase)
            && let (Ok(from), Ok(to)) = (i16::try_from(from), i16::try_from(to))
        {
            w.learned.insert((from, edge), to);
            w.learned_dirty = true;
            self.learn_step(from, edge, to, &world.atlas);
        }
        let w = &mut self.workmap;
        w.clear_travel();
        if matches!(
            w.phase,
            WorkmapPhase::Teleport | WorkmapPhase::Sailing | WorkmapPhase::Crossing
        ) {
            w.phase = WorkmapPhase::Sailing;
            w.failures = 0;
        }
    }

    /// The work map step: returns true while travelling (nothing else runs).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn workmap_step(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        map: &MapState,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
        out: &mut Vec<Action>,
    ) -> bool {
        let cfg = &self.cfg.workmap;
        if !cfg.enabled || cfg.maps.is_empty() {
            self.workmap.phase = WorkmapPhase::Idle;
            return false;
        }
        if map.kind == MapKind::Death || Self::on_eldorado(map) || self.on_bonus(world, map) {
            return false;
        }
        let count = cfg.maps.len();
        let total = u64::from(cfg.minutes.max(1)) * 60_000;
        if self.workmap.phase == WorkmapPhase::Idle {
            // Starting on a listed map: that one is first.
            if let Some(i) = cfg.maps.iter().position(|e| i32::from(e.map) == map.id) {
                self.workmap.index = i;
            }
            self.workmap.phase = WorkmapPhase::Sailing;
        }
        self.workmap.index %= count;
        let target = cfg.maps[self.workmap.index].map;
        let gap = self
            .workmap
            .last_tick
            .map_or(0, |t| now.saturating_sub(t).min(TICK_CAP_MS));
        self.workmap.last_tick = Some(now);

        if i32::from(target) == map.id {
            if self.workmap.phase != WorkmapPhase::Working {
                self.workmap.clear_travel();
                self.workmap.phase = WorkmapPhase::Working;
                self.workmap.worked_ms = 0;
                self.workmap.failures = 0;
                self.emit(Level::Success, EventCode::WorkmapArrived, &[target.to_string()]);
            }
            self.workmap.worked_ms += gap;
            if self.workmap.worked_ms < total || count == 1 {
                if count == 1 {
                    self.workmap.worked_ms = self.workmap.worked_ms.min(total);
                }
                return false;
            }
            self.workmap.index = (self.workmap.index + 1) % count;
            self.workmap.phase = WorkmapPhase::Sailing;
            let next = self.cfg.workmap.maps[self.workmap.index].map;
            self.emit(Level::Info, EventCode::WorkmapTravel, &[next.to_string()]);
            return false;
        }
        if self.workmap.phase == WorkmapPhase::Working {
            self.workmap.phase = WorkmapPhase::Sailing;
        }
        if self.workmap.blocked_until.is_some_and(|t| now < t) {
            self.workmap.phase = WorkmapPhase::Blocked;
            return false;
        }
        if self.workmap.phase == WorkmapPhase::Blocked {
            self.workmap.blocked_until = None;
            self.workmap.failures = 0;
            self.workmap.phase = WorkmapPhase::Sailing;
        }
        self.activity = Activity::Travelling;
        let quiet = self.last_damage.is_none_or(|t| now.saturating_sub(t) >= QUIET_MS);

        // A jump on its way: hold still.
        if let Some(since) = self.workmap.jump_since {
            if now.saturating_sub(since) < JUMP_MS {
                out.extend(self.nav.stop(now, hero));
                return true;
            }
            self.workmap.jump_since = None;
            if self.workmap.phase == WorkmapPhase::Teleport {
                self.workmap.teleport_failed.insert(target);
                self.emit(Level::Warning, EventCode::WorkmapFailed, &["teleport".into()]);
            } else {
                self.workmap_failed(now, "jump");
            }
            self.workmap.phase = WorkmapPhase::Sailing;
            return true;
        }

        if self.engage.take().is_some() || self.attack_active {
            out.push(Action::AbortAttack);
            self.attack_active = false;
        }
        self.collect = None;

        // Teleport first.
        if self.cfg.workmap.teleport
            && world.teleports > 0
            && world.teleport_ready
            && !self.workmap.teleport_failed.contains(&target)
        {
            if !quiet || hero.moving == Some(true) {
                out.extend(self.nav.stop(now, hero));
                return true;
            }
            out.push(Action::Teleport(target));
            self.workmap.phase = WorkmapPhase::Teleport;
            self.workmap.jump_since = Some(now);
            self.emit(Level::Info, EventCode::WorkmapTeleport, &[target.to_string()]);
            return true;
        }

        let Some(geometry) = geometry else { return false };
        if world.atlas.is_empty() {
            self.warn_once(now, EventCode::WorkmapFailed, 0, &["atlas".into()]);
            self.workmap.blocked_until = Some(now + NO_ATLAS_MS);
            return false;
        }
        let Ok(here) = i16::try_from(map.id) else {
            return false;
        };
        let Some(hop) = self.route(here, target, &world.atlas) else {
            self.warn_once(
                now,
                EventCode::WorkmapFailed,
                i64::from(target),
                &["route".into(), target.to_string()],
            );
            self.workmap.index = (self.workmap.index + 1) % count;
            return false;
        };
        if self.workmap.hop != Some(hop) {
            self.workmap.clear_travel();
            self.workmap.hop = Some(hop);
        }
        self.workmap.phase = WorkmapPhase::Sailing;

        // An offer for the hop: take it.
        if let Some((offered, confirm, at)) = self.workmap.ask
            && now.saturating_sub(at) < ASK_FRESH_MS
        {
            if offered == hop {
                self.workmap.edge = Some(edge_of(hero.pos, geometry));
                out.extend(self.nav.stop(now, hero));
                out.push(if confirm {
                    Action::ConfirmJump
                } else {
                    Action::EdgeJump(hop)
                });
                self.workmap.phase = WorkmapPhase::Crossing;
                self.workmap.jump_since = Some(now);
                self.workmap.ask = None;
                tracing::info!(from = here, hop, "edge jump accepted");
                return true;
            }
            // Some other map is offered here: note it and try another edge.
            let edge = edge_of(hero.pos, geometry);
            self.workmap.learned.insert((here, edge), offered);
            self.workmap.learned_dirty = true;
            self.learn_step(here, edge, offered, &world.atlas);
            self.workmap.ask = None;
            if self.workmap.edge == Some(edge) {
                self.workmap.tried.push(edge);
                self.workmap.edge = None;
            }
        }

        let edge = match self.workmap.edge {
            Some(edge) => edge,
            None => {
                let Some(edge) = self.pick_edge(here, hop, &world.atlas) else {
                    self.workmap_failed(now, "edges");
                    return false;
                };
                tracing::info!(from = here, hop, edge, "edge picked");
                self.workmap.edge = Some(edge);
                self.workmap.at_edge_since = None;
                self.workmap.approaches = 0;
                edge
            }
        };
        let goal = edge_goal(edge, geometry);

        // Backing off after an edge visit without an offer.
        if let Some(back) = self.workmap.backing_off {
            let step = self
                .nav
                .drive(now, hero, back, Owner::Workmap, Some(geometry), terrain);
            if step.arrived || hero.pos.distance(back) <= EDGE_REACHED || step.blocked {
                self.workmap.backing_off = None;
            } else {
                out.extend(step.action);
            }
            return true;
        }

        let step = self
            .nav
            .drive(now, hero, goal, Owner::Workmap, Some(geometry), terrain);
        let at_edge = step.arrived || hero.pos.distance(goal) <= EDGE_REACHED;
        if step.blocked || self.nav.stuck_count() >= 3 || at_edge {
            let since = *self.workmap.at_edge_since.get_or_insert(now);
            if !at_edge || now.saturating_sub(since) >= ASK_WAIT_MS {
                self.workmap.at_edge_since = None;
                self.workmap.approaches += 1;
                self.nav.clear();
                if !at_edge || self.workmap.approaches > EDGE_APPROACHES {
                    tracing::info!(from = here, hop, edge, "edge gave no offer");
                    self.workmap.tried.push(edge);
                    self.workmap.edge = None;
                } else {
                    self.workmap.backing_off = Some(goal.toward(geometry.centre(), 15.0).rounded());
                }
            }
            return true;
        }
        out.extend(step.action);
        true
    }

    fn workmap_failed(&mut self, now: u64, why: &str) {
        let w = &mut self.workmap;
        w.failures += 1;
        w.clear_travel();
        if w.failures >= FAIL_LIMIT {
            w.blocked_until = Some(now + BLOCK_MS);
            w.phase = WorkmapPhase::Blocked;
        }
        self.emit(Level::Warning, EventCode::WorkmapFailed, &[why.into()]);
    }

    /// The next map on the shortest way to `target` over the world map grid
    /// and the edges seen so far.
    fn route(&self, from: i16, target: i16, atlas: &[MapCell]) -> Option<i16> {
        let havens = self.cfg.workmap.safe_havens;
        let neighbours = |id: i16| -> Vec<i16> {
            let mut out: Vec<i16> = self
                .workmap
                .learned
                .iter()
                .filter(|((f, _), _)| *f == id)
                .map(|(_, to)| *to)
                .collect();
            if let Some(c) = cell(atlas, id) {
                for other in atlas {
                    if step_between(c, other).is_some() && !out.contains(&other.id) {
                        out.push(other.id);
                    }
                }
            }
            out
        };
        let usable = |id: i16| id == target || cell(atlas, id).is_some_and(|c| havens || !c.safe_haven);
        let mut prev: HashMap<i16, i16> = HashMap::new();
        let mut queue = VecDeque::from([from]);
        let mut seen = HashSet::from([from]);
        while let Some(id) = queue.pop_front() {
            if id == target {
                let mut node = target;
                while let Some(&p) = prev.get(&node) {
                    if p == from {
                        return Some(node);
                    }
                    node = p;
                }
                return None;
            }
            for next in neighbours(id) {
                if usable(next) && seen.insert(next) {
                    prev.insert(next, id);
                    queue.push_back(next);
                }
            }
        }
        None
    }

    /// The edge to try for `hop`: a learnt one, else the one the grid step
    /// maps to, else any edge not yet tried that leads nowhere known.
    fn pick_edge(&self, here: i16, hop: i16, atlas: &[MapCell]) -> Option<usize> {
        let w = &self.workmap;
        let open = |e: &usize| !w.tried.contains(e);
        if let Some(edge) = (0..EDGES)
            .filter(open)
            .find(|e| w.learned.get(&(here, *e)) == Some(&hop))
        {
            return Some(edge);
        }
        let known_elsewhere = |e: usize| w.learned.get(&(here, e)).is_some_and(|to| *to != hop);
        if let (Some(a), Some(b)) = (cell(atlas, here), cell(atlas, hop))
            && let Some(step) = step_between(a, b)
            && let Some(edge) = w.step_edge[step].filter(|e| open(e) && !known_elsewhere(*e))
        {
            return Some(edge);
        }
        // Edges claimed by other grid steps go last.
        let claimed = |e: usize| w.step_edge.contains(&Some(e));
        (0..EDGES)
            .filter(open)
            .filter(|e| !known_elsewhere(*e))
            .min_by_key(|e| claimed(*e))
    }
}

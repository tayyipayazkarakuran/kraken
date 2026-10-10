use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use kraken_proto::Point;

use super::geometry::{Geometry, angle, angle_gap, polar};
use super::world::{Action, Eid, HeroState, Terrain};

pub const SEGMENT: f32 = 40.0;
pub const MOVE_INTERVAL_MS: u64 = 500;
/// Between moves on a timed map (El Dorado), where every moment counts.
pub const FAST_MOVE_INTERVAL_MS: u64 = 120;
const ARRIVE: f32 = 2.0;
const NEXT_SEGMENT_NEAR: f32 = 8.0;
const RESEND_AFTER_MS: u64 = 4_000;
const STUCK_IDLE_MS: u64 = 3_000;
const STUCK_MOVING_MS: u64 = 10_000;
const FAILED_DIR_MS: u64 = 90_000;
const ESCAPE_DISTANCE: f32 = 25.0;
const ESCAPE_MS: u64 = 6_000;
const MARGIN: f32 = 0.04;
const PATH_CHECK: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Roam(u32),
    Npc(Eid),
    Monster(Eid),
    Box(Eid),
    Repair,
    /// A map corner (El Dorado countdown spot).
    Corner,
    /// A map edge, on the way to the next work map.
    Workmap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavStep {
    pub action: Option<Action>,
    pub arrived: bool,
    pub stuck: bool,
    pub blocked: bool,
}

#[derive(Clone, Copy)]
struct Sent {
    point: Point,
    at: u64,
}

#[derive(Default)]
pub struct Navigator {
    owner: Option<Owner>,
    goal: Point,
    sent: Option<Sent>,
    best: f32,
    best_at: u64,
    escape: Option<(Point, u64)>,
    failed: Vec<(f32, u64)>,
    last_move: Option<u64>,
    stuck_count: u32,
    fast: bool,
}

impl Navigator {
    pub fn clear(&mut self) {
        let last_move = self.last_move;
        let failed = std::mem::take(&mut self.failed);
        *self = Self {
            last_move,
            failed,
            fast: self.fast,
            ..Self::default()
        };
    }

    pub fn reset_memory(&mut self) {
        self.clear();
        self.failed.clear();
    }

    pub fn owner(&self) -> Option<Owner> {
        self.owner
    }

    pub fn stuck_count(&self) -> u32 {
        self.stuck_count
    }

    pub fn waypoint(&self) -> Option<Point> {
        self.sent.map(|s| s.point)
    }

    /// Shorter gaps between moves (El Dorado).
    pub fn set_fast(&mut self, fast: bool) {
        self.fast = fast;
    }

    pub fn can_move(&self, now: u64) -> bool {
        let gap = if self.fast {
            FAST_MOVE_INTERVAL_MS
        } else {
            MOVE_INTERVAL_MS
        };
        self.last_move.is_none_or(|t| now.saturating_sub(t) >= gap)
    }

    pub fn stop(&mut self, now: u64, hero: &HeroState) -> Option<Action> {
        let moving = hero.moving != Some(false) && self.sent.is_some();
        self.clear();
        (moving && self.can_move(now)).then(|| {
            self.last_move = Some(now);
            Action::Move(hero.pos.rounded())
        })
    }

    pub fn drive(
        &mut self,
        now: u64,
        hero: &HeroState,
        goal: Point,
        owner: Owner,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
    ) -> NavStep {
        self.failed.retain(|(_, until)| *until > now);
        if self.owner != Some(owner) {
            self.clear();
            self.owner = Some(owner);
            self.restart(now, f32::INFINITY);
        } else if self.goal.distance(goal) > 3.0 {
            self.restart(now, f32::INFINITY);
            self.sent = None;
        }
        self.goal = goal;

        let distance = hero.pos.distance(goal);
        if distance <= ARRIVE {
            self.sent = None;
            self.escape = None;
            self.stuck_count = 0;
            return NavStep {
                arrived: true,
                ..NavStep::default()
            };
        }
        if distance < self.best - 0.5 {
            self.restart(now, distance);
        }

        let mut step = NavStep::default();
        if self.is_stuck(now, hero) {
            step.stuck = true;
            self.stuck_count += 1;
            let heading = self
                .sent
                .map_or_else(|| angle(hero.pos, goal), |s| angle(hero.pos, s.point));
            self.failed.push((heading, now + FAILED_DIR_MS));
            self.escape = self
                .escape_point(hero.pos, heading, geometry, terrain)
                .map(|p| (p, now + ESCAPE_MS));
            self.sent = None;
            self.restart(now, distance);
        }

        let target = match self.escape {
            Some((point, until)) if until > now && hero.pos.distance(point) > ARRIVE => point,
            _ => {
                self.escape = None;
                goal
            }
        };

        let Some(segment) = self.segment(hero.pos, target, geometry, terrain) else {
            step.blocked = true;
            return step;
        };
        if self.should_send(now, hero, segment) {
            self.sent = Some(Sent {
                point: segment,
                at: now,
            });
            self.last_move = Some(now);
            step.action = Some(Action::Move(segment));
        }
        step
    }

    fn restart(&mut self, now: u64, best: f32) {
        self.best = best;
        self.best_at = now;
    }

    fn is_stuck(&self, now: u64, hero: &HeroState) -> bool {
        let Some(sent) = self.sent else { return false };
        if now.saturating_sub(sent.at) < 1_500 || hero.movement_enabled == Some(false) {
            return false;
        }
        let window = if hero.moving == Some(false) {
            STUCK_IDLE_MS
        } else {
            STUCK_MOVING_MS
        };
        now.saturating_sub(self.best_at) > window
    }

    fn should_send(&self, now: u64, hero: &HeroState, segment: Point) -> bool {
        if !self.can_move(now) {
            return false;
        }
        let Some(sent) = self.sent else { return true };
        if sent.point.distance(segment) <= 4.0 {
            let lost = hero.moving == Some(false) && now.saturating_sub(sent.at) > RESEND_AFTER_MS;
            return lost && now.saturating_sub(self.best_at) > RESEND_AFTER_MS;
        }
        hero.pos.distance(sent.point) <= NEXT_SEGMENT_NEAR || now.saturating_sub(sent.at) > 2_000
    }

    fn segment(
        &self,
        from: Point,
        to: Point,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
    ) -> Option<Point> {
        let direct = from.toward(to, SEGMENT).rounded();
        let clear_path = |p: Point| {
            let length = from.distance(p);
            let checks = (length / PATH_CHECK).ceil().max(1.0) as u32;
            (1..=checks).all(|i| {
                let at = from.toward(p, length * i as f32 / checks as f32);
                terrain.blocked(at) != Some(true)
            })
        };
        let usable =
            |p: Point| geometry.is_none_or(|g| g.contains(p, 0.0) || !g.contains(to, 0.0)) && clear_path(p);
        if usable(direct) {
            return Some(direct);
        }
        let base = angle(from, to);
        let reach = from.distance(to).min(SEGMENT);
        [0.35f32, -0.35, 0.7, -0.7, 1.05, -1.05]
            .into_iter()
            .map(|offset| polar(from, base + offset, reach).rounded())
            .find(|p| usable(*p))
    }

    fn escape_point(
        &self,
        from: Point,
        heading: f32,
        geometry: Option<Geometry>,
        terrain: &dyn Terrain,
    ) -> Option<Point> {
        [
            FRAC_PI_2,
            -FRAC_PI_2,
            PI,
            FRAC_PI_4 * 3.0,
            -FRAC_PI_4 * 3.0,
            FRAC_PI_4,
            -FRAC_PI_4,
        ]
        .into_iter()
        .map(|turn| heading + turn)
        .filter(|a| self.failed.iter().all(|(f, _)| angle_gap(*a, *f) > 0.5))
        .map(|a| polar(from, a, ESCAPE_DISTANCE).rounded())
        .find(|p| geometry.is_none_or(|g| g.contains(*p, MARGIN)) && terrain.blocked(*p) != Some(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::world::OpenWater;

    fn hero_at(x: f32, y: f32) -> HeroState {
        HeroState {
            pos: Point::new(x, y),
            hp: 100.0,
            hp_max: 100.0,
            hp_known: true,
            ..HeroState::default()
        }
    }

    #[test]
    fn splits_long_paths_into_segments() {
        let mut nav = Navigator::default();
        let step = nav.drive(
            0,
            &hero_at(0.0, 0.0),
            Point::new(140.0, 0.0),
            Owner::Roam(1),
            None,
            &OpenWater,
        );
        assert_eq!(step.action, Some(Action::Move(Point::new(40.0, 0.0))));
    }

    #[test]
    fn respects_move_interval_and_does_not_resend_while_progressing() {
        let mut nav = Navigator::default();
        let goal = Point::new(140.0, 0.0);
        assert!(
            nav.drive(0, &hero_at(0.0, 0.0), goal, Owner::Roam(1), None, &OpenWater)
                .action
                .is_some()
        );
        assert!(
            nav.drive(200, &hero_at(2.0, 0.0), goal, Owner::Roam(1), None, &OpenWater)
                .action
                .is_none()
        );
        assert!(
            nav.drive(900, &hero_at(10.0, 0.0), goal, Owner::Roam(1), None, &OpenWater)
                .action
                .is_none()
        );
        let next = nav.drive(1_800, &hero_at(34.0, 0.0), goal, Owner::Roam(1), None, &OpenWater);
        assert_eq!(next.action, Some(Action::Move(Point::new(74.0, 0.0))));
    }

    #[test]
    fn detects_stuck_and_escapes_sideways() {
        let mut nav = Navigator::default();
        let goal = Point::new(140.0, 0.0);
        let mut hero = hero_at(0.0, 0.0);
        nav.drive(0, &hero, goal, Owner::Roam(1), None, &OpenWater);
        hero.moving = Some(false);
        let step = nav.drive(3_600, &hero, goal, Owner::Roam(1), None, &OpenWater);
        assert!(step.stuck);
        let Some(Action::Move(p)) = step.action else {
            panic!("expected escape move")
        };
        assert!(p.x.abs() < 1.0 && (p.y.abs() - 25.0).abs() < 1.0);
    }

    #[test]
    fn avoids_blocked_cells() {
        struct Wall;
        impl Terrain for Wall {
            fn blocked(&self, p: Point) -> Option<bool> {
                Some(p.y.abs() < 5.0 && p.x > 30.0)
            }
        }
        let mut nav = Navigator::default();
        let step = nav.drive(
            0,
            &hero_at(0.0, 0.0),
            Point::new(100.0, 0.0),
            Owner::Roam(1),
            None,
            &Wall,
        );
        let Some(Action::Move(p)) = step.action else {
            panic!("expected move")
        };
        assert!(p.y.abs() >= 5.0);
    }

    #[test]
    fn arrival_is_reported() {
        let mut nav = Navigator::default();
        let step = nav.drive(
            0,
            &hero_at(10.0, 10.0),
            Point::new(11.0, 10.0),
            Owner::Roam(1),
            None,
            &OpenWater,
        );
        assert!(step.arrived && step.action.is_none());
    }
}

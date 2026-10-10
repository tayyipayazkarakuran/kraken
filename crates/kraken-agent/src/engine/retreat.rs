//! Sailing to a map corner: the nearest one before a map jump (El Dorado,
//! bonus maps), the one away from the enemy while repairing. A corner tip
//! often sits on land or the map edge, so each corner is pulled towards the
//! centre to the first open water, and a corner that cannot be reached is
//! dropped for the next one instead of giving up on the retreat.

use kraken_proto::Point;

use super::ROAM_MARGIN;
use super::geometry::{Geometry, angle, angle_gap};
use super::nav::{Navigator, Owner};
use super::world::{Action, HeroState, Terrain};

/// Close enough to the corner.
pub(super) const CORNER_REACHED: f32 = 4.0;
/// How far a corner may be pulled in to find open water.
const POCKET_DEPTH: f32 = 40.0;
const POCKET_STEP: f32 = 4.0;
/// A flight path may not pass the enemy closer than this share of the
/// current distance (it would sail back into its guns).
const PASS_SHARE: f32 = 0.7;
/// The enemy moved round this far: pick the flight corner again.
const TURN: f32 = std::f32::consts::FRAC_PI_2;

/// The corner, pulled in until it is open water; `None` if all of it is land.
fn pocket(corner: Point, geometry: Geometry, terrain: &dyn Terrain) -> Option<Point> {
    let centre = geometry.centre();
    let steps = (POCKET_DEPTH / POCKET_STEP) as u32;
    (0..=steps)
        .map(|i| corner.toward(centre, i as f32 * POCKET_STEP).rounded())
        .find(|p| geometry.contains(*p, 0.0) && terrain.blocked(*p) != Some(true))
}

/// The reachable corners, nearest first.
pub(super) fn corners_by_distance(pos: Point, geometry: Geometry, terrain: &dyn Terrain) -> Vec<Point> {
    let mut corners: Vec<Point> = geometry
        .corners(ROAM_MARGIN)
        .into_iter()
        .filter_map(|c| pocket(c, geometry, terrain))
        .collect();
    corners.sort_by(|a, b| pos.distance(*a).total_cmp(&pos.distance(*b)));
    corners
}

/// Closest distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

/// The corner to flee to from `threat`: the one most directly away from it
/// whose path does not run back past it; ties go to the farther one.
pub(super) fn flee_corner(
    hero: Point,
    threat: Point,
    geometry: Geometry,
    terrain: &dyn Terrain,
    skip: &[Point],
) -> Option<Point> {
    let away = angle(threat, hero);
    let gap = threat.distance(hero);
    corners_by_distance(hero, geometry, terrain)
        .into_iter()
        .filter(|c| !skip.iter().any(|s| s.distance(*c) < 1.0))
        .filter(|c| {
            hero.distance(*c) <= CORNER_REACHED || segment_distance(threat, hero, *c) >= gap * PASS_SHARE
        })
        .min_by(|a, b| {
            let turn = |c: &Point| {
                if hero.distance(*c) <= CORNER_REACHED {
                    0.0
                } else {
                    angle_gap(away, angle(hero, *c))
                }
            };
            turn(a)
                .total_cmp(&turn(b))
                .then(threat.distance(*b).total_cmp(&threat.distance(*a)))
        })
}

/// Whether the enemy has come round far enough to rethink the flight corner.
pub(super) fn turned(previous: f32, hero: Point, threat: Point) -> bool {
    angle_gap(previous, angle(threat, hero)) > TURN
}

/// A retreat to the nearest reachable corner, moving on to the next one when
/// the way is blocked.
#[derive(Debug, Default)]
pub(super) struct CornerRetreat {
    order: Option<Vec<Point>>,
    index: usize,
}

pub(super) enum Sail {
    /// Still on the way (maybe a move to send).
    Sailing(Option<Action>),
    /// At the corner, or no corner could be reached.
    Done,
}

impl CornerRetreat {
    pub(super) fn sail(
        &mut self,
        nav: &mut Navigator,
        now: u64,
        hero: &HeroState,
        geometry: Geometry,
        terrain: &dyn Terrain,
    ) -> Sail {
        let order = self.order.get_or_insert_with(|| {
            let order = corners_by_distance(hero.pos, geometry, terrain);
            tracing::info!(pos = ?hero.pos, corners = ?order, "corner pick");
            order
        });
        while let Some(&goal) = order.get(self.index) {
            let step = nav.drive(now, hero, goal, Owner::Corner, Some(geometry), terrain);
            if step.arrived || hero.pos.distance(goal) <= CORNER_REACHED {
                return Sail::Done;
            }
            if step.blocked || nav.stuck_count() >= 3 {
                tracing::info!(corner = ?goal, blocked = step.blocked, "corner unreachable, next");
                self.index += 1;
                nav.clear();
                continue;
            }
            return Sail::Sailing(step.action);
        }
        Sail::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::world::OpenWater;

    fn diamond() -> Geometry {
        Geometry::new(600.0, 600.0).unwrap()
    }

    #[test]
    fn nearest_corner_first() {
        let order = corners_by_distance(Point::new(300.0, 200.0), diamond(), &OpenWater);
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], Point::new(300.0, 276.0));
    }

    #[test]
    fn blocked_corner_tip_is_pulled_in() {
        struct Tip;
        impl Terrain for Tip {
            fn blocked(&self, p: Point) -> Option<bool> {
                Some(p.y > 260.0)
            }
        }
        let order = corners_by_distance(Point::new(300.0, 200.0), diamond(), &Tip);
        assert!(order.contains(&Point::new(300.0, 260.0)));
    }

    #[test]
    fn flees_to_the_corner_opposite_the_enemy() {
        // Enemy east of the hero: the west corner.
        let hero = Point::new(300.0, 0.0);
        let corner = flee_corner(hero, Point::new(340.0, 0.0), diamond(), &OpenWater, &[]);
        assert_eq!(corner, Some(Point::new(24.0, 0.0)));
    }

    #[test]
    fn never_flees_past_the_enemy() {
        // Hero near the west corner, enemy between it and the centre: the
        // west corner is behind the hero, so a side corner it is not;
        // north/south both run past the enemy less than the east one.
        let hero = Point::new(60.0, 0.0);
        let threat = Point::new(90.0, 0.0);
        let corner = flee_corner(hero, threat, diamond(), &OpenWater, &[]).unwrap();
        assert_eq!(corner, Point::new(24.0, 0.0));
        // With the west corner gone, it does not pick the east corner.
        let corner = flee_corner(hero, threat, diamond(), &OpenWater, &[Point::new(24.0, 0.0)]);
        assert_ne!(corner, Some(Point::new(576.0, 0.0)));
    }
}

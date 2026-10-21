use kraken_proto::{
    Activity, BoxTarget, Capabilities, Control, EntityKind, NpcTarget, Point, RangeStyle, RoamMode, RunState,
    Stock, StockKind, WorkMode,
};

use super::Engine;
use super::world::{
    Action, Actor, Eid, Feedback, HeroState, MapKind, MapState, OpenWater, Relation, RepairBlock, World,
};

const HERO: Eid = Eid::new(1, 1);

fn caps() -> Capabilities {
    Capabilities {
        world: true,
        movement: true,
        cannon: true,
        harpoon: true,
        abort_attack: true,
        repair: true,
        respawn: true,
        reconnect: true,
        terrain: false,
        screen: false,
        select_target: true,
        use_item: true,
    }
}

fn world(hero: Point) -> World {
    World {
        connected: true,
        ready: true,
        map: Some(MapState {
            id: 33,
            kind: MapKind::Normal,
            kind_name: "PVE".into(),
            width: 600.0,
            height: 600.0,
            atlantis: false,
        }),
        hero: Some(HeroState {
            id: HERO,
            name: "Captain".into(),
            pos: hero,
            hp: 1000.0,
            hp_max: 1000.0,
            hp_known: true,
            moving: Some(false),
            movement_enabled: Some(true),
            cannon_range: 14.0,
            harpoon_range: 10.0,
            repairing: Some(false),
            repair_block: None,
            base_cannon_range: 14.0,
            vp: 0.0,
            vp_max: 0.0,
            selected: None,
        }),
        actors: Vec::new(),
        stock: vec![
            Stock {
                kind: StockKind::Ammo,
                id: 3,
                amount: 500,
            },
            Stock {
                kind: StockKind::Harpoon,
                id: 18,
                amount: 50,
            },
        ],
        stock_known: true,
        items: Vec::new(),
        skills: Vec::new(),
        bonus: Vec::new(),
        torpedo: Default::default(),
        atlas: Vec::new(),
        teleports: 0,
        teleport_ready: false,
    }
}

fn npc(id: i64, type_id: i32, x: f32, y: f32) -> Actor {
    Actor {
        id: Eid::new(id, 1),
        kind: EntityKind::Npc,
        type_id,
        name: format!("Npc{type_id}"),
        pos: Point::new(x, y),
        hp_percent: Some(100.0),
        visible: true,
        relation: Relation::Unknown,
    }
}

fn chest(id: i64, x: f32, y: f32) -> Actor {
    Actor {
        id: Eid::new(id, 1),
        kind: EntityKind::Box,
        type_id: 7,
        name: "Chest".into(),
        pos: Point::new(x, y),
        hp_percent: None,
        visible: true,
        relation: Relation::Unknown,
    }
}

fn running(mode: WorkMode) -> Engine {
    let mut engine = Engine::new(42);
    let mut cfg = engine.config().clone();
    // Deterministic timing for the tests; humanization is exercised on its own.
    cfg.humanize.enabled = false;
    cfg.mode = mode;
    cfg.npc.targets.push(NpcTarget {
        type_id: 5,
        name: "Npc5".into(),
        priority: 1,
        ammo: Some(3),
        torpedo: None,
        range: RangeStyle::Default,
        items: Vec::new(),
    });
    cfg.collector.boxes.push(BoxTarget {
        type_id: 7,
        name: "Chest".into(),
        priority: 1,
    });
    engine.configure(cfg);
    engine.control(0, Control::Start, &World::default());
    engine
}

fn moves(actions: &[Action]) -> Vec<Point> {
    actions
        .iter()
        .filter_map(|a| if let Action::Move(p) = a { Some(*p) } else { None })
        .collect()
}

#[test]
fn approaches_and_fires_at_listed_npc() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 360.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Npc);
    assert_eq!(moves(&actions), vec![Point::new(340.0, 0.0)]);

    w.hero.as_mut().unwrap().pos = Point::new(350.0, 0.0);
    let actions = engine.step(1_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Cannon {
        target: Eid::new(10, 1),
        ammo: 3
    }));
}

#[test]
fn ignores_unlisted_npc_unless_attack_all() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(11, 99, 305.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);

    let mut cfg = engine.config().clone();
    cfg.npc.attack_all = true;
    engine.configure(cfg);
    let actions = engine.step(1_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Cannon {
        target: Eid::new(11, 1),
        ammo: 3
    }));
}

/// With humanization on, a backward game clock (an anti-cheat signal) holds
/// every action until the clock is sane again.
#[test]
fn humanize_clock_guard_pauses_then_resumes() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.humanize.enabled = true;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 355.0, 0.0));
    engine.step(10_000, &w, &caps(), &OpenWater);
    let back = engine.step(100, &w, &caps(), &OpenWater);
    assert!(back.is_empty(), "actions held while the clock is backwards");
    assert_eq!(engine.activity(), Activity::Recovering);
    engine.step(10_001, &w, &caps(), &OpenWater);
    assert_ne!(engine.activity(), Activity::Recovering, "resumes once sane");
}

/// Humanization delays and spreads fire, but the bot still engages: well past
/// the reaction window it fires at a listed NPC in range.
#[test]
fn humanize_still_fires_after_reaction() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.humanize.enabled = true;
    cfg.humanize.reaction_ms = 500;
    engine.configure(cfg);
    let mut w = world(Point::new(350.0, 0.0));
    w.actors.push(npc(10, 5, 355.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    let actions = engine.step(5_000, &w, &caps(), &OpenWater);
    assert!(
        actions.iter().any(|a| matches!(a, Action::Cannon { .. })),
        "fires once the reaction delay has passed"
    );
}

#[test]
fn counts_kill_after_confirmed_hits_and_removal() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 305.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    engine.feedback(
        500,
        Feedback::Hit {
            source: HERO,
            target: Eid::new(10, 1),
        },
    );
    engine.feedback(900, Feedback::Removed(Eid::new(10, 1)));
    w.actors.clear();
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.counters(1_000).npc_kills, 1);
}

#[test]
fn skips_target_without_hits() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 305.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    let actions = engine.step(21_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::AbortAttack));
    engine.step(22_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);
}

#[test]
fn stops_using_empty_ammo() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.stock[0].amount = 0;
    w.actors.push(npc(10, 5, 305.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert!(!actions.iter().any(|a| matches!(a, Action::Cannon { .. })));
    assert!(
        engine
            .drain_events()
            .iter()
            .any(|e| e.code == kraken_proto::EventCode::NoAmmo)
    );
}

#[test]
fn collects_box_and_confirms_by_removal() {
    let mut engine = running(WorkMode::Collect);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(chest(20, 320.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(moves(&actions), vec![Point::new(320.0, 0.0)]);
    w.hero.as_mut().unwrap().pos = Point::new(319.5, 0.0);
    engine.step(1_000, &w, &caps(), &OpenWater);
    engine.feedback(1_200, Feedback::Removed(Eid::new(20, 1)));
    w.actors.clear();
    engine.step(1_300, &w, &caps(), &OpenWater);
    assert_eq!(engine.counters(1_300).boxes, 1);
}

#[test]
fn skips_box_that_never_disappears() {
    let mut engine = running(WorkMode::Collect);
    let mut w = world(Point::new(320.0, 0.0));
    w.actors.push(chest(20, 320.5, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    engine.step(2_500, &w, &caps(), &OpenWater);
    engine.step(3_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);
}

#[test]
fn mixed_mode_prefers_combat_by_default() {
    let mut engine = running(WorkMode::Mixed);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(chest(20, 302.0, 0.0));
    w.actors.push(npc(10, 5, 330.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Npc);
}

#[test]
fn repairs_below_threshold_and_finishes() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().hp = 300.0;
    engine.step(10_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Repairing);
    let actions = engine.step(10_100, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Repair(true)));
    w.hero.as_mut().unwrap().hp = 950.0;
    let actions = engine.step(12_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Repair(false)));
}

#[test]
fn repair_flees_to_the_corner_away_from_the_attacker() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().hp = 300.0;
    // The ship firing sits east; a closer, quiet one sits south.
    w.actors.push(npc(10, 9, 340.0, 0.0));
    w.actors.push(npc(11, 9, 300.0, -20.0));
    engine.step(10_000, &w, &caps(), &OpenWater);
    engine.feedback(
        10_050,
        Feedback::Hit {
            source: Eid::new(10, 1),
            target: HERO,
        },
    );
    // The hit sends the pending request back to the retreat; it sails next.
    engine.step(10_100, &w, &caps(), &OpenWater);
    let first = moves(&engine.step(10_200, &w, &caps(), &OpenWater));
    let Some(p) = first.first() else {
        panic!("expected a flight move")
    };
    assert!(p.x < 270.0 && p.y.abs() < 1.0, "flies west, got {p:?}");
    // The chosen corner holds while the ship moves and is hit again.
    for (i, at) in [11_000u64, 12_000, 13_000].into_iter().enumerate() {
        w.hero.as_mut().unwrap().pos = Point::new(p.x - 10.0 * i as f32, 0.0);
        engine.feedback(
            at - 50,
            Feedback::Hit {
                source: Eid::new(10, 1),
                target: HERO,
            },
        );
        for m in moves(&engine.step(at, &w, &caps(), &OpenWater)) {
            assert!(
                m.y.abs() < 1.0 && m.x < w.hero.as_ref().unwrap().pos.x,
                "kept west, got {m:?}"
            );
        }
    }
}

#[test]
fn respawns_after_death_with_retries_then_pauses() {
    let mut engine = running(WorkMode::Combat);
    let w = world(Point::new(300.0, 0.0));
    engine.feedback(0, Feedback::OwnDeath);
    assert!(engine.step(1_000, &w, &caps(), &OpenWater).is_empty());
    for at in [3_100, 11_200, 19_300] {
        assert!(
            engine
                .step(at, &w, &caps(), &OpenWater)
                .contains(&Action::Respawn)
        );
    }
    engine.step(27_400, &w, &caps(), &OpenWater);
    assert_eq!(engine.run_state(), RunState::Paused);
}

fn on_map(w: &mut World, id: i32, kind: MapKind) {
    let map = w.map.as_mut().unwrap();
    map.id = id;
    map.kind = kind;
}

#[test]
fn death_map_triggers_standard_respawn_then_repair() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);

    engine.feedback(100, Feedback::OwnDeath);
    engine.feedback(200, Feedback::MapChanged);
    on_map(&mut w, 500, MapKind::Death);
    w.hero.as_mut().unwrap().hp = 200.0;
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Dead);
    let actions = engine.step(3_500, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Respawn));
    assert_eq!(engine.run_state(), RunState::Running);

    engine.feedback(4_000, Feedback::MapChanged);
    on_map(&mut w, 33, MapKind::Normal);
    w.hero.as_mut().unwrap().hp = 400.0;
    let actions = engine.step(5_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::CloseDeathWindow));
    assert_eq!(engine.activity(), Activity::Repairing);
    let actions = engine.step(5_100, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Repair(true)));
    assert_eq!(engine.run_state(), RunState::Running);
    w.hero.as_mut().unwrap().hp = 950.0;
    let actions = engine.step(8_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Repair(false)));
    engine.step(8_500, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);
}

#[test]
fn bot_started_on_death_map_respawns() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    on_map(&mut w, 500, MapKind::Death);
    engine.step(0, &w, &caps(), &OpenWater);
    assert!(
        engine
            .step(3_100, &w, &caps(), &OpenWater)
            .contains(&Action::Respawn)
    );
}

#[test]
fn unknown_hp_is_not_death() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    let hero = w.hero.as_mut().unwrap();
    hero.hp = 0.0;
    hero.hp_known = false;
    engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);
}

#[test]
fn attack_all_ignores_hidden_and_distant_npcs() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.npc.attack_all = true;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    let mut hidden = npc(11, 99, 310.0, 0.0);
    hidden.visible = false;
    w.actors.push(hidden);
    w.actors.push(npc(12, 98, 300.0, 250.0));
    engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.activity(), Activity::Roaming);

    w.actors.push(npc(13, 97, 320.0, 0.0));
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.target(&w).map(|t| t.type_id), Some(97));
}

#[test]
fn listed_priority_ten_beats_unlisted() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.npc.attack_all = true;
    cfg.npc.targets[0].priority = 10;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(11, 99, 305.0, 0.0));
    w.actors.push(npc(10, 5, 330.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(engine.target(&w).map(|t| t.type_id), Some(5));
}

#[test]
fn keeps_engaged_target_when_same_priority_npc_comes_closer() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert!(actions.iter().any(|a| matches!(a, Action::Cannon { .. })));
    w.actors.push(npc(11, 5, 302.0, 0.0));
    let actions = engine.step(3_000, &w, &caps(), &OpenWater);
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::Cannon { target, .. } if target.id == 11))
    );
    assert_eq!(engine.target(&w).map(|t| t.distance as i32), Some(10));
}

#[test]
fn untouched_target_gives_way_to_higher_priority() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.npc.targets[0].priority = 5;
    cfg.npc.targets.push(NpcTarget {
        type_id: 6,
        name: "Npc6".into(),
        priority: 1,
        ammo: Some(3),
        torpedo: None,
        range: RangeStyle::Default,
        items: Vec::new(),
    });
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 350.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    w.actors.push(npc(11, 6, 355.0, 0.0));
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.target(&w).map(|t| t.type_id), Some(6));
}

#[test]
fn keeps_target_that_briefly_leaves_view() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    w.actors[0].visible = false;
    w.actors.push(npc(11, 5, 305.0, 0.0));
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.target(&w).map(|t| t.distance as i32), Some(10));
}

#[test]
fn falling_hp_counts_as_progress_without_hit_messages() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    for (step, hp) in (0..30u64).zip((0..30).map(|i| 100.0 - i as f32)) {
        w.actors[0].hp_percent = Some(hp);
        let actions = engine.step(step * 1_000, &w, &caps(), &OpenWater);
        assert!(!actions.contains(&Action::AbortAttack), "dropped at {step}s");
    }
    assert_eq!(engine.activity(), Activity::Npc);
}

#[test]
fn box_during_fight_does_not_stop_the_attack() {
    let mut engine = running(WorkMode::Mixed);
    let mut cfg = engine.config().clone();
    cfg.mixed_priority = kraken_proto::MixedPriority::Collect;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    w.actors.push(chest(20, 330.0, 0.0));
    let actions = engine.step(3_000, &w, &caps(), &OpenWater);
    assert!(!actions.contains(&Action::AbortAttack));
    assert_eq!(engine.activity(), Activity::Npc);
}

fn detouring(engine: &mut Engine) {
    let mut cfg = engine.config().clone();
    cfg.collector.enabled = true;
    cfg.collector.during_combat = true;
    engine.configure(cfg);
}

fn fires(actions: &[Action]) -> bool {
    actions.iter().any(|a| matches!(a, Action::Cannon { .. }))
}

#[test]
fn detours_to_box_inside_range_and_keeps_firing() {
    let mut engine = running(WorkMode::Mixed);
    detouring(&mut engine);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    w.actors.push(chest(20, 305.0, 6.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert_eq!(moves(&actions), vec![Point::new(305.0, 6.0)], "{actions:?}");
    assert!(fires(&actions), "{actions:?}");
    assert!(!actions.contains(&Action::AbortAttack));
    assert_eq!(engine.activity(), Activity::Npc);
}

#[test]
fn box_that_would_leave_firing_range_is_not_detoured_to() {
    let mut engine = running(WorkMode::Mixed);
    detouring(&mut engine);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    // Close to the hero but 12.8 from the target (range 14, limit 85%).
    w.actors.push(chest(20, 300.0, 8.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert!(moves(&actions).is_empty(), "{actions:?}");
    assert!(fires(&actions));
}

#[test]
fn no_detour_before_the_target_is_in_range() {
    let mut engine = running(WorkMode::Mixed);
    detouring(&mut engine);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 330.0, 0.0));
    w.actors.push(chest(20, 305.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert!(!moves(&actions).contains(&Point::new(305.0, 0.0)), "{actions:?}");
}

#[test]
fn detour_is_dropped_when_the_target_moves_away() {
    let mut engine = running(WorkMode::Mixed);
    detouring(&mut engine);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 310.0, 0.0));
    w.actors.push(chest(20, 305.0, 6.0));
    engine.step(0, &w, &caps(), &OpenWater);
    w.actors[0].pos = Point::new(322.0, 0.0);
    let actions = engine.step(500, &w, &caps(), &OpenWater);
    assert!(!moves(&actions).contains(&Point::new(305.0, 6.0)), "{actions:?}");
    assert!(!actions.contains(&Action::AbortAttack));
}

#[test]
fn repair_waits_out_cooldown_instead_of_pausing() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().hp = 300.0;
    w.hero.as_mut().unwrap().repair_block = Some(RepairBlock::Cooldown);
    for at in (0..60).map(|s| s * 1_000) {
        let actions = engine.step(at, &w, &caps(), &OpenWater);
        assert!(!actions.contains(&Action::Repair(true)));
    }
    assert_eq!(engine.run_state(), RunState::Running);
    assert_eq!(engine.activity(), Activity::Repairing);
    w.hero.as_mut().unwrap().repair_block = None;
    let actions = engine.step(61_000, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::Repair(true)));
}

#[test]
fn unanswered_repair_retries_without_pausing() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().hp = 300.0;
    let mut sent = 0;
    for at in (0..80).map(|s| s * 1_000) {
        sent += engine
            .step(at, &w, &caps(), &OpenWater)
            .iter()
            .filter(|a| **a == Action::Repair(true))
            .count();
    }
    assert!(sent > 4, "{sent}");
    assert_eq!(engine.run_state(), RunState::Running);
}

#[test]
fn repair_aborts_an_attack_in_progress() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().hp = 300.0;
    w.hero.as_mut().unwrap().repair_block = Some(RepairBlock::Attacking);
    engine.step(0, &w, &caps(), &OpenWater);
    let actions = engine.step(3_500, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::AbortAttack));
}

#[test]
fn warns_when_harpoon_range_is_missing() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.monster.attack_all = true;
    cfg.monster.enabled = true;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.hero.as_mut().unwrap().harpoon_range = 0.0;
    let mut monster = npc(30, 4, 305.0, 0.0);
    monster.kind = EntityKind::Monster;
    w.actors.push(monster);
    engine.step(0, &w, &caps(), &OpenWater);
    assert!(
        engine
            .drain_events()
            .iter()
            .any(|e| e.code == kraken_proto::EventCode::NoHarpoonRange)
    );
}

#[test]
fn reconnects_with_backoff_and_blocks_fatal_reasons() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.connected = false;
    engine.feedback(0, Feedback::Disconnected("TIMEOUT".into()));
    assert!(engine.step(0, &w, &caps(), &OpenWater).is_empty());
    assert!(
        engine
            .step(10_000, &w, &caps(), &OpenWater)
            .contains(&Action::Reconnect)
    );
    assert!(engine.step(15_000, &w, &caps(), &OpenWater).is_empty());
    assert!(
        engine
            .step(30_000, &w, &caps(), &OpenWater)
            .contains(&Action::Reconnect)
    );
    w.connected = true;
    engine.step(31_000, &w, &caps(), &OpenWater);
    assert_eq!(engine.run_state(), RunState::Running);

    let mut engine = running(WorkMode::Combat);
    w.connected = false;
    engine.feedback(0, Feedback::Disconnected("USER_AREADY_LOGGED_IN".into()));
    engine.step(0, &w, &caps(), &OpenWater);
    assert!(engine.step(60_000, &w, &caps(), &OpenWater).is_empty());
    assert_eq!(engine.run_state(), RunState::Paused);
}

/// A server reset / maintenance waits the configured long delay (15 min by
/// default) before the first reconnect, instead of retrying every minute.
#[test]
fn long_outage_waits_the_configured_delay() {
    let mut engine = running(WorkMode::Combat);
    let wait_ms = 15 * 60_000;
    let mut w = world(Point::new(300.0, 0.0));
    w.connected = false;
    engine.feedback(0, Feedback::Disconnected("MAINTENANCE".into()));
    assert!(engine.step(0, &w, &caps(), &OpenWater).is_empty());
    assert!(
        engine.step(wait_ms - 1_000, &w, &caps(), &OpenWater).is_empty(),
        "no reconnect while still inside the wait"
    );
    assert!(
        engine
            .step(wait_ms, &w, &caps(), &OpenWater)
            .contains(&Action::Reconnect),
        "reconnects once the long wait has passed"
    );
}

/// A generic outage retries fast at first, then settles into the long wait
/// once enough tries have failed (the game has been down a while).
#[test]
fn generic_outage_escalates_to_long_wait() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.safety.reconnect_wait_mins = 5;
    engine.configure(cfg);
    let mut w = world(Point::new(300.0, 0.0));
    w.connected = false;
    engine.feedback(0, Feedback::Disconnected("TIMEOUT".into()));
    // Drive several fast retries to climb past the escalation threshold.
    let mut now = 0u64;
    let mut reconnects = 0;
    for _ in 0..8 {
        now += 60_000;
        if engine
            .step(now, &w, &caps(), &OpenWater)
            .contains(&Action::Reconnect)
        {
            reconnects += 1;
        }
    }
    // After escalation the gap is 5 min, so a +1 min step no longer reconnects.
    let quiet = engine.step(now + 60_000, &w, &caps(), &OpenWater);
    assert!(reconnects >= 6, "fast retries happened first");
    assert!(
        !quiet.contains(&Action::Reconnect),
        "then it waits the long delay"
    );
}

#[test]
fn never_works_in_safe_haven() {
    let mut engine = running(WorkMode::Mixed);
    let mut w = world(Point::new(300.0, 0.0));
    w.map.as_mut().unwrap().kind = MapKind::SafeHaven;
    w.actors.push(npc(10, 5, 305.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    assert!(actions.is_empty());
    assert_eq!(engine.activity(), Activity::Idle);
}

#[test]
fn roams_corners_inside_bounds() {
    let mut engine = running(WorkMode::Combat);
    let mut cfg = engine.config().clone();
    cfg.navigation.roam = RoamMode::Corners;
    engine.configure(cfg);
    let w = world(Point::new(300.0, 0.0));
    let actions = engine.step(0, &w, &caps(), &OpenWater);
    let p = moves(&actions)[0];
    assert!(p.distance(Point::new(300.0, 0.0)) <= 40.5);
}

#[test]
fn pause_aborts_attack_and_stops_ship() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    w.actors.push(npc(10, 5, 360.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    w.hero.as_mut().unwrap().moving = Some(true);
    let actions = engine.control(1_000, Control::Pause, &w);
    assert!(actions.contains(&Action::AbortAttack));
    assert!(actions.contains(&Action::Move(Point::new(300.0, 0.0))));
    assert!(engine.step(2_000, &w, &caps(), &OpenWater).is_empty());
}

#[test]
fn leaving_death_map_counts_as_respawn_without_messages() {
    let mut engine = running(WorkMode::Combat);
    let mut w = world(Point::new(300.0, 0.0));
    engine.step(0, &w, &caps(), &OpenWater);
    on_map(&mut w, 500, MapKind::Death);
    w.hero.as_mut().unwrap().hp = 100.0;
    engine.step(1_000, &w, &caps(), &OpenWater);
    assert!(
        engine
            .step(4_100, &w, &caps(), &OpenWater)
            .contains(&Action::Respawn)
    );
    on_map(&mut w, 33, MapKind::Normal);
    let actions = engine.step(4_300, &w, &caps(), &OpenWater);
    assert!(actions.contains(&Action::CloseDeathWindow));
    for at in (5..30).map(|s| s * 1_000) {
        assert!(
            !engine
                .step(at, &w, &caps(), &OpenWater)
                .contains(&Action::Respawn)
        );
    }
    assert_eq!(engine.run_state(), RunState::Running);
}

mod war {
    use kraken_proto::{EnemyScope, ItemWhen, WarItemRule};

    use super::*;
    use crate::engine::world::ActionItem;

    const HAILSTORM: i16 = 28;
    const WINDSTORM: i16 = 29;
    const LEGBA: i16 = 45;
    const BROTHERS: i16 = 47;

    fn player(id: i64, name: &str, x: f32, relation: Relation) -> Actor {
        Actor {
            id: Eid::new(id, 1),
            kind: EntityKind::Player,
            type_id: 0,
            name: name.into(),
            pos: Point::new(x, 300.0),
            hp_percent: Some(100.0),
            visible: true,
            relation,
        }
    }

    fn arena() -> World {
        let mut w = world(Point::new(300.0, 300.0));
        w.hero.as_mut().unwrap().cannon_range = 50.0;
        w.hero.as_mut().unwrap().base_cannon_range = 50.0;
        w
    }

    fn item(id: i16, active: bool) -> ActionItem {
        ActionItem {
            id,
            amount: 10,
            active,
            range: 0.0,
            ready: true,
        }
    }

    fn war_engine(edit: impl FnOnce(&mut kraken_proto::WarConfig)) -> Engine {
        let mut engine = Engine::new(7);
        let mut cfg = engine.config().clone();
        cfg.humanize.enabled = false;
        cfg.mode = WorkMode::War;
        edit(&mut cfg.war);
        engine.configure(cfg);
        engine.control(0, Control::Start, &World::default());
        engine
    }

    /// Runs one step and mirrors `Select` into the world, as the game does.
    fn step(engine: &mut Engine, now: u64, w: &mut World) -> Vec<Action> {
        let actions = engine.step(now, w, &caps(), &OpenWater);
        for action in &actions {
            if let Action::Select(id) = action {
                w.hero.as_mut().unwrap().selected = Some(*id);
            }
        }
        actions
    }

    fn uses(actions: &[Action]) -> Vec<(i16, Option<Eid>)> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::UseItem { item, target } => Some((*item, *target)),
                _ => None,
            })
            .collect()
    }

    fn assert_no_helm(actions: &[Action]) {
        assert!(
            !actions.iter().any(|a| matches!(
                a,
                Action::Move(_) | Action::Repair(_) | Action::Respawn | Action::AbortAttack
            )),
            "{actions:?}"
        );
    }

    #[test]
    fn picks_nearest_enemy_player_and_fires() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "Far", 340.0, Relation::Neutral));
        w.actors.push(player(11, "Near", 330.0, Relation::Neutral));
        w.actors.push(npc(12, 5, 305.0, 300.0));
        let actions = step(&mut engine, 0, &mut w);
        assert_eq!(
            actions,
            vec![Action::Select(Eid::new(11, 1)), Action::AttackSelected]
        );
        assert_eq!(engine.activity(), Activity::War);
    }

    #[test]
    fn never_targets_friends() {
        let mut engine = war_engine(|cfg| cfg.friends.push("buddy".into()));
        let mut w = arena();
        w.actors.push(player(10, "Guildmate", 310.0, Relation::OwnGuild));
        w.actors.push(player(11, "Grouped", 311.0, Relation::Group));
        w.actors.push(player(12, "Peaceful", 312.0, Relation::Peace));
        w.actors.push(player(13, "Buddy", 313.0, Relation::Neutral));
        let actions = step(&mut engine, 0, &mut w);
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn relation_scope_filters_enemies() {
        let mut engine = war_engine(|cfg| {
            cfg.scope = EnemyScope::ByRelation;
            cfg.relations.neutral = false;
            cfg.guildless = false;
        });
        let mut w = arena();
        w.actors.push(player(10, "Neutral", 310.0, Relation::Neutral));
        w.actors.push(player(11, "Loner", 311.0, Relation::NoGuild));
        w.actors.push(player(12, "Foe", 340.0, Relation::War));
        let actions = step(&mut engine, 0, &mut w);
        assert_eq!(actions[0], Action::Select(Eid::new(12, 1)));
    }

    #[test]
    fn players_pick_locks_until_it_sinks_then_nearest() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "Near", 320.0, Relation::Neutral));
        w.actors.push(player(11, "Chosen", 345.0, Relation::Neutral));
        step(&mut engine, 0, &mut w);
        w.hero.as_mut().unwrap().selected = Some(Eid::new(11, 1));
        let actions = step(&mut engine, 100, &mut w);
        assert_eq!(actions, vec![Action::AttackSelected]);
        assert_eq!(engine.war_target(), Some(Eid::new(11, 1)));

        // Still locked although "Near" is closer.
        let actions = step(&mut engine, 400, &mut w);
        assert!(!actions.iter().any(|a| matches!(a, Action::Select(_))));

        engine.feedback(500, Feedback::Removed(Eid::new(11, 1)));
        w.actors.retain(|a| a.id != Eid::new(11, 1));
        let actions = step(&mut engine, 500, &mut w);
        assert_eq!(
            actions,
            vec![Action::Select(Eid::new(10, 1)), Action::AttackSelected]
        );
    }

    #[test]
    fn npc_pick_is_left_alone_until_an_enemy_shows_up() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(npc(20, 5, 310.0, 300.0));
        w.hero.as_mut().unwrap().selected = Some(Eid::new(20, 1));
        let actions = step(&mut engine, 0, &mut w);
        assert!(actions.is_empty(), "{actions:?}");

        w.actors.push(player(10, "Foe", 330.0, Relation::Neutral));
        let actions = step(&mut engine, 100, &mut w);
        assert_eq!(
            actions,
            vec![Action::Select(Eid::new(10, 1)), Action::AttackSelected]
        );
    }

    #[test]
    fn keeps_target_in_range_when_closer_one_arrives() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "First", 340.0, Relation::Neutral));
        step(&mut engine, 0, &mut w);
        w.actors.push(player(11, "Closer", 310.0, Relation::Neutral));
        let actions = step(&mut engine, 100, &mut w);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Select(_))),
            "{actions:?}"
        );
        assert_eq!(engine.war_target(), Some(Eid::new(10, 1)));
    }

    #[test]
    fn out_of_range_target_is_selected_but_not_fired_at() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "Distant", 380.0, Relation::Neutral));
        let actions = step(&mut engine, 0, &mut w);
        assert_eq!(actions, vec![Action::Select(Eid::new(10, 1))]);
    }

    #[test]
    fn refires_quickly_after_attack_ends() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "Foe", 330.0, Relation::Neutral));
        step(&mut engine, 0, &mut w);
        engine.feedback(10, Feedback::AttackStarted);
        assert!(step(&mut engine, 100, &mut w).is_empty());
        engine.feedback(150, Feedback::AttackEnded);
        assert_eq!(step(&mut engine, 200, &mut w), vec![Action::AttackSelected]);
    }

    #[test]
    fn distance_items_switch_on_and_off() {
        let mut engine = war_engine(|cfg| {
            cfg.items.push(WarItemRule::new(
                HAILSTORM,
                "Hailstorm",
                ItemWhen::TargetNear { max_pct: 50 },
            ));
            cfg.items.push(WarItemRule::new(
                WINDSTORM,
                "Windstorm",
                ItemWhen::TargetFar { min_pct: 100 },
            ));
        });
        let mut w = arena();
        w.items = vec![item(HAILSTORM, false), item(WINDSTORM, false)];
        // Far enemy: windstorm on.
        w.actors.push(player(10, "Foe", 370.0, Relation::Neutral));
        let actions = step(&mut engine, 0, &mut w);
        assert_eq!(uses(&actions), vec![(WINDSTORM, None)]);
        w.items[1].active = true;

        // Close in: hailstorm on, windstorm off only after the delay.
        w.actors[0].pos = Point::new(320.0, 300.0);
        let actions = step(&mut engine, 2_000, &mut w);
        assert_eq!(uses(&actions), vec![(HAILSTORM, None)]);
        w.items[0].active = true;
        assert!(uses(&step(&mut engine, 4_000, &mut w)).is_empty());
        let actions = step(&mut engine, 5_100, &mut w);
        assert_eq!(uses(&actions), vec![(WINDSTORM, None)]);
        assert_no_helm(&actions);
    }

    #[test]
    fn item_that_does_not_switch_off_is_not_retried() {
        let mut engine = war_engine(|cfg| {
            cfg.items.push(WarItemRule::new(
                HAILSTORM,
                "Hailstorm",
                ItemWhen::TargetNear { max_pct: 50 },
            ));
        });
        let mut w = arena();
        w.items = vec![item(HAILSTORM, true)];
        assert_eq!(uses(&step(&mut engine, 0, &mut w)), vec![]);
        assert_eq!(uses(&step(&mut engine, 3_100, &mut w)), vec![(HAILSTORM, None)]);
        assert!(uses(&step(&mut engine, 6_000, &mut w)).is_empty());
        assert!(uses(&step(&mut engine, 20_000, &mut w)).is_empty());
    }

    #[test]
    fn cooldown_or_empty_items_are_not_used() {
        let mut engine = war_engine(|cfg| {
            cfg.items
                .push(WarItemRule::new(LEGBA, "Legba", ItemWhen::VpBelow(50)));
        });
        let mut w = arena();
        let hero = w.hero.as_mut().unwrap();
        hero.vp = 10.0;
        hero.vp_max = 100.0;
        w.items = vec![ActionItem {
            ready: false,
            ..item(LEGBA, false)
        }];
        assert!(uses(&step(&mut engine, 0, &mut w)).is_empty());
        w.items[0].ready = true;
        assert_eq!(uses(&step(&mut engine, 100, &mut w)), vec![(LEGBA, None)]);
        // One-shot items wait out the gap.
        assert!(uses(&step(&mut engine, 500, &mut w)).is_empty());
        w.items[0].amount = 0;
        assert!(uses(&step(&mut engine, 2_000, &mut w)).is_empty());
    }

    #[test]
    fn support_items_aim_at_friends_only_when_enabled() {
        let rule = WarItemRule::new(BROTHERS, "Brothers", ItemWhen::FriendNear { max_pct: 100 });
        let mut w = arena();
        w.items = vec![item(BROTHERS, false)];
        w.actors.push(player(30, "Mate", 320.0, Relation::OwnGuild));

        let mut off = war_engine(|cfg| cfg.items.push(rule.clone()));
        assert!(uses(&step(&mut off, 0, &mut w)).is_empty());

        let mut on = war_engine(|cfg| {
            cfg.support = true;
            cfg.items.push(rule.clone());
        });
        assert_eq!(
            uses(&step(&mut on, 0, &mut w)),
            vec![(BROTHERS, Some(Eid::new(30, 1)))]
        );
    }

    #[test]
    fn dead_hero_does_nothing() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.hero.as_mut().unwrap().hp = 0.0;
        w.actors.push(player(10, "Foe", 330.0, Relation::Neutral));
        assert!(step(&mut engine, 0, &mut w).is_empty());
        assert_eq!(engine.activity(), Activity::Dead);
    }

    #[test]
    fn stop_in_war_mode_leaves_the_ship_alone() {
        let mut engine = war_engine(|_| {});
        let mut w = arena();
        w.actors.push(player(10, "Foe", 330.0, Relation::Neutral));
        step(&mut engine, 0, &mut w);
        engine.feedback(10, Feedback::AttackStarted);
        let actions = engine.control(100, Control::Stop, &w);
        assert!(actions.is_empty(), "{actions:?}");
    }

    const UNSTOPPABLE: i16 = 52;

    fn smoke(active: bool, millis: Option<u64>) -> Feedback {
        Feedback::Effect {
            name: "SMOKEBOMB".into(),
            target: HERO,
            active,
            millis,
        }
    }

    #[test]
    fn smoke_bomb_on_hero_fires_unstoppable_at_once() {
        let mut engine = war_engine(|cfg| {
            cfg.items.push(WarItemRule::new(
                UNSTOPPABLE,
                "Unstoppable",
                ItemWhen::Immobilized,
            ));
        });
        let mut w = arena();
        w.items = vec![item(UNSTOPPABLE, false)];
        w.actors.push(player(10, "Foe", 330.0, Relation::Neutral));
        assert!(uses(&step(&mut engine, 0, &mut w)).is_empty());

        // Someone else's smoke does not count.
        engine.feedback(
            50,
            Feedback::Effect {
                name: "SMOKEBOMB".into(),
                target: Eid::new(10, 1),
                active: true,
                millis: None,
            },
        );
        assert!(uses(&step(&mut engine, 60, &mut w)).is_empty());

        let hit = smoke(true, Some(5_000));
        assert!(engine.holds_hero(&hit));
        engine.feedback(100, hit);
        let actions = step(&mut engine, 100, &mut w);
        assert_eq!(uses(&actions), vec![(UNSTOPPABLE, None)]);
        assert_no_helm(&actions);

        // Once it is over nothing more is used.
        engine.feedback(200, smoke(false, None));
        assert!(uses(&step(&mut engine, 1_500, &mut w)).is_empty());
    }

    #[test]
    fn disabled_movement_flag_counts_as_held() {
        let mut engine = war_engine(|cfg| {
            cfg.items.push(WarItemRule::new(
                UNSTOPPABLE,
                "Unstoppable",
                ItemWhen::Immobilized,
            ));
        });
        let mut w = arena();
        w.items = vec![item(UNSTOPPABLE, false)];
        w.hero.as_mut().unwrap().movement_enabled = Some(false);
        // A short flicker is ignored.
        assert!(uses(&step(&mut engine, 0, &mut w)).is_empty());
        assert!(uses(&step(&mut engine, 200, &mut w)).is_empty());
        assert_eq!(uses(&step(&mut engine, 350, &mut w)), vec![(UNSTOPPABLE, None)]);
    }

    #[test]
    fn default_rules_are_added_once() {
        let mut cfg = kraken_proto::WarConfig::default();
        assert!(!cfg.add_defaults(std::iter::empty()));
        let owned = [
            (HAILSTORM, "Hail"),
            (WINDSTORM, "Wind"),
            (UNSTOPPABLE, "Unstop"),
            (LEGBA, "Legba"),
        ];
        assert!(cfg.add_defaults(owned));
        let ids: Vec<i16> = cfg.items.iter().map(|r| r.item).collect();
        assert_eq!(ids, vec![HAILSTORM, WINDSTORM, UNSTOPPABLE]);
        cfg.items.clear();
        assert!(!cfg.add_defaults(owned));
        assert!(cfg.items.is_empty());
    }
}

mod items {
    use kraken_proto::{Condition, ItemRule, Scene};

    use super::*;
    use crate::engine::world::ActionItem;

    const SWIFT: i16 = 27;
    const POTION: i16 = 90;

    fn item(id: i16) -> ActionItem {
        ActionItem {
            id,
            amount: 10,
            active: false,
            range: 0.0,
            ready: true,
        }
    }

    fn uses(actions: &[Action]) -> Vec<i16> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::UseItem { item, .. } => Some(*item),
                _ => None,
            })
            .collect()
    }

    fn with_rule(mode: WorkMode, rule: ItemRule) -> Engine {
        let mut engine = running(mode);
        let mut cfg = engine.config().clone();
        cfg.items.push(rule);
        engine.configure(cfg);
        engine
    }

    #[test]
    fn swiftstone_only_while_collecting() {
        let rule = ItemRule::new(SWIFT, "Swift", Scene::Collecting, Condition::None);
        let mut engine = with_rule(WorkMode::Collect, rule);
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(SWIFT)];
        // Nothing to collect: roaming, no use.
        assert!(uses(&engine.step(0, &w, &caps(), &OpenWater)).is_empty());
        w.actors.push(chest(20, 330.0, 0.0));
        let actions = engine.step(2_000, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Collecting);
        assert_eq!(uses(&actions), vec![SWIFT]);
    }

    #[test]
    fn repeats_every_n_seconds() {
        let rule = ItemRule {
            every_secs: 30,
            ..ItemRule::new(POTION, "Potion", Scene::Always, Condition::None)
        };
        let mut engine = with_rule(WorkMode::Collect, rule);
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(POTION)];
        assert_eq!(uses(&engine.step(0, &w, &caps(), &OpenWater)), vec![POTION]);
        assert!(uses(&engine.step(10_000, &w, &caps(), &OpenWater)).is_empty());
        assert!(uses(&engine.step(29_000, &w, &caps(), &OpenWater)).is_empty());
        assert_eq!(uses(&engine.step(30_000, &w, &caps(), &OpenWater)), vec![POTION]);
    }

    #[test]
    fn hp_threshold_needs_a_ready_stocked_item() {
        let rule = ItemRule::new(POTION, "Potion", Scene::Always, Condition::HpBelow(40));
        let mut engine = with_rule(WorkMode::Collect, rule);
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(POTION)];
        assert!(uses(&engine.step(0, &w, &caps(), &OpenWater)).is_empty());
        w.hero.as_mut().unwrap().hp = 300.0;
        w.items[0].ready = false;
        assert!(uses(&engine.step(1_000, &w, &caps(), &OpenWater)).is_empty());
        w.items[0].ready = true;
        w.items[0].amount = 0;
        assert!(uses(&engine.step(2_000, &w, &caps(), &OpenWater)).is_empty());
        w.items[0].amount = 3;
        assert_eq!(uses(&engine.step(3_000, &w, &caps(), &OpenWater)), vec![POTION]);
    }

    fn npc_rule(engine: &mut Engine, rule: ItemRule) {
        let mut cfg = engine.config().clone();
        cfg.npc.targets[0].items.push(rule);
        engine.configure(cfg);
    }

    #[test]
    fn npc_in_sight_rule_follows_that_type() {
        let mut engine = running(WorkMode::Collect);
        npc_rule(
            &mut engine,
            ItemRule::new(SWIFT, "Swift", Scene::InSight, Condition::None),
        );
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(SWIFT)];
        w.actors.push(npc(10, 6, 320.0, 0.0));
        assert!(uses(&engine.step(0, &w, &caps(), &OpenWater)).is_empty());
        w.actors.push(npc(11, 5, 340.0, 0.0));
        assert_eq!(uses(&engine.step(1_000, &w, &caps(), &OpenWater)), vec![SWIFT]);
    }

    #[test]
    fn npc_fighting_rule_only_while_engaged_with_it() {
        let mut engine = running(WorkMode::Combat);
        npc_rule(
            &mut engine,
            ItemRule::new(POTION, "Potion", Scene::FightingNpc, Condition::None),
        );
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(POTION)];
        assert!(uses(&engine.step(0, &w, &caps(), &OpenWater)).is_empty());
        w.actors.push(npc(10, 5, 310.0, 0.0));
        let actions = engine.step(1_000, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Npc);
        assert_eq!(uses(&actions), vec![POTION]);
    }

    #[test]
    fn toggle_item_goes_off_after_the_delay() {
        let rule = ItemRule {
            off_delay_ms: 3_000,
            ..ItemRule::new(SWIFT, "Swift", Scene::Collecting, Condition::None)
        };
        let mut engine = with_rule(WorkMode::Collect, rule);
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(SWIFT)];
        w.actors.push(chest(20, 330.0, 0.0));
        assert_eq!(uses(&engine.step(0, &w, &caps(), &OpenWater)), vec![SWIFT]);
        w.items[0].active = true;
        w.actors.clear();
        assert!(uses(&engine.step(1_000, &w, &caps(), &OpenWater)).is_empty());
        assert!(uses(&engine.step(3_500, &w, &caps(), &OpenWater)).is_empty());
        assert_eq!(uses(&engine.step(4_100, &w, &caps(), &OpenWater)), vec![SWIFT]);
    }

    #[test]
    fn nothing_while_stopped_or_dead() {
        let rule = ItemRule::new(POTION, "Potion", Scene::Always, Condition::None);
        let mut engine = with_rule(WorkMode::Collect, rule);
        let mut w = world(Point::new(300.0, 0.0));
        w.items = vec![item(POTION)];
        w.hero.as_mut().unwrap().hp = 0.0;
        assert!(uses(&engine.step(0, &w, &caps(), &OpenWater)).is_empty());
        w.hero.as_mut().unwrap().hp = 1000.0;
        engine.control(100, Control::Stop, &w);
        assert!(uses(&engine.step(200, &w, &caps(), &OpenWater)).is_empty());
    }
}

mod eldorado {
    use kraken_proto::{ELDORADO_MAP, ELDORADO_SKILL, ElDoradoPhase, EventCode};

    use super::*;
    use crate::engine::world::{ActionItem, MasterSkill};

    const SWIFT: i16 = 27;

    fn skill(charge: u8) -> MasterSkill {
        MasterSkill {
            id: ELDORADO_SKILL,
            charge,
            active: true,
            remaining_ms: 0,
            eldorado: true,
            ..MasterSkill::default()
        }
    }

    fn swift() -> ActionItem {
        ActionItem {
            id: SWIFT,
            amount: 5,
            active: false,
            range: 0.0,
            ready: true,
        }
    }

    fn enabled(mode: WorkMode) -> Engine {
        let mut engine = running(mode);
        let mut cfg = engine.config().clone();
        cfg.eldorado.enabled = true;
        engine.configure(cfg);
        engine
    }

    fn charged(hero: Point, charge: u8) -> World {
        let mut w = world(hero);
        w.skills = vec![skill(charge)];
        w
    }

    fn bonus(hero: Point) -> World {
        let mut w = charged(hero, 0);
        if let Some(map) = w.map.as_mut() {
            map.id = ELDORADO_MAP;
        }
        w
    }

    fn triggers(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, Action::TriggerSkill(ELDORADO_SKILL)))
            .count()
    }

    fn codes(engine: &mut Engine) -> Vec<EventCode> {
        engine.drain_events().into_iter().map(|e| e.code).collect()
    }

    fn phase(engine: &Engine, w: &World) -> ElDoradoPhase {
        engine.eldorado_info(0, w).expect("info").phase
    }

    fn set_hero(w: &mut World, pos: Point) {
        if let Some(hero) = w.hero.as_mut() {
            hero.pos = pos;
        }
    }

    #[test]
    fn waits_until_fully_charged() {
        let mut engine = enabled(WorkMode::Collect);
        let w = charged(Point::new(300.0, 100.0), 99);
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(triggers(&actions), 0);
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Waiting);
        assert_ne!(engine.activity(), Activity::ElDorado);
    }

    #[test]
    fn disabled_or_stopped_does_nothing() {
        let w = charged(Point::new(300.0, 270.0), 100);
        let mut off = running(WorkMode::Collect);
        assert_eq!(triggers(&off.step(0, &w, &caps(), &OpenWater)), 0);
        let mut stopped = enabled(WorkMode::Collect);
        stopped.control(0, Control::Stop, &w);
        assert_eq!(triggers(&stopped.step(0, &w, &caps(), &OpenWater)), 0);
    }

    #[test]
    fn retreats_to_the_nearest_corner_then_triggers_once() {
        let mut engine = enabled(WorkMode::Collect);
        // The nearest inset corner of the 600x600 diamond is (300, 276).
        let w = charged(Point::new(300.0, 100.0), 100);
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::ElDorado);
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Retreat);
        assert_eq!(triggers(&actions), 0);
        assert_eq!(moves(&actions), vec![Point::new(300.0, 140.0)]);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoReady));

        let mut w = charged(Point::new(300.0, 275.0), 100);
        let actions = engine.step(5_000, &w, &caps(), &OpenWater);
        assert_eq!(triggers(&actions), 1);
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Countdown);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoTriggered));
        engine.feedback(
            5_150,
            Feedback::SkillTriggered {
                id: ELDORADO_SKILL,
                remaining_ms: 64_000,
            },
        );

        // The countdown holds still: no second trigger, no fight, no chests.
        w.actors.push(npc(10, 5, 305.0, 270.0));
        w.actors.push(chest(20, 302.0, 274.0));
        for now in [5_500, 8_000, 14_000] {
            let actions = engine.step(now, &w, &caps(), &OpenWater);
            assert!(actions.is_empty(), "at {now}: {actions:?}");
        }
    }

    #[test]
    fn triggers_in_place_without_retreat() {
        let mut engine = enabled(WorkMode::Collect);
        let mut cfg = engine.config().clone();
        cfg.eldorado.retreat = false;
        engine.configure(cfg);
        let w = charged(Point::new(300.0, 100.0), 100);
        assert_eq!(triggers(&engine.step(0, &w, &caps(), &OpenWater)), 1);
    }

    #[test]
    fn an_unanswered_trigger_is_retried_once_then_waits_for_a_real_charge() {
        let mut engine = enabled(WorkMode::Collect);
        let w = charged(Point::new(300.0, 275.0), 100);
        assert_eq!(triggers(&engine.step(0, &w, &caps(), &OpenWater)), 1);
        assert_eq!(triggers(&engine.step(2_000, &w, &caps(), &OpenWater)), 0);
        assert_eq!(triggers(&engine.step(3_000, &w, &caps(), &OpenWater)), 1);
        codes(&mut engine);
        assert_eq!(triggers(&engine.step(6_000, &w, &caps(), &OpenWater)), 0);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoFailed));
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Waiting);
        // The 100% shown is stale: nothing until the charge is seen lower.
        assert_eq!(triggers(&engine.step(60_000, &w, &caps(), &OpenWater)), 0);
        engine.step(
            61_000,
            &charged(Point::new(300.0, 275.0), 40),
            &caps(),
            &OpenWater,
        );
        assert_eq!(triggers(&engine.step(62_000, &w, &caps(), &OpenWater)), 1);
    }

    #[test]
    fn an_accepted_trigger_waits_for_the_jump() {
        let mut engine = enabled(WorkMode::Collect);
        let w = charged(Point::new(300.0, 275.0), 100);
        assert_eq!(triggers(&engine.step(0, &w, &caps(), &OpenWater)), 1);
        engine.feedback(
            150,
            Feedback::SkillTriggered {
                id: ELDORADO_SKILL,
                remaining_ms: 64_000,
            },
        );
        for now in [3_000, 9_000, 19_000] {
            assert_eq!(triggers(&engine.step(now, &w, &caps(), &OpenWater)), 0);
        }
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Countdown);
        engine.step(20_000, &w, &caps(), &OpenWater);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoFailed));
    }

    #[test]
    fn finishes_the_current_fight_without_taking_new_targets() {
        let mut engine = enabled(WorkMode::Combat);
        let mut w = charged(Point::new(300.0, 0.0), 0);
        w.actors.push(npc(10, 5, 310.0, 0.0));
        engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Npc);

        w.skills = vec![skill(100)];
        w.actors.push(npc(11, 5, 312.0, 0.0));
        engine.step(1_000, &w, &caps(), &OpenWater);
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Finishing);
        assert_eq!(engine.activity(), Activity::Npc);
        assert_eq!(engine.target_id(), Some(Eid::new(10, 1)));

        // The target sinks: the other NPC is not taken, the retreat starts.
        engine.feedback(1_500, Feedback::Removed(Eid::new(10, 1)));
        w.actors.retain(|a| a.id != Eid::new(10, 1));
        engine.step(2_000, &w, &caps(), &OpenWater);
        assert_eq!(phase(&engine, &w), ElDoradoPhase::Retreat);
        assert_eq!(engine.activity(), Activity::ElDorado);
    }

    /// A treasure chest (box type 5) or glitter (type 1) of the bonus map.
    fn bonus_box(id: i64, type_id: i32, x: f32, y: f32) -> Actor {
        Actor {
            type_id,
            ..chest(id, x, y)
        }
    }

    #[test]
    fn collects_nearest_chests_only_and_waits_on_each() {
        let mut engine = enabled(WorkMode::Combat);
        let mut w = bonus(Point::new(300.0, 0.0));
        w.actors.push(bonus_box(20, 5, 320.0, 0.0));
        w.actors.push(bonus_box(21, 5, 340.0, 0.0));
        w.actors.push(bonus_box(22, 5, 250.0, 0.0));
        // Glitter is closer, but only chests count.
        w.actors.push(bonus_box(23, 1, 305.0, 0.0));
        // No fighting on the bonus map.
        w.actors.push(npc(10, 5, 305.0, 0.0));
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoEntered));
        assert_eq!(engine.activity(), Activity::Collecting);
        assert_eq!(moves(&actions), vec![Point::new(320.0, 0.0)]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Cannon { .. })));

        // On chest 20: holds course until the game takes it.
        set_hero(&mut w, Point::new(319.5, 0.0));
        assert!(moves(&engine.step(600, &w, &caps(), &OpenWater)).is_empty());
        assert!(moves(&engine.step(1_200, &w, &caps(), &OpenWater)).is_empty());

        // Taken: counted, and on to 21 in the same step.
        engine.feedback(1_300, Feedback::Removed(Eid::new(20, 1)));
        w.actors.retain(|a| a.id != Eid::new(20, 1));
        let actions = engine.step(1_300, &w, &caps(), &OpenWater);
        assert_eq!(moves(&actions), vec![Point::new(340.0, 0.0)]);
        assert_eq!(engine.eldorado_info(1_300, &w).unwrap().chests, 1);
        assert!(codes(&mut engine).contains(&EventCode::BoxCollected));
    }

    #[test]
    fn keeps_its_chest_while_others_come_closer() {
        let mut engine = enabled(WorkMode::Collect);
        let mut w = bonus(Point::new(300.0, 0.0));
        w.actors.push(bonus_box(20, 5, 330.0, 0.0));
        assert_eq!(
            moves(&engine.step(0, &w, &caps(), &OpenWater)),
            vec![Point::new(330.0, 0.0)]
        );
        set_hero(&mut w, Point::new(310.0, 0.0));
        w.actors.push(bonus_box(21, 5, 305.0, 0.0));
        let actions = engine.step(2_500, &w, &caps(), &OpenWater);
        assert!(!moves(&actions).contains(&Point::new(305.0, 0.0)));
        assert_eq!(engine.target_id(), Some(Eid::new(20, 1)));
    }

    #[test]
    fn a_chest_that_stays_is_skipped_after_a_short_wait() {
        let mut engine = enabled(WorkMode::Collect);
        let mut w = bonus(Point::new(300.0, 0.0));
        w.actors.push(bonus_box(20, 5, 300.0, 1.0));
        w.actors.push(bonus_box(21, 5, 350.0, 0.0));
        engine.step(0, &w, &caps(), &OpenWater);
        assert!(moves(&engine.step(1_000, &w, &caps(), &OpenWater)).is_empty());
        // No key: given up after the wait, next chest.
        let later = engine.step(2_600, &w, &caps(), &OpenWater);
        assert!(codes(&mut engine).contains(&EventCode::BoxSkipped));
        assert_eq!(moves(&later), vec![Point::new(340.0, 0.0)]);
    }

    #[test]
    fn swiftstone_on_the_bonus_map_only() {
        let mut engine = enabled(WorkMode::Collect);
        let mut w = bonus(Point::new(300.0, 0.0));
        w.items = vec![swift()];
        w.actors.push(bonus_box(20, 5, 330.0, 0.0));
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert!(actions.contains(&Action::UseItem {
            item: SWIFT,
            target: None
        }));

        let mut normal = enabled(WorkMode::Collect);
        let mut w = charged(Point::new(300.0, 0.0), 0);
        w.items = vec![swift()];
        w.actors.push(chest(20, 330.0, 0.0));
        let actions = normal.step(0, &w, &caps(), &OpenWater);
        assert!(!actions.contains(&Action::UseItem {
            item: SWIFT,
            target: None
        }));
    }

    #[test]
    fn a_stale_full_charge_after_a_run_is_not_used() {
        let mut engine = enabled(WorkMode::Collect);
        let w = bonus(Point::new(300.0, 0.0));
        engine.step(0, &w, &caps(), &OpenWater);
        // Back on the normal map; the game still shows 100%.
        let stale = charged(Point::new(300.0, 275.0), 100);
        assert_eq!(triggers(&engine.step(1_000, &stale, &caps(), &OpenWater)), 0);
        assert_eq!(triggers(&engine.step(60_000, &stale, &caps(), &OpenWater)), 0);
        // The reset shows up, then a real full charge later.
        engine.step(61_000, &charged(Point::new(300.0, 275.0), 0), &caps(), &OpenWater);
        assert_eq!(triggers(&engine.step(62_000, &stale, &caps(), &OpenWater)), 1);
    }

    #[test]
    fn leaving_the_bonus_map_resumes_normal_work() {
        let mut engine = enabled(WorkMode::Collect);
        let mut w = bonus(Point::new(300.0, 0.0));
        w.actors.push(chest(20, 330.0, 0.0));
        engine.step(0, &w, &caps(), &OpenWater);
        codes(&mut engine);
        let mut back = charged(Point::new(300.0, 0.0), 0);
        back.actors.push(chest(30, 330.0, 0.0));
        engine.step(1_000, &back, &caps(), &OpenWater);
        assert!(codes(&mut engine).contains(&EventCode::ElDoradoLeft));
        assert_eq!(phase(&engine, &back), ElDoradoPhase::Waiting);
        assert_eq!(engine.eldorado_info(1_000, &back).unwrap().runs, 1);
        assert_eq!(engine.activity(), Activity::Collecting);
    }
}

mod bonus {
    use kraken_proto::{BonusEntry, BonusPhase, ELDORADO_SKILL, EventCode};

    use super::*;
    use crate::engine::world::{BonusMap, MasterSkill};

    const VIRGO: i16 = 106;
    const CAPRICORN: i16 = 107;

    fn queued(entries: &[(i16, Option<u16>)]) -> Engine {
        let mut engine = running(WorkMode::Combat);
        let mut cfg = engine.config().clone();
        cfg.bonus.enabled = true;
        cfg.bonus.ammo = Some(3);
        cfg.bonus.queue = entries
            .iter()
            .map(|(map, count)| BonusEntry {
                map: *map,
                name: format!("Map{map}"),
                count: *count,
            })
            .collect();
        engine.configure(cfg);
        engine
    }

    fn owning(hero: Point, maps: &[(i16, i64)]) -> World {
        let mut w = world(hero);
        w.bonus = maps
            .iter()
            .map(|(id, amount)| BonusMap {
                id: *id,
                amount: *amount,
                wave: 0,
                waves: 10,
            })
            .collect();
        w
    }

    fn on_map(mut w: World, id: i16) -> World {
        if let Some(map) = w.map.as_mut() {
            map.id = i32::from(id);
            map.kind_name = "WAVE".into();
        }
        w
    }

    fn enters(actions: &[Action]) -> Vec<i16> {
        actions
            .iter()
            .filter_map(|a| {
                if let Action::EnterMap(m) = a {
                    Some(*m)
                } else {
                    None
                }
            })
            .collect()
    }

    fn shots(actions: &[Action]) -> Vec<i64> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Cannon { target, .. } => Some(target.id),
                _ => None,
            })
            .collect()
    }

    fn codes(engine: &mut Engine) -> Vec<EventCode> {
        engine.drain_events().into_iter().map(|e| e.code).collect()
    }

    fn phase(engine: &Engine, w: &World) -> BonusPhase {
        engine.bonus_info(0, w).expect("info").phase
    }

    #[test]
    fn retreats_then_requests_the_map_once() {
        let mut engine = queued(&[(VIRGO, Some(1))]);
        let w = owning(Point::new(300.0, 100.0), &[(VIRGO, 3)]);
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Bonus);
        assert_eq!(phase(&engine, &w), BonusPhase::Retreat);
        assert_eq!(moves(&actions), vec![Point::new(300.0, 140.0)]);
        assert!(codes(&mut engine).contains(&EventCode::BonusEntering));

        let w = owning(Point::new(300.0, 275.0), &[(VIRGO, 3)]);
        assert_eq!(enters(&engine.step(5_000, &w, &caps(), &OpenWater)), vec![VIRGO]);
        assert_eq!(phase(&engine, &w), BonusPhase::Countdown);
        // The delay arrives: hold still, no second request.
        engine.feedback(
            5_100,
            Feedback::MapDelay {
                map: VIRGO,
                delay_ms: 10_000,
            },
        );
        for now in [6_000, 9_000, 14_000] {
            let actions = engine.step(now, &w, &caps(), &OpenWater);
            assert!(actions.is_empty(), "at {now}: {actions:?}");
        }
    }

    #[test]
    fn an_unanswered_request_is_retried_once_then_skipped() {
        let mut engine = queued(&[(VIRGO, None), (CAPRICORN, None)]);
        let mut cfg = engine.config().clone();
        cfg.bonus.retreat = false;
        engine.configure(cfg);
        let w = owning(Point::new(300.0, 0.0), &[(VIRGO, 1), (CAPRICORN, 1)]);
        assert_eq!(enters(&engine.step(0, &w, &caps(), &OpenWater)), vec![VIRGO]);
        assert_eq!(enters(&engine.step(3_000, &w, &caps(), &OpenWater)), vec![VIRGO]);
        codes(&mut engine);
        let actions = engine.step(6_000, &w, &caps(), &OpenWater);
        assert!(codes(&mut engine).contains(&EventCode::BonusFailed));
        // Virgo is left alone for a while; Capricorn is next.
        let actions = [actions, engine.step(6_100, &w, &caps(), &OpenWater)].concat();
        assert_eq!(enters(&actions), vec![CAPRICORN]);
    }

    #[test]
    fn plays_the_queue_then_goes_back_to_npcs() {
        let mut engine = queued(&[(VIRGO, Some(2))]);
        let mut cfg = engine.config().clone();
        cfg.bonus.retreat = false;
        engine.configure(cfg);
        let home = owning(Point::new(300.0, 0.0), &[(VIRGO, 5)]);
        let mut seen = Vec::new();
        let mut requests = 0;
        for round in 0..2u64 {
            let t = round * 50_000;
            requests += enters(&engine.step(t, &home, &caps(), &OpenWater)).len();
            let inside = on_map(home.clone(), VIRGO);
            engine.feedback(t + 1_000, Feedback::MapChanged);
            engine.step(t + 1_000, &inside, &caps(), &OpenWater);
            engine.feedback(t + 50_000, Feedback::MapChanged);
            requests += enters(&engine.step(t + 50_000, &home, &caps(), &OpenWater)).len();
            seen.extend(codes(&mut engine));
        }
        assert_eq!(requests, 2);
        let count = |code| seen.iter().filter(|c| **c == code).count();
        assert_eq!(count(EventCode::BonusEntered), 2);
        assert_eq!(count(EventCode::BonusFinished), 2);
        assert_eq!(count(EventCode::BonusQueueDone), 1);

        let mut w = home.clone();
        w.actors.push(npc(10, 5, 330.0, 0.0));
        let actions = engine.step(300_000, &w, &caps(), &OpenWater);
        assert!(enters(&actions).is_empty());
        assert_eq!(engine.activity(), Activity::Npc);
        assert_eq!(engine.bonus_info(0, &w).unwrap().done, vec![(VIRGO, 2)]);
    }

    #[test]
    fn maps_not_owned_are_skipped() {
        let mut engine = queued(&[(VIRGO, None), (CAPRICORN, Some(1))]);
        let mut cfg = engine.config().clone();
        cfg.bonus.retreat = false;
        engine.configure(cfg);
        let w = owning(Point::new(300.0, 0.0), &[(VIRGO, 0), (CAPRICORN, 2)]);
        assert_eq!(enters(&engine.step(0, &w, &caps(), &OpenWater)), vec![CAPRICORN]);
    }

    #[test]
    fn bonus_maps_go_before_el_dorado() {
        let mut engine = queued(&[(VIRGO, Some(1))]);
        let mut cfg = engine.config().clone();
        cfg.bonus.retreat = false;
        cfg.eldorado.enabled = true;
        cfg.eldorado.retreat = false;
        engine.configure(cfg);
        let mut w = owning(Point::new(300.0, 0.0), &[(VIRGO, 1)]);
        w.skills = vec![MasterSkill {
            id: ELDORADO_SKILL,
            charge: 100,
            active: true,
            remaining_ms: 0,
            eldorado: true,
            ..MasterSkill::default()
        }];
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(enters(&actions), vec![VIRGO]);
        assert!(!actions.contains(&Action::TriggerSkill(ELDORADO_SKILL)));
    }

    #[test]
    fn rotates_fire_over_every_npc_in_range_without_moving() {
        let mut engine = queued(&[(VIRGO, None)]);
        let mut w = on_map(owning(Point::new(300.0, 0.0), &[(VIRGO, 1)]), VIRGO);
        w.actors.push(npc(10, 900, 305.0, 0.0));
        w.actors.push(npc(11, 901, 308.0, 0.0));
        w.actors.push(npc(12, 902, 310.0, 0.0));
        let first = engine.step(0, &w, &caps(), &OpenWater);
        assert_eq!(shots(&first), vec![10]);
        assert!(moves(&first).is_empty());
        // Our hit lands: next target at once.
        engine.feedback(
            400,
            Feedback::Hit {
                source: HERO,
                target: Eid::new(10, 1),
            },
        );
        assert_eq!(shots(&engine.step(400, &w, &caps(), &OpenWater)), vec![11]);
        // No hit for a while: rotate anyway.
        assert!(shots(&engine.step(1_000, &w, &caps(), &OpenWater)).is_empty());
        assert_eq!(shots(&engine.step(1_600, &w, &caps(), &OpenWater)), vec![12]);
        engine.feedback(
            1_900,
            Feedback::Hit {
                source: HERO,
                target: Eid::new(12, 1),
            },
        );
        let actions = engine.step(2_000, &w, &caps(), &OpenWater);
        assert_eq!(shots(&actions), vec![10]);
        assert!(moves(&actions).is_empty());
    }

    #[test]
    fn approaches_far_waves_and_waits_for_the_next_one() {
        let mut engine = queued(&[(VIRGO, None)]);
        let mut w = on_map(owning(Point::new(300.0, 0.0), &[(VIRGO, 1)]), VIRGO);
        w.actors.push(npc(10, 900, 360.0, 0.0));
        let actions = engine.step(0, &w, &caps(), &OpenWater);
        assert!(shots(&actions).is_empty());
        assert_eq!(moves(&actions).len(), 1);

        w.actors.clear();
        assert!(moves(&engine.step(1_000, &w, &caps(), &OpenWater)).is_empty());
        assert!(moves(&engine.step(4_000, &w, &caps(), &OpenWater)).is_empty());
        let w = on_map(owning(Point::new(400.0, 100.0), &[(VIRGO, 1)]), VIRGO);
        assert_eq!(moves(&engine.step(6_000, &w, &caps(), &OpenWater)).len(), 1);
    }
}

mod workmap {
    use kraken_proto::{Activity, Control, Point, WorkMode, WorkmapEntry, WorkmapPhase};

    use super::{HERO, caps, running, world};
    use crate::engine::Engine;
    use crate::engine::world::{Action, Feedback, MapCell, OpenWater, World};

    fn cell(id: i16, row: u8, col: u8) -> MapCell {
        MapCell {
            id,
            world: 0,
            row,
            col,
            safe_haven: false,
            min_level: 0,
            label: id.to_string(),
        }
    }

    fn rotating(maps: &[i16]) -> Engine {
        let mut engine = running(WorkMode::Combat);
        let mut cfg = engine.config().clone();
        cfg.workmap.enabled = true;
        cfg.workmap.minutes = 1;
        cfg.workmap.maps = maps
            .iter()
            .map(|m| WorkmapEntry {
                map: *m,
                name: m.to_string(),
            })
            .collect();
        engine.configure(cfg);
        engine.control(0, Control::Start, &World::default());
        engine
    }

    fn atlas_world() -> World {
        let mut w = world(Point::new(300.0, 0.0));
        w.atlas = vec![cell(33, 0, 0), cell(34, 0, 1), cell(35, 1, 1)];
        w
    }

    /// Works the minute out on map 33 (until the rotation moves on);
    /// returns the time reached and that step's actions.
    fn work_out(engine: &mut Engine, w: &World) -> (u64, Vec<Action>) {
        let mut now = 1_000;
        loop {
            let actions = engine.step(now, w, &caps(), &OpenWater);
            if engine.activity() == Activity::Travelling {
                return (now, actions);
            }
            assert!(now < 70_000, "never left");
            now += 1_000;
        }
    }

    #[test]
    fn stays_while_the_time_runs_then_teleports() {
        let mut engine = rotating(&[33, 34]);
        let mut w = atlas_world();
        w.teleports = 2;
        w.teleport_ready = true;
        engine.step(500, &w, &caps(), &OpenWater);
        assert_eq!(engine.workmap_info(&w).unwrap().phase, WorkmapPhase::Working);
        let (now, actions) = work_out(&mut engine, &w);
        assert!(now >= 60_000);
        assert!(actions.contains(&Action::Teleport(34)), "{actions:?}");
        assert_eq!(engine.activity(), Activity::Travelling);
        // Held still while the jump is on its way.
        let actions = engine.step(now + 1_000, &w, &caps(), &OpenWater);
        assert!(!actions.contains(&Action::Teleport(34)));
    }

    #[test]
    fn without_teleports_sails_and_takes_only_the_right_offer() {
        let mut engine = rotating(&[33, 34]);
        let w = atlas_world();
        let (now, _) = work_out(&mut engine, &w);
        assert_eq!(engine.workmap_info(&w).unwrap().phase, WorkmapPhase::Sailing);
        assert_eq!(engine.workmap_info(&w).unwrap().next_hop, Some(34));
        // An offer for some other map is not taken.
        engine.feedback(
            now + 100,
            Feedback::MapAsk {
                map: 35,
                confirm: false,
            },
        );
        let actions = engine.step(now + 600, &w, &caps(), &OpenWater);
        assert!(!actions.iter().any(|a| matches!(a, Action::EdgeJump(_))));
        // The right one is.
        engine.feedback(
            now + 700,
            Feedback::MapAsk {
                map: 34,
                confirm: false,
            },
        );
        let actions = engine.step(now + 1_200, &w, &caps(), &OpenWater);
        assert!(actions.contains(&Action::EdgeJump(34)), "{actions:?}");
        assert_eq!(engine.workmap_info(&w).unwrap().phase, WorkmapPhase::Crossing);
    }

    #[test]
    fn a_confirm_offer_is_confirmed() {
        let mut engine = rotating(&[33, 34]);
        let w = atlas_world();
        let (now, _) = work_out(&mut engine, &w);
        engine.feedback(
            now + 100,
            Feedback::MapAsk {
                map: 34,
                confirm: true,
            },
        );
        let actions = engine.step(now + 600, &w, &caps(), &OpenWater);
        assert!(actions.contains(&Action::ConfirmJump), "{actions:?}");
    }

    #[test]
    fn starts_travelling_at_once_off_the_list() {
        let mut engine = rotating(&[34]);
        let w = atlas_world();
        engine.step(500, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Travelling);
    }

    #[test]
    fn repair_comes_before_travelling() {
        let mut engine = rotating(&[34]);
        let mut w = atlas_world();
        w.hero.as_mut().unwrap().hp = 200.0;
        engine.step(500, &w, &caps(), &OpenWater);
        assert_eq!(engine.activity(), Activity::Repairing);
        let _ = HERO;
    }
}

mod torpedo {
    use kraken_proto::{Point, Stock, StockKind, WorkMode};

    use super::{caps, npc, running, world};
    use crate::engine::world::{Action, OpenWater, TorpedoState, World};

    fn undersea(slots: &[(i16, i16)], cooling: bool) -> World {
        let mut w = world(Point::new(300.0, 0.0));
        w.map.as_mut().unwrap().atlantis = true;
        w.stock.push(Stock {
            kind: StockKind::Torpedo,
            id: 7,
            amount: 20,
        });
        w.torpedo = TorpedoState {
            preset: Some(240),
            slots: slots.to_vec(),
            cooling,
        };
        w.actors.push(npc(10, 5, 305.0, 0.0));
        w
    }

    fn fire(engine: &mut crate::engine::Engine, w: &World) -> Vec<Action> {
        let mut all = Vec::new();
        for at in [1_000, 2_000, 3_000] {
            all.extend(engine.step(at, w, &caps(), &OpenWater));
        }
        all
    }

    #[test]
    fn fires_torpedoes_not_cannonballs_undersea() {
        let mut engine = running(WorkMode::Combat);
        let actions = fire(&mut engine, &undersea(&[(1, 3), (2, 3)], false));
        assert!(
            actions.iter().any(|a| matches!(a, Action::Torpedo { .. })),
            "{actions:?}"
        );
        assert!(!actions.iter().any(|a| matches!(a, Action::Cannon { .. })));
    }

    #[test]
    fn loads_the_chosen_torpedo_into_other_slots() {
        let mut engine = running(WorkMode::Combat);
        let mut cfg = engine.config().clone();
        cfg.npc.targets[0].torpedo = Some(7);
        engine.configure(cfg);
        let actions = fire(&mut engine, &undersea(&[(1, 3), (2, 7)], false));
        assert!(actions.contains(&Action::LoadTorpedo {
            preset: 240,
            slot: 1,
            torpedo: 7
        }));
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::LoadTorpedo { slot: 2, .. }))
        );
        assert!(actions.iter().any(|a| matches!(a, Action::Torpedo { .. })));
    }

    #[test]
    fn waits_for_the_load_and_the_launchers() {
        let mut engine = running(WorkMode::Combat);
        let mut cfg = engine.config().clone();
        cfg.npc.default_torpedo = Some(7);
        engine.configure(cfg);
        // Nothing holds torpedo 7 yet: load, but no shot.
        let actions = fire(&mut engine, &undersea(&[(1, 3)], false));
        assert!(actions.iter().any(|a| matches!(a, Action::LoadTorpedo { .. })));
        assert!(!actions.iter().any(|a| matches!(a, Action::Torpedo { .. })));
        // Loaded but every launcher cooling: still no shot.
        let mut engine = running(WorkMode::Combat);
        let actions = fire(&mut engine, &undersea(&[(1, 3)], true));
        assert!(!actions.iter().any(|a| matches!(a, Action::Torpedo { .. })));
    }
}

use kraken_proto::{CollectorConfig, EntityKind, MonsterConfig, NpcConfig, RangeStyle, StockKind};

use super::world::{Actor, Eid, World};

/// Unlisted targets rank below every listed one (the UI allows 1–10).
pub const LOWEST_PRIORITY: u8 = 11;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choice {
    pub id: Eid,
    pub kind: EntityKind,
    pub priority: u8,
    pub distance: f32,
    pub weapon: i16,
    pub style: RangeStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    NoWeapon,
    NoStock(StockKind, i16),
}

fn richest(world: &World, kind: StockKind) -> Option<i16> {
    world
        .stock
        .iter()
        .filter(|s| s.kind == kind && s.amount > 0)
        .max_by_key(|s| s.amount)
        .map(|s| s.id)
}

fn armed(world: &World, kind: StockKind, weapon: Option<i16>) -> Result<i16, Rejection> {
    let weapon = weapon
        .or_else(|| richest(world, kind))
        .ok_or(Rejection::NoWeapon)?;
    match world.stock(kind, weapon) {
        Some(amount) if amount <= 0 => Err(Rejection::NoStock(kind, weapon)),
        _ => Ok(weapon),
    }
}

pub fn npc(
    cfg: &NpcConfig,
    world: &World,
    actor: &Actor,
) -> Option<Result<(u8, i16, RangeStyle), Rejection>> {
    if !cfg.enabled || actor.kind != EntityKind::Npc {
        return None;
    }
    let listed = cfg.targets.iter().find(|t| t.type_id == actor.type_id);
    let (priority, ammo, style) = match listed {
        Some(t) => (t.priority, t.ammo.or(cfg.default_ammo), t.range),
        None if cfg.attack_all => (LOWEST_PRIORITY, cfg.default_ammo, RangeStyle::Default),
        None => return None,
    };
    Some(armed(world, StockKind::Ammo, ammo).map(|w| (priority, w, style)))
}

pub fn monster(cfg: &MonsterConfig, world: &World, actor: &Actor) -> Option<Result<(u8, i16), Rejection>> {
    if !cfg.enabled || actor.kind != EntityKind::Monster {
        return None;
    }
    let listed = cfg.targets.iter().find(|t| t.type_id == actor.type_id);
    let (priority, harpoon) = match listed {
        Some(t) => (t.priority, t.harpoon.or(cfg.default_harpoon)),
        None if cfg.attack_all => (LOWEST_PRIORITY, cfg.default_harpoon),
        None => return None,
    };
    Some(armed(world, StockKind::Harpoon, harpoon).map(|w| (priority, w)))
}

pub fn boxed(cfg: &CollectorConfig, actor: &Actor) -> Option<u8> {
    if !cfg.enabled || actor.kind != EntityKind::Box {
        return None;
    }
    match cfg.boxes.iter().find(|b| b.type_id == actor.type_id) {
        Some(b) => Some(b.priority),
        None => cfg.collect_all.then_some(LOWEST_PRIORITY),
    }
}

pub fn rank(choices: impl Iterator<Item = Choice>) -> Option<Choice> {
    choices.min_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.distance.total_cmp(&b.distance))
    })
}

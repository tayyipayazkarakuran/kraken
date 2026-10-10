//! Action-item automation shared by every mode: the Items tab's rules, the
//! rules attached to NPC and monster targets, and (through `ItemRunner`) the
//! war bot's own rules.

use std::collections::HashMap;

use kraken_proto::{
    Activity, Capabilities, Condition, EntityKind, EventCode, ITEM_SWIFTSTONE, ItemRule, Level, RunState,
    Scene,
};

use super::world::{Action, ActionItem, Eid, World};
use super::{Engine, SIGHT_FACTOR, SIGHT_MIN};

/// The NPC or monster type a target's own rule belongs to.
type Owner = (EntityKind, i32);

/// Smallest gap between two uses of the same rule's item.
pub(super) const USE_GAP_MS: u64 = 1_000;
/// How long a use may take to show up as `isActive` before it counts as failed.
const SETTLE_MS: u64 = 2_500;
/// Item uses per step, so a burst of rules does not flood the game.
const USES_PER_STEP: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
struct RuleState {
    false_since: Option<u64>,
    last_use: Option<u64>,
    /// A switch-off attempt that left the item active; do not retry.
    no_off: bool,
    /// An "off" use was sent and has not shown up yet.
    off_pending: Option<u64>,
    /// This rule switched the item on (only those are switched off again).
    ours: bool,
}

/// What one rule wants right now.
pub(super) struct Want<'a> {
    pub key: String,
    pub item: &'a ActionItem,
    pub wanted: bool,
    pub aim: Option<Eid>,
    /// The item may be switched off again once the rule no longer wants it.
    pub can_off: bool,
    /// Only switch off what this rule switched on.
    pub only_ours: bool,
    pub off_delay_ms: u16,
    /// Extra gap between uses (one-shot repeats).
    pub every_ms: u64,
}

/// Per-rule state: when an item was last used, switched off, or failed to.
#[derive(Debug, Default)]
pub(super) struct ItemRunner {
    states: HashMap<String, RuleState>,
}

impl ItemRunner {
    pub fn clear(&mut self) {
        self.states.clear();
    }

    /// Returns the action to send and whether it switches the item off.
    pub fn decide(&mut self, now: u64, want: Want<'_>) -> Option<(Action, bool)> {
        let item = want.item;
        let st = self.states.entry(want.key).or_default();
        if st.off_pending.is_some_and(|t| now.saturating_sub(t) >= SETTLE_MS) {
            st.off_pending = None;
            if item.active {
                st.no_off = true;
            }
        }
        let since_use = |gap: u64| st.last_use.is_none_or(|t| now.saturating_sub(t) >= gap);
        if want.wanted {
            st.false_since = None;
            if !item.active && item.ready && item.amount > 0 && since_use(USE_GAP_MS.max(want.every_ms)) {
                st.last_use = Some(now);
                st.ours = true;
                return Some((
                    Action::UseItem {
                        item: item.id,
                        target: want.aim,
                    },
                    false,
                ));
            }
            return None;
        }
        let since = *st.false_since.get_or_insert(now);
        let settled = now.saturating_sub(since) >= u64::from(want.off_delay_ms);
        if !item.active {
            st.ours = false;
            return None;
        }
        if settled
            && since_use(USE_GAP_MS)
            && want.can_off
            && (!want.only_ours || st.ours)
            && !st.no_off
            && st.off_pending.is_none()
        {
            st.last_use = Some(now);
            st.off_pending = Some(now);
            st.ours = false;
            return Some((
                Action::UseItem {
                    item: item.id,
                    target: None,
                },
                true,
            ));
        }
        None
    }
}

/// War mode, and the special maps the hero may be on.
#[derive(Clone, Copy)]
struct Modes {
    war: bool,
    eldorado: bool,
    bonus: bool,
}

/// Whether a rule's scene holds. `owner` is the NPC or monster type a
/// target's own rule belongs to.
fn scene_holds(
    scene: Scene,
    owner: Option<(EntityKind, i32)>,
    activity: Activity,
    target: Option<(EntityKind, i32)>,
    in_sight: &[(EntityKind, i32)],
    modes: Modes,
) -> bool {
    let fighting = |kind: EntityKind, busy: Activity| {
        activity == busy && target.is_some_and(|(k, t)| k == kind && owner.is_none_or(|(_, o)| o == t))
    };
    match scene {
        Scene::Always => true,
        Scene::Collecting => activity == Activity::Collecting,
        Scene::Roaming => activity == Activity::Roaming,
        Scene::FightingNpc => fighting(EntityKind::Npc, Activity::Npc),
        Scene::FightingMonster => fighting(EntityKind::Monster, Activity::Monster),
        Scene::InSight => match owner {
            Some(owner) => in_sight.contains(&owner),
            None => !in_sight.is_empty(),
        },
        Scene::War => modes.war,
        Scene::ElDorado => modes.eldorado,
        Scene::BonusMap => modes.bonus,
    }
}

impl Engine {
    /// The general and per-target item rules; runs after every step.
    pub(super) fn item_rules(&mut self, now: u64, world: &World, caps: &Capabilities, out: &mut Vec<Action>) {
        if !caps.use_item || self.run != RunState::Running || self.death.is_some() {
            return;
        }
        let Some(hero) = world.hero.as_ref() else { return };
        if hero.dead() {
            return;
        }
        let mut rules: Vec<(String, &ItemRule, Option<Owner>)> = Vec::new();
        for (index, rule) in self.cfg.items.iter().enumerate() {
            rules.push((format!("g{index}"), rule, None));
        }
        for target in &self.cfg.npc.targets {
            for (index, rule) in target.items.iter().enumerate() {
                let owner = (EntityKind::Npc, target.type_id);
                rules.push((format!("n{}:{index}", target.type_id), rule, Some(owner)));
            }
        }
        for target in &self.cfg.monster.targets {
            for (index, rule) in target.items.iter().enumerate() {
                let owner = (EntityKind::Monster, target.type_id);
                rules.push((format!("m{}:{index}", target.type_id), rule, Some(owner)));
            }
        }
        let eldorado = world.map.as_ref().is_some_and(Self::on_eldorado);
        let bonus = self.bonus.inside.is_some();
        // El Dorado's own Swiftstone, unless the user has a rule for it.
        let swiftstone = ItemRule {
            every_secs: 0,
            ..ItemRule::new(ITEM_SWIFTSTONE, "Swiftstone", Scene::ElDorado, Condition::None)
        };
        let user_swiftstone = rules
            .iter()
            .any(|(_, r, _)| r.item == ITEM_SWIFTSTONE && r.scene == Scene::ElDorado);
        if self.cfg.eldorado.enabled && self.cfg.eldorado.swiftstone && !user_swiftstone {
            rules.push(("eldorado-swiftstone".to_owned(), &swiftstone, None));
        }
        if rules.is_empty() {
            return;
        }
        let held = self.war.held(now, hero);
        let engaged = self.engage.as_ref().map(|e| (e.kind, e.type_id, e.id));
        let target = engaged.map(|(kind, type_id, _)| (kind, type_id));
        let target_hp = engaged
            .and_then(|(_, _, id)| world.actor(id))
            .and_then(|a| a.hp_percent);
        let sight = (hero.cannon_range.max(hero.harpoon_range) * SIGHT_FACTOR).max(SIGHT_MIN);
        let in_sight: Vec<(EntityKind, i32)> = world
            .actors
            .iter()
            .filter(|a| a.visible && matches!(a.kind, EntityKind::Npc | EntityKind::Monster))
            .filter(|a| a.pos.distance(hero.pos) <= sight)
            .map(|a| (a.kind, a.type_id))
            .collect();
        let war = self.cfg.mode.war();
        let activity = self.activity;
        let mut decisions = Vec::new();
        for (key, rule, owner) in rules {
            if decisions.len() >= USES_PER_STEP {
                break;
            }
            if !rule.enabled {
                continue;
            }
            let Some(item) = world.item(rule.item).copied() else {
                continue;
            };
            let scene = scene_holds(
                rule.scene,
                owner,
                activity,
                target,
                &in_sight,
                Modes { war, eldorado, bonus },
            );
            let condition = match rule.condition {
                Condition::None => true,
                Condition::HpBelow(p) => hero.hp_known && hero.hp_percent() < f32::from(p),
                Condition::VpBelow(p) => hero.vp_percent().is_some_and(|v| v < f32::from(p)),
                Condition::TargetHpBelow(p) => target_hp.is_some_and(|h| h < f32::from(p)),
                Condition::Held => held,
            };
            let base_every = u64::from(rule.every_secs) * 1_000;
            // A human does not re-use an item on an exact clock; spread the
            // interval so repeated uses never share one cadence. Inlined
            // (not a `self` method) so the `&self.cfg` borrow above and the
            // `&mut self.human` draw stay on disjoint fields.
            let hum = self.cfg.humanize;
            let every_ms = if hum.enabled && hum.jitter_pct != 0 {
                let span = base_every.max(USE_GAP_MS) * u64::from(hum.jitter_pct) / 100;
                base_every.saturating_add(self.human.range_u64(0, span))
            } else {
                base_every
            };
            let want = Want {
                key,
                item: &item,
                wanted: scene && condition,
                aim: None,
                can_off: rule.off_delay_ms > 0,
                only_ours: true,
                off_delay_ms: rule.off_delay_ms,
                every_ms,
            };
            if let Some((action, off)) = self.items.decide(now, want) {
                decisions.push((action, off, rule.name.clone()));
            }
        }
        for (action, off, name) in decisions {
            out.push(action);
            let code = if off {
                EventCode::ItemOff
            } else {
                EventCode::ItemUsed
            };
            self.emit(Level::Info, code, &[name]);
        }
    }
}

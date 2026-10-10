//! War mode: PvP assist. The player steers; this only picks the enemy, fires
//! at it through the game's own attack call and switches action items on and
//! off as the fight changes. It never emits `Move`, `Repair` or `Respawn`.

use kraken_proto::{
    Activity, Capabilities, EnemyScope, EntityKind, EventCode, ItemWhen, Level, WarConfig, WarInfo,
    WarItemRule,
};

use super::Engine;
use super::items::{ItemRunner, Want};
use super::world::{Action, ActionItem, Actor, Eid, HeroState, Relation, World};

/// Smallest gap between two attack calls on the same target.
const ATTACK_GAP_MS: u64 = 150;
/// Re-issue the attack after this long even while the game reports one.
const ATTACK_REFRESH_MS: u64 = 4_000;
/// Range margin over the current cannon range before the attack call.
const FIRE_MARGIN: f32 = 1.02;
const SIGHT_FLOOR: f32 = 30.0;
/// Effects (`module.effect.Effect`) that hold the ship in place.
const HOLDING_EFFECTS: [&str; 4] = ["SMOKEBOMB", "ICEAMMO_FREEZE", "CHAINS", "STRONG_ICE"];
/// How long a holding effect counts when the server gives no duration and
/// sends no "off".
const HOLD_FALLBACK_MS: u64 = 6_000;
/// How long the game's own "movement disabled" flag must stay off before it
/// counts as being held (it also flickers during map changes).
const FROZEN_FLAG_MS: u64 = 300;

#[derive(Debug, Default)]
pub(super) struct War {
    target: Option<Eid>,
    locked: bool,
    /// The selection we last saw or set, to tell the player's clicks apart.
    seen_selection: Option<Eid>,
    last_attack: u64,
    attacked: Option<Eid>,
    rules: ItemRunner,
    enemies: u16,
    distance: f32,
    /// A holding effect (smoke bomb, freeze) is on the hero until then.
    held_until: Option<u64>,
    /// Since when the game has reported the hero's movement as disabled.
    frozen_since: Option<u64>,
}

impl War {
    pub(super) fn reset(&mut self) {
        self.target = None;
        self.locked = false;
        self.attacked = None;
    }

    /// An effect message about the hero.
    pub(super) fn effect(&mut self, now: u64, name: &str, active: bool, millis: Option<u64>) {
        if !HOLDING_EFFECTS.contains(&name) {
            return;
        }
        self.held_until = active.then(|| now + millis.filter(|m| *m > 0).unwrap_or(HOLD_FALLBACK_MS));
    }

    pub(super) fn held(&mut self, now: u64, hero: &HeroState) -> bool {
        if hero.movement_enabled == Some(false) {
            self.frozen_since.get_or_insert(now);
        } else {
            self.frozen_since = None;
        }
        let flag = self
            .frozen_since
            .is_some_and(|t| now.saturating_sub(t) >= FROZEN_FLAG_MS);
        let effect = self.held_until.is_some_and(|t| now < t);
        flag || effect
    }

    pub(super) fn removed(&mut self, id: Eid) {
        if self.target.is_some_and(|t| t.matches(id)) {
            self.reset();
        }
    }
}

fn hostile(cfg: &WarConfig, actor: &Actor) -> bool {
    if actor.kind != EntityKind::Player || cfg.is_friend(&actor.name) || actor.relation.friendly() {
        return false;
    }
    match cfg.scope {
        EnemyScope::AllPlayers => true,
        EnemyScope::ByRelation => match actor.relation {
            Relation::War => cfg.relations.war,
            Relation::Neutral | Relation::Unknown => cfg.relations.neutral,
            Relation::Custom1 => cfg.relations.custom1,
            Relation::Custom2 => cfg.relations.custom2,
            Relation::NoGuild => cfg.guildless,
            Relation::OwnGuild | Relation::Group | Relation::Peace => false,
        },
    }
}

fn friendly(cfg: &WarConfig, actor: &Actor) -> bool {
    actor.kind == EntityKind::Player && (actor.relation.friendly() || cfg.is_friend(&actor.name))
}

fn selected_is(hero: &HeroState, id: Eid) -> bool {
    hero.selected.is_some_and(|s| s.matches(id))
}

impl Engine {
    pub(super) fn war_step(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        caps: &Capabilities,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        self.activity = Activity::War;
        if !caps.select_target {
            self.warn_once(now, EventCode::BindingMissing, 1, &["war".into()]);
            return out;
        }
        if hero.dead() {
            self.war.reset();
            self.activity = Activity::Dead;
            return out;
        }
        let cfg = self.cfg.war.clone();
        let base = hero.base_range();
        let sight = (base * f32::from(cfg.sight_pct) / 100.0).max(SIGHT_FLOOR);

        self.war_selection(world, hero);
        let valid = |id: Eid| world.actor(id).is_some_and(|a| a.kind == EntityKind::Player);
        if self.war.target.is_some_and(|t| !valid(t)) {
            self.war.reset();
        }

        let enemies: Vec<&Actor> = world
            .actors
            .iter()
            .filter(|a| a.visible && hostile(&cfg, a) && a.pos.distance(hero.pos) <= sight)
            .collect();
        self.war.enemies = enemies.len() as u16;
        if !self.war.locked {
            self.war_pick(hero, &enemies);
        }

        let target = self.war.target.and_then(|t| world.actor(t));
        self.war.distance = target.map_or(0.0, |a| a.pos.distance(hero.pos));
        if let Some(actor) = target {
            self.war_fire(now, hero, actor, &mut out);
        }
        if caps.use_item {
            self.war_items(now, world, hero, &cfg, target, &mut out);
        }
        out
    }

    /// Follows the player's own clicks: a picked player locks the bot onto
    /// them; a picked NPC is left to the player.
    fn war_selection(&mut self, world: &World, hero: &HeroState) {
        let selection = hero.selected;
        let same = match (selection, self.war.seen_selection) {
            (Some(a), Some(b)) => a.matches(b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.war.seen_selection = selection;
        match selection.and_then(|s| world.actor(s)) {
            Some(actor)
                if actor.kind == EntityKind::Player
                    && !actor.id.matches(hero.id)
                    && !matches!(actor.relation, Relation::OwnGuild | Relation::Group) =>
            {
                let changed = !self.war.target.is_some_and(|t| t.matches(actor.id));
                self.war.target = Some(actor.id);
                self.war.locked = true;
                if changed {
                    self.war.attacked = None;
                    self.emit(
                        Level::Info,
                        EventCode::WarTarget,
                        &[actor.name.clone(), "locked".into()],
                    );
                }
            }
            _ => self.war.locked = false,
        }
    }

    /// Nearest enemy, but a target already in range is kept until it leaves
    /// range while another one is in it.
    fn war_pick(&mut self, hero: &HeroState, enemies: &[&Actor]) {
        let range = hero.cannon_range * FIRE_MARGIN;
        let dist = |a: &Actor| a.pos.distance(hero.pos);
        let best = enemies.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).copied();
        let current = self
            .war
            .target
            .and_then(|t| enemies.iter().find(|a| a.id.matches(t)))
            .copied();
        let choice = match (current, best) {
            (Some(c), Some(b)) if dist(c) <= range || dist(b) > range => Some(c),
            (_, b) => b,
        };
        let changed = match (choice, self.war.target) {
            (Some(c), Some(t)) => !c.id.matches(t),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.war.target = choice.map(|c| c.id);
            self.war.attacked = None;
            if let Some(c) = choice {
                self.emit(Level::Info, EventCode::WarTarget, std::slice::from_ref(&c.name));
            }
        }
    }

    fn war_fire(&mut self, now: u64, hero: &HeroState, actor: &Actor, out: &mut Vec<Action>) {
        if !selected_is(hero, actor.id) {
            out.push(Action::Select(actor.id));
            self.war.seen_selection = Some(actor.id);
        }
        if actor.pos.distance(hero.pos) > hero.cannon_range * FIRE_MARGIN {
            return;
        }
        let fresh = !self.war.attacked.is_some_and(|a| a.matches(actor.id));
        let since = now.saturating_sub(self.war.last_attack);
        let due = fresh || (!self.attack_active && since >= ATTACK_GAP_MS) || since >= ATTACK_REFRESH_MS;
        if due {
            out.push(Action::AttackSelected);
            self.war.last_attack = now;
            self.war.attacked = Some(actor.id);
        }
    }

    fn war_items(
        &mut self,
        now: u64,
        world: &World,
        hero: &HeroState,
        cfg: &WarConfig,
        target: Option<&Actor>,
        out: &mut Vec<Action>,
    ) {
        let base = hero.base_range();
        let target_dist = target.map(|a| a.pos.distance(hero.pos));
        let held = self.war.held(now, hero);
        for rule in cfg.items.iter().filter(|r| r.enabled) {
            if rule.when.support() && !cfg.support {
                continue;
            }
            let Some(item) = world.item(rule.item).copied() else {
                continue;
            };
            let (wanted, aim) = if rule.when == ItemWhen::Immobilized {
                (held, None)
            } else {
                Self::rule_wants(rule, world, hero, cfg, base, target_dist, &item)
            };
            let want = Want {
                key: format!("w{}", rule.item),
                item: &item,
                wanted,
                aim,
                can_off: toggles(rule.when),
                only_ours: false,
                off_delay_ms: rule.off_delay_ms,
                every_ms: 0,
            };
            if let Some((action, off)) = self.war.rules.decide(now, want) {
                out.push(action);
                let code = if off {
                    EventCode::ItemOff
                } else {
                    EventCode::ItemUsed
                };
                self.emit(Level::Info, code, std::slice::from_ref(&rule.name));
            }
        }
    }

    /// Whether a rule wants its item now, and the friend to aim it at.
    fn rule_wants(
        rule: &WarItemRule,
        world: &World,
        hero: &HeroState,
        cfg: &WarConfig,
        base: f32,
        target_dist: Option<f32>,
        item: &ActionItem,
    ) -> (bool, Option<Eid>) {
        let pct = |p: u16| base * f32::from(p) / 100.0;
        let reach = if item.range > 0.0 { item.range } else { base };
        let friends = || {
            world
                .actors
                .iter()
                .filter(|a| a.visible && friendly(cfg, a))
                .map(|a| (a, a.pos.distance(hero.pos)))
        };
        match rule.when {
            ItemWhen::TargetNear { max_pct } => (target_dist.is_some_and(|d| d <= pct(max_pct)), None),
            ItemWhen::TargetFar { min_pct } => (target_dist.is_some_and(|d| d >= pct(min_pct)), None),
            ItemWhen::InCombat => (
                target_dist.is_some_and(|d| d <= hero.cannon_range * FIRE_MARGIN),
                None,
            ),
            ItemWhen::Always => (true, None),
            ItemWhen::HpBelow(p) => (hero.hp_known && hero.hp_percent() < f32::from(p), None),
            ItemWhen::VpBelow(p) => (hero.vp_percent().is_some_and(|v| v < f32::from(p)), None),
            ItemWhen::FriendHpBelow(p) => {
                let aim = friends()
                    .filter(|(a, d)| *d <= reach && a.hp_percent.is_some_and(|h| h < f32::from(p)))
                    .min_by(|(a, _), (b, _)| {
                        a.hp_percent
                            .unwrap_or(100.0)
                            .total_cmp(&b.hp_percent.unwrap_or(100.0))
                    })
                    .map(|(a, _)| a.id);
                (aim.is_some(), aim)
            }
            ItemWhen::FriendNear { max_pct } => {
                let limit = pct(max_pct).min(reach);
                let aim = friends()
                    .filter(|(_, d)| *d <= limit)
                    .min_by(|(_, a), (_, b)| a.total_cmp(b))
                    .map(|(a, _)| a.id);
                (aim.is_some(), aim)
            }
            // Decided in `war_items`, which tracks the hero's effects.
            ItemWhen::Immobilized => (false, None),
        }
    }

    /// Whether this feedback puts a holding effect on the hero, so the
    /// counter item goes out in the same frame.
    pub fn holds_hero(&self, feedback: &super::world::Feedback) -> bool {
        matches!(
            feedback,
            super::world::Feedback::Effect { name, target, active: true, .. }
                if self.hero_id.is_set()
                    && target.matches(self.hero_id)
                    && HOLDING_EFFECTS.contains(&name.as_str())
        )
    }

    pub fn war_info(&self, world: &World) -> Option<WarInfo> {
        if !self.cfg.mode.war() {
            return None;
        }
        let target = self.war.target.and_then(|t| world.actor(t));
        Some(WarInfo {
            target: target.map(|a| a.name.clone()),
            distance: self.war.distance,
            locked: self.war.locked,
            active_items: world.items.iter().filter(|i| i.active).map(|i| i.id).collect(),
            enemies: self.war.enemies,
        })
    }

    pub fn war_target(&self) -> Option<Eid> {
        self.cfg.mode.war().then_some(self.war.target).flatten()
    }
}

/// Distance and combat rules switch their item back off; health, voodoo and
/// support rules only ever use theirs.
fn toggles(when: ItemWhen) -> bool {
    matches!(
        when,
        ItemWhen::TargetNear { .. } | ItemWhen::TargetFar { .. } | ItemWhen::InCombat | ItemWhen::Always
    )
}

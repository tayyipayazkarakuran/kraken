//! What the war bot reads and does beyond the regular scan: the player's
//! selection, guild relations, the group, action items and their cooldowns,
//! and the game's own select, attack and item calls.

use std::collections::{HashMap, HashSet};

use super::Game;
use crate::engine::world::{ActionItem, Eid, Relation};
use crate::il2cpp::{Class, Field, Method, Ptr, Slot, boxed_bool};

/// Methods and fields found once in `Game::resolve`.
#[derive(Clone, Copy, Default)]
pub struct WarBindings {
    /// `GameActorModel.set_TargetIdValue(EntityId)`.
    pub select: Option<Method>,
    /// `AttackSystem.Attack()`.
    pub attack: Option<Method>,
    /// `ActionItemUtils.useActionItem(network, model, mapView, worldMap, cooldowns, stub)`.
    pub use_item: Option<Method>,
    /// `DefaultItemConfigurationModel.ActionItems` (static).
    pub items: Option<Field>,
    /// `CooldownSystem.IsAtLeastOneStackReady(short)`.
    pub stack_ready: Option<Method>,
    /// `Seafight.GameActors.Target`, the per-ship component that holds the group system.
    pub target_component: Option<Class>,
}

/// Item state is read this often; each item costs a cooldown call.
const ITEMS_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// Teleport takes a map pick from a window: kept out of the item rules, used
/// only by the work map travel.
pub const TELEPORT_ITEM: i16 = 34;

pub type GuildKey = (i64, i64);

#[derive(Default)]
pub struct Guilds {
    pub own: Option<GuildKey>,
    pub colours: HashMap<GuildKey, String>,
}

impl WarBindings {
    pub fn resolve(registry: &crate::il2cpp::Registry, missing: &mut Vec<String>) -> Self {
        let find = |name: &str| registry.find(name, Some(super::GAME));
        let select =
            find("Seafight.GameActors.GameActorModel").and_then(|c| c.method("set_TargetIdValue", 1));
        let attack = find("Seafight.Attack.AttackSystem").and_then(|c| c.method("Attack", 0));
        let use_item =
            find("Seafight.UserInterface.ActionItemUtils").and_then(|c| c.method("useActionItem", 6));
        let items =
            find("Seafight.Storage.DefaultItemConfigurationModel").and_then(|c| c.field("ActionItems"));
        let stack_ready = find("Seafight.CooldownSystem").and_then(|c| c.method("IsAtLeastOneStackReady", 1));
        let target_component = find("Seafight.GameActors.Target");
        for (label, ok) in [
            ("GameActorModel.set_TargetIdValue", select.is_some()),
            ("AttackSystem.Attack", attack.is_some()),
            ("ActionItemUtils.useActionItem", use_item.is_some()),
            ("DefaultItemConfigurationModel.ActionItems", items.is_some()),
            ("CooldownSystem.IsAtLeastOneStackReady", stack_ready.is_some()),
        ] {
            if !ok {
                missing.push(label.to_owned());
            }
        }
        Self {
            select,
            attack,
            use_item,
            items,
            stack_ready,
            target_component,
        }
    }
}

impl Game {
    fn guild_key(&mut self, guild: Ptr) -> Option<GuildKey> {
        let id = self.int(guild, "id")?;
        (id > 0).then(|| (id, self.int(guild, "instanceid").unwrap_or(0)))
    }

    /// The guild an actor flies for, from its `GameActorGuildData` component.
    pub fn actor_guild(&mut self, component: Ptr) -> Option<GuildKey> {
        let guild = self.obj(component, "guildid");
        self.guild_key(guild)
    }

    /// What the player has selected: `GameActorModel.TargetId.Value`.
    pub fn selected(&mut self, model: Ptr) -> Option<Eid> {
        let property = self.obj(model, "targetid");
        let field = Class::of(property).and_then(|c| self.field(c, "value"))?;
        self.eid_field(property, field).filter(|e| e.id > 0)
    }

    pub fn guilds(&mut self, model: Ptr) -> Guilds {
        let system = self.obj(model, "guildsystem");
        let own = self.obj(system, "userguildid");
        let own = self.obj(own, "value");
        let mut guilds = Guilds {
            own: self.guild_key(own),
            colours: HashMap::new(),
        };
        let table = self.obj(system, "guildrelationstatus");
        for (key, value) in self.collections.dict(table) {
            let Some(guild) = self.guild_key(key.object()) else {
                continue;
            };
            if let Some(colour) = value.i64().and_then(|v| self.enum_name(value.ty, v)) {
                guilds.colours.insert(guild, colour);
            }
        }
        guilds
    }

    /// Group members, from the group system any ship's `Target` component holds.
    pub fn group(&mut self, view: Ptr) -> HashSet<Eid> {
        let (Some(get), Some(class)) = (self.unity.get_component, self.war.target_component) else {
            return HashSet::new();
        };
        if view.is_null() {
            return HashSet::new();
        }
        let component = match get.invoke(view, &mut [class.type_object()]) {
            Ok(c) if !c.is_null() => c,
            _ => return HashSet::new(),
        };
        let system = self.obj(component, "groupsystem");
        let members = self.obj(system, "usersingroup");
        self.collections
            .set(members)
            .into_iter()
            .filter_map(|slot| self.read_eid(slot))
            .collect()
    }

    pub fn relation(guilds: &Guilds, group: &HashSet<Eid>, id: Eid, guild: Option<GuildKey>) -> Relation {
        if group.iter().any(|m| m.matches(id)) {
            return Relation::Group;
        }
        let Some(guild) = guild else {
            return Relation::NoGuild;
        };
        if guilds.own == Some(guild) {
            return Relation::OwnGuild;
        }
        match guilds.colours.get(&guild).map(String::as_str) {
            Some("PEACE") => Relation::Peace,
            Some("WAR") => Relation::War,
            Some("CUSTOM1") => Relation::Custom1,
            Some("CUSTOM2") => Relation::Custom2,
            _ => Relation::Neutral,
        }
    }

    fn stubs(&mut self) -> Vec<(i16, Ptr)> {
        let Some(field) = self.war.items else {
            return Vec::new();
        };
        let table = field.static_ptr();
        self.collections
            .dict(table)
            .into_iter()
            .filter_map(|(key, value)| Some((key.i64()? as i16, value.object())))
            .filter(|(_, stub)| !stub.is_null())
            .collect()
    }

    fn stack_ready(&self, cooldowns: Ptr, id: i64) -> bool {
        let Some(method) = self.war.stack_ready else {
            return true;
        };
        if cooldowns.is_null() {
            return true;
        }
        let mut raw = id as i16;
        method
            .invoke(cooldowns, &mut [(&raw mut raw).cast()])
            .ok()
            .and_then(boxed_bool)
            .unwrap_or(true)
    }

    /// Owned action items with their live state.
    pub fn action_items(&mut self, attack_system: Ptr) -> Vec<ActionItem> {
        let cooldowns = self.obj(attack_system, "cooldownsystem");
        let mut out = Vec::new();
        self.teleport_stock = (0, false);
        for (id, stub) in self.stubs() {
            let amount = self.int(stub, "amount").unwrap_or(0) as i32;
            if amount <= 0 {
                continue;
            }
            let pre = self.int(stub, "cooldownidpre").unwrap_or(0);
            if id == TELEPORT_ITEM {
                self.teleport_stock = (amount, self.stack_ready(cooldowns, pre));
                continue;
            }
            out.push(ActionItem {
                id,
                amount,
                active: self.int(stub, "isactive").is_some_and(|v| v != 0),
                range: self.int(stub, "range").unwrap_or(0) as f32,
                ready: self.stack_ready(cooldowns, pre),
            });
        }
        out.sort_by_key(|i| i.id);
        out
    }

    pub fn cached_items(&mut self, attack_system: Ptr) -> Vec<ActionItem> {
        if self.items.0.is_none_or(|t| t.elapsed() >= ITEMS_EVERY) {
            let items = self.action_items(attack_system);
            self.items = (Some(std::time::Instant::now()), items);
        }
        self.items.1.clone()
    }

    /// Forces a fresh read on the next scan, after the bot used an item.
    pub fn refresh_items(&mut self) {
        self.items.0 = None;
    }

    fn model_and_attack(&mut self, network: Ptr) -> (Ptr, Ptr) {
        let handler = self.obj(network, "userinitmessagehandler");
        let model = self.obj(handler, "gameactormodel");
        let info = self.obj(handler, "playerinfosystem");
        (model, self.obj(info, "attacksystem"))
    }

    /// The actor's `EntityId` exactly as the game keys it, protocol id included.
    fn entity_key(&mut self, model: Ptr, target: Eid) -> Option<[u8; 32]> {
        let table = self.obj(model, "actors");
        let (slot, size) = self.collections.dict(table).into_iter().find_map(|(key, _)| {
            let id = self.read_eid(key)?;
            id.matches(target)
                .then(|| (key, key.ty.class().map_or(0, Class::value_size)))
        })?;
        key_bytes(slot, size)
    }

    fn set_selection(&mut self, model: Ptr, target: Eid) -> Result<(), String> {
        let select = self.war.select.ok_or("set_TargetIdValue unavailable")?;
        let mut key = self
            .entity_key(model, target)
            .ok_or_else(|| format!("{target:?} not in actor table"))?;
        select.invoke(model, &mut [key.as_mut_ptr().cast()]).map(|_| ())
    }

    pub fn select(&mut self, network: Ptr, target: Eid) -> Result<(), String> {
        let (model, _) = self.model_and_attack(network);
        if model.is_null() {
            return Err("no actor model".into());
        }
        self.set_selection(model, target)
    }

    pub fn attack_selected(&mut self, network: Ptr) -> Result<(), String> {
        let attack = self.war.attack.ok_or("AttackSystem.Attack unavailable")?;
        let (_, system) = self.model_and_attack(network);
        if system.is_null() {
            return Err("no attack system".into());
        }
        attack.invoke(system, &mut []).map(|_| ())
    }

    pub fn use_item(&mut self, network: Ptr, item: i16, target: Option<Eid>) -> Result<(), String> {
        let method = self.war.use_item.ok_or("useActionItem unavailable")?;
        let stub = self
            .stubs()
            .into_iter()
            .find_map(|(id, stub)| (id == item).then_some(stub))
            .ok_or_else(|| format!("item {item} not owned"))?;
        let (model, attack) = self.model_and_attack(network);
        if model.is_null() {
            return Err("no actor model".into());
        }
        let cooldowns = self.obj(attack, "cooldownsystem");
        let map_view = self.obj(model, "mapview");
        let previous = self.selected(model);
        if let Some(target) = target {
            self.set_selection(model, target)?;
        }
        let result = method
            .invoke(
                std::ptr::null_mut(),
                &mut [network, model, map_view, std::ptr::null_mut(), cooldowns, stub],
            )
            .map(|_| ());
        self.refresh_items();
        if target.is_some()
            && let Some(previous) = previous
        {
            let _ = self.set_selection(model, previous);
        }
        result
    }
}

fn key_bytes(slot: Slot, size: usize) -> Option<[u8; 32]> {
    if size == 0 || size > 32 || !crate::il2cpp::readable(slot.at, size) {
        return None;
    }
    let mut out = [0u8; 32];
    unsafe { std::ptr::copy_nonoverlapping(slot.at, out.as_mut_ptr(), size) };
    Some(out)
}

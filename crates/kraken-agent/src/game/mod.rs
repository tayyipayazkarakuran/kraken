mod act;
mod bonus;
mod loca;
pub mod messages;
pub mod projection;
mod read;
mod skills;
mod ui;
mod war;
mod workmap;

use std::collections::HashMap;

use kraken_proto::{BindingReport, Capabilities};

use crate::engine::world::Eid;
use crate::il2cpp::{
    Class, Collections, Field, Method, Ptr, Registry, Slot, Type, get_ref, inline_i64, normalize,
};

pub use read::{Scan, Vec3};
pub use workmap::{load_edges, save_edges};

const PROTOCOL: &str = "libcom-protocol";
const GAME: &str = "Seafight";

pub struct Messages {
    pub moving: Option<Class>,
    pub cannon: Option<Class>,
    pub harpoon: Option<Class>,
    pub abort: Option<Class>,
    pub repair: Option<Class>,
    pub respawn: Option<Class>,
}

pub struct Unity {
    pub get_transform: Method,
    pub get_position: Method,
    pub get_component: Option<Method>,
    pub camera_main: Option<Method>,
    pub world_to_screen: Option<Method>,
    pub screen_width: Option<Method>,
    pub screen_height: Option<Method>,
}

pub struct Game {
    pub registry: Registry,
    pub collections: Collections,
    pub network: Class,
    pub tick: Method,
    pub dispatch: Option<Method>,
    pub is_connected: Method,
    pub reconnect: Option<Method>,
    pub connect: Option<Method>,
    pub map_current: Field,
    pub map_to_field: Method,
    pub map_view_instance: Option<Field>,
    pub coord_blocked: Option<Method>,
    pub movement_class: Option<Class>,
    pub box_types: Option<Class>,
    pub unity: Unity,
    pub messages: Messages,
    pub war: war::WarBindings,
    pub skill_bindings: skills::SkillBindings,
    pub bonus_bindings: bonus::BonusBindings,
    pub workmap_bindings: workmap::WorkmapBindings,
    atlas: workmap::AtlasCache,
    bonus_waves: HashMap<i16, u8>,
    skills: skills::SkillCache,
    send: HashMap<Class, Option<Method>>,
    fields: HashMap<(Class, &'static str), Option<Field>>,
    eids: HashMap<Class, (Option<Field>, Option<Field>)>,
    enums: HashMap<Class, HashMap<i64, String>>,
    missing: Vec<String>,
    items: (Option<std::time::Instant>, Vec<crate::engine::world::ActionItem>),
    /// Teleport items (action item 34) in stock, and whether one is ready.
    teleport_stock: (i32, bool),
}

unsafe impl Send for Game {}

fn require<T>(missing: &mut Vec<String>, label: &str, value: Option<T>) -> Option<T> {
    if value.is_none() {
        missing.push(label.to_owned());
    }
    value
}

impl Game {
    pub fn resolve(registry: Registry) -> Result<Self, Vec<String>> {
        let mut missing = Vec::new();
        let find = |name: &str, image: &str| registry.find(name, Some(image));

        let network = require(
            &mut missing,
            "NetworkManager",
            find("Seafight.Network.NetworkManager", GAME),
        );
        let tick = network.and_then(|c| c.method("Tick", 0));
        let tick = require(&mut missing, "NetworkManager.Tick", tick);
        let is_connected = require(
            &mut missing,
            "NetworkManager.IsConnected",
            network.and_then(|c| c.method("IsConnected", 0)),
        );
        let reconnect = network.and_then(|c| {
            c.method_where("ReConnect", |m| {
                m.param_count() == 1 && m.param(0).name() == "System.Boolean"
            })
        });
        let connect = network.and_then(|c| c.method("Connect", 0));
        let dispatch = Self::dispatch(&registry);
        if dispatch.is_none() {
            missing.push("SeafightDispatcher.Dispatch".into());
        }

        let map_current = find("Seafight.Map.MapData", GAME).and_then(|c| c.field("current"));
        let map_current = require(&mut missing, "MapData.current", map_current);
        let map_to_field =
            find("Seafight.Utilities.MapUtils", GAME).and_then(|c| c.method("GetMapFieldFloat", 1));
        let map_to_field = require(&mut missing, "MapUtils.GetMapFieldFloat", map_to_field);
        let map_view = find("MapView", GAME);
        let map_view_instance = map_view.and_then(|c| c.field("instance"));
        let coord_blocked = map_view.and_then(|c| {
            c.method_where("isCoordBlocked", |m| {
                m.param_count() == 1 && m.param(0).name() == "UnityEngine.Vector2Int"
            })
        });
        let movement_class = find("Seafight.GameActors.MovementBehaviour", GAME);

        let component = registry.find("UnityEngine.Component", Some("UnityEngine.CoreModule"));
        let transform = registry.find("UnityEngine.Transform", Some("UnityEngine.CoreModule"));
        let camera = registry.find("UnityEngine.Camera", Some("UnityEngine.CoreModule"));
        let screen = registry.find("UnityEngine.Screen", Some("UnityEngine.CoreModule"));
        let get_transform = require(
            &mut missing,
            "Component.get_transform",
            component.and_then(|c| c.method("get_transform", 0)),
        );
        let get_position = require(
            &mut missing,
            "Transform.get_position",
            transform.and_then(|c| c.method("get_position", 0)),
        );
        let get_component = component.and_then(|c| {
            c.method_where("GetComponent", |m| {
                m.param_count() == 1 && m.param(0).name() == "System.Type"
            })
        });

        let message = |name: &str| registry.find(name, Some(PROTOCOL));
        let messages = Messages {
            moving: message("MoveMessage"),
            cannon: message("CannonAttackMessage"),
            harpoon: message("HarpoonAttackMessage"),
            abort: message("AbortAttackMessage"),
            repair: message("RepairMessage"),
            respawn: message("MapReturnToOldMapMessage"),
        };
        for (label, class) in [
            ("MoveMessage", messages.moving),
            ("CannonAttackMessage", messages.cannon),
            ("HarpoonAttackMessage", messages.harpoon),
            ("AbortAttackMessage", messages.abort),
            ("RepairMessage", messages.repair),
            ("MapReturnToOldMapMessage", messages.respawn),
        ] {
            if class.is_none() {
                missing.push(label.to_owned());
            }
        }

        let (Some(network), Some(tick), Some(is_connected), Some(map_current), Some(map_to_field)) =
            (network, tick, is_connected, map_current, map_to_field)
        else {
            return Err(missing);
        };
        let (Some(get_transform), Some(get_position)) = (get_transform, get_position) else {
            return Err(missing);
        };

        let box_types = registry.find("BoxType", Some(PROTOCOL)).filter(|c| c.is_enum());
        let war = war::WarBindings::resolve(&registry, &mut missing);
        let skill_bindings = skills::SkillBindings::resolve(&registry, &mut missing);
        let bonus_bindings = bonus::BonusBindings::resolve(&registry, &mut missing);
        let workmap_bindings = workmap::WorkmapBindings::resolve(&registry, &mut missing);
        if missing.is_empty() {
            tracing::info!("all bindings resolved");
        } else {
            tracing::warn!(?missing, "optional bindings missing");
        }
        Ok(Self {
            registry,
            collections: Collections::default(),
            network,
            tick,
            dispatch,
            is_connected,
            reconnect,
            connect,
            map_current,
            map_to_field,
            map_view_instance,
            coord_blocked,
            movement_class,
            box_types,
            unity: Unity {
                get_transform,
                get_position,
                get_component,
                camera_main: camera.and_then(|c| c.method("get_main", 0)),
                world_to_screen: camera.and_then(|c| {
                    c.method_where("WorldToScreenPoint", |m| {
                        m.param_count() == 1 && m.param(0).name() == "UnityEngine.Vector3"
                    })
                }),
                screen_width: screen.and_then(|c| c.method("get_width", 0)),
                screen_height: screen.and_then(|c| c.method("get_height", 0)),
            },
            messages,
            war,
            skill_bindings,
            bonus_bindings,
            workmap_bindings,
            atlas: workmap::AtlasCache::default(),
            bonus_waves: HashMap::new(),
            skills: skills::SkillCache::default(),
            send: HashMap::new(),
            fields: HashMap::new(),
            eids: HashMap::new(),
            enums: HashMap::new(),
            missing,
            items: (None, Vec::new()),
            teleport_stock: (0, false),
        })
    }

    /// `SeafightDispatcher.Dispatch<T>` as the game runs it: every message is
    /// a reference type, so all of them go through the one shared instance.
    fn dispatch(registry: &Registry) -> Option<Method> {
        let dispatcher = registry.find("Seafight.Network.SeafightDispatcher", Some(GAME))?;
        let definition = dispatcher.method("Dispatch", 1)?;
        let sample = registry.find("DisplayHitMessage", Some(PROTOCOL))?;
        let type_class = registry.find("System.Type", Some("mscorlib"))?;
        let shared = definition.inflate(dispatcher, &[sample], type_class)?;
        let base = crate::il2cpp::game_module().map_or(0, |m| m as usize);
        let rva = (shared.code() as usize).wrapping_sub(base);
        if shared.code() == definition.code() || shared.code().is_null() {
            tracing::warn!(rva = format_args!("{rva:#x}"), "dispatch did not inflate");
            return None;
        }
        tracing::info!(rva = format_args!("{rva:#x}"), "dispatch resolved");
        Some(shared)
    }

    pub fn report(&mut self, network: Ptr) -> BindingReport {
        let sendable =
            |game: &mut Self, class: Option<Class>| class.is_some_and(|c| game.sender(c).is_some());
        let movement = sendable(self, self.messages.moving);
        let capabilities = Capabilities {
            world: true,
            movement,
            cannon: sendable(self, self.messages.cannon),
            harpoon: sendable(self, self.messages.harpoon),
            abort_attack: sendable(self, self.messages.abort),
            repair: sendable(self, self.messages.repair),
            respawn: sendable(self, self.messages.respawn),
            reconnect: self.reconnect.is_some() || self.connect.is_some(),
            terrain: self.coord_blocked.is_some() && self.map_view_instance.is_some(),
            screen: self.unity.camera_main.is_some() && self.unity.world_to_screen.is_some(),
            select_target: self.war.select.is_some() && self.war.attack.is_some(),
            use_item: self.war.use_item.is_some() && self.war.items.is_some(),
        };
        let _ = network;
        BindingReport {
            missing: self.missing.clone(),
            capabilities,
        }
    }

    pub fn sender(&mut self, message: Class) -> Option<Method> {
        let network = self.network;
        *self.send.entry(message).or_insert_with(|| {
            network.method_where("Send", |m| {
                !m.is_static()
                    && m.param_count() == 1
                    && m.param(0).class().is_some_and(|p| p.is_assignable_from(message))
            })
        })
    }

    pub fn field(&mut self, class: Class, name: &'static str) -> Option<Field> {
        *self
            .fields
            .entry((class, name))
            .or_insert_with(|| class.instance_field(name))
    }

    pub fn obj(&mut self, object: Ptr, name: &'static str) -> Ptr {
        let Some(class) = Class::of(object) else {
            return std::ptr::null_mut();
        };
        match self.field(class, name) {
            Some(field) if field.ty().is_reference() => unsafe { get_ref(object, field) },
            _ => std::ptr::null_mut(),
        }
    }

    pub fn num(&mut self, object: Ptr, name: &'static str) -> Option<f64> {
        let class = Class::of(object)?;
        let field = self.field(class, name)?;
        unsafe { crate::il2cpp::get_f64(object, field) }
    }

    pub fn int(&mut self, object: Ptr, name: &'static str) -> Option<i64> {
        let class = Class::of(object)?;
        let field = self.field(class, name)?;
        unsafe { crate::il2cpp::get_i64(object, field) }
    }

    pub fn enum_name(&mut self, ty: Type, value: i64) -> Option<String> {
        let class = ty.class().filter(|c| c.is_enum())?;
        let names = self.enums.entry(class).or_insert_with(|| {
            class
                .enum_values()
                .into_iter()
                .map(|(name, v)| (v, name))
                .collect()
        });
        names.get(&value).cloned()
    }

    pub fn enum_value(&mut self, ty: Type, name: &str) -> Option<i64> {
        let class = ty.class().filter(|c| c.is_enum())?;
        let names = self
            .enums
            .entry(class)
            .or_insert_with(|| class.enum_values().into_iter().map(|(n, v)| (v, n)).collect());
        names.iter().find(|(_, n)| n.as_str() == name).map(|(v, _)| *v)
    }

    fn eid_fields(&mut self, class: Class) -> (Option<Field>, Option<Field>) {
        *self.eids.entry(class).or_insert_with(|| eid_layout(class))
    }

    pub fn read_eid(&mut self, slot: Slot) -> Option<Eid> {
        let class = slot.ty.class()?;
        if slot.ty.is_value() {
            let (id, instance) = self.eid_fields(class);
            let id = unsafe { inline_i64(slot.at, id?) }?;
            let instance = instance
                .and_then(|f| unsafe { inline_i64(slot.at, f) })
                .unwrap_or(0);
            return Some(Eid::new(id, instance));
        }
        self.eid_object(slot.object())
    }

    pub fn eid_object(&mut self, object: Ptr) -> Option<Eid> {
        let class = Class::of(object)?;
        let (id, instance) = self.eid_fields(class);
        let id = unsafe { crate::il2cpp::get_i64(object, id?) }?;
        let instance = instance
            .and_then(|f| unsafe { crate::il2cpp::get_i64(object, f) })
            .unwrap_or(0);
        Some(Eid::new(id, instance))
    }

    pub fn eid_field(&mut self, object: Ptr, field: Field) -> Option<Eid> {
        let at = unsafe { crate::il2cpp::field_addr(object, field) };
        self.read_eid(Slot { at, ty: field.ty() })
    }
}

pub fn eid_layout(class: Class) -> (Option<Field>, Option<Field>) {
    let fields: Vec<Field> = class
        .fields()
        .into_iter()
        .filter(|f| !f.is_static() && f.prim().is_some())
        .collect();
    let named = |wanted: &[&str]| {
        fields
            .iter()
            .copied()
            .find(|f| wanted.contains(&normalize(f.name()).as_str()))
    };
    let id = named(&["id", "entityid", "value"]).or_else(|| fields.first().copied());
    let instance =
        named(&["instanceid", "instance"]).or_else(|| fields.iter().copied().find(|f| Some(*f) != id));
    (id, instance)
}

pub fn is_eid_type(ty: Type) -> bool {
    let name = ty.name();
    name.ends_with("EntityId") || name.ends_with("EntityInstanceId")
}

pub fn pretty(raw: &str) -> String {
    raw.split(['_', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let lower = p.to_lowercase();
            let mut chars = lower.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn prettifies_enum_names() {
        assert_eq!(super::pretty("TREASURE_CHEST"), "Treasure Chest");
        assert_eq!(super::pretty("glitter"), "Glitter");
    }
}

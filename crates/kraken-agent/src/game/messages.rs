use std::collections::HashMap;

use super::{eid_layout, is_eid_type};
use crate::engine::world::{Eid, Feedback};
use crate::il2cpp::{
    Class, Field, Ptr, Registry, field_addr, get_bool, get_i64, get_ref, inline_i64, normalize,
};

#[derive(Clone, Copy)]
struct EidSlot {
    field: Field,
    value: bool,
    id: Field,
    instance: Option<Field>,
}

impl EidSlot {
    fn new(field: Field) -> Option<Self> {
        let ty = field.ty();
        let (id, instance) = eid_layout(ty.class()?);
        Some(Self {
            field,
            value: ty.is_value(),
            id: id?,
            instance,
        })
    }

    fn read(self, message: Ptr) -> Option<Eid> {
        unsafe {
            if self.value {
                let base = field_addr(message, self.field);
                let id = inline_i64(base, self.id)?;
                let instance = self.instance.and_then(|f| inline_i64(base, f)).unwrap_or(0);
                Some(Eid::new(id, instance))
            } else {
                let object = get_ref(message, self.field);
                let id = get_i64(object, self.id)?;
                let instance = self.instance.and_then(|f| get_i64(object, f)).unwrap_or(0);
                Some(Eid::new(id, instance))
            }
        }
    }
}

#[derive(Clone)]
struct EnumSlot {
    field: Field,
    names: HashMap<i64, String>,
}

impl EnumSlot {
    fn new(field: Field) -> Option<Self> {
        let class = field.ty().class().filter(|c| c.is_enum())?;
        Some(Self {
            field,
            names: class.enum_values().into_iter().map(|(n, v)| (v, n)).collect(),
        })
    }

    fn read(&self, message: Ptr) -> Option<&str> {
        let value = unsafe { get_i64(message, self.field) }?;
        self.names.get(&value).map(String::as_str)
    }
}

#[derive(Clone)]
enum Rule {
    Hit {
        source: EidSlot,
        target: EidSlot,
    },
    Removed(EidSlot),
    AttackState(EnumSlot),
    Aborted,
    Death,
    MapChanged,
    Spawned,
    Disconnect(Option<EnumSlot>),
    /// `EntityEffectMessage`, `TimedEffectMessage`, `MapObjectEffectMessage`.
    Effect {
        effect: EnumSlot,
        target: EidSlot,
        active: Option<Field>,
        millis: Option<Field>,
    },
    /// `MasterSkillTriggerMessage`.
    SkillTriggered {
        id: Field,
        remaining: Option<Field>,
    },
    /// `MapChangeDelayedMessage`.
    MapDelay {
        map: Field,
        delay: Field,
    },
    /// `MapChangeAskMessage`: the ship is at a map edge (or a confirmation
    /// is needed) and may go to `map`.
    MapAsk {
        map: Field,
        kind: Option<EnumSlot>,
    },
    /// `MapChangeCancelMessage`.
    MapCancel,
    /// `MasterSkillChargeUpdateMessage`; without a charge field, an untrigger.
    SkillCharge {
        id: Field,
        charge: Option<Field>,
    },
}

pub struct MessageMap {
    rules: HashMap<Class, Rule>,
}

unsafe impl Send for MessageMap {}
unsafe impl Sync for MessageMap {}

fn eid_fields(class: Class) -> Vec<Field> {
    let mut fields: Vec<Field> = class
        .lineage()
        .flat_map(Class::fields)
        .filter(|f| !f.is_static() && is_eid_type(f.ty()))
        .collect();
    fields.sort_by_key(|f| f.offset());
    fields
}

fn pick(fields: &[Field], hints: &[&str]) -> Option<Field> {
    fields.iter().copied().find(|f| {
        let name = normalize(f.name());
        hints.iter().any(|h| name.contains(h))
    })
}

fn enum_field(class: Class, name: &str) -> Option<EnumSlot> {
    let field = class.instance_field(name)?;
    EnumSlot::new(field)
}

impl MessageMap {
    pub fn build(registry: &Registry) -> Self {
        let mut rules = HashMap::new();
        let find = |name: &str| registry.find(name, Some(super::PROTOCOL));

        if let Some(class) = find("DisplayHitMessage") {
            let fields = eid_fields(class);
            let source = pick(&fields, &["source", "attacker"]).or_else(|| fields.first().copied());
            let target = pick(&fields, &["target", "defender"]).or_else(|| fields.get(1).copied());
            if let (Some(source), Some(target)) =
                (source.and_then(EidSlot::new), target.and_then(EidSlot::new))
            {
                rules.insert(class, Rule::Hit { source, target });
            }
        }
        for name in ["BoxRemoveMessage", "ShipRemoveMessage", "MonsterRemoveMessage"] {
            if let Some(class) = find(name)
                && let Some(slot) = eid_fields(class).first().copied().and_then(EidSlot::new)
            {
                rules.insert(class, Rule::Removed(slot));
            }
        }
        if let Some(class) = find("AttackStateMessage")
            && let Some(slot) = enum_field(class, "currentstate")
        {
            rules.insert(class, Rule::AttackState(slot));
        }
        if let Some(class) = find("AbortAttackMessage") {
            rules.insert(class, Rule::Aborted);
        }
        for name in [
            "DeathMessage",
            "DeathByUserMessage",
            "DeathByNPCMessage",
            "DeathByTowerMessage",
            "DeathByAreaEffectMessage",
        ] {
            if let Some(class) = find(name) {
                rules.insert(class, Rule::Death);
            }
        }
        if let Some(class) = find("MapChangeMessage") {
            rules.insert(class, Rule::MapChanged);
        }
        if let Some(class) = find("UserInitMessage") {
            rules.insert(class, Rule::Spawned);
        }
        if let Some(class) = find("DisconnectMessage") {
            rules.insert(class, Rule::Disconnect(enum_field(class, "reason")));
        }
        for (name, hints) in [
            ("EntityEffectMessage", &["entity"][..]),
            ("TimedEffectMessage", &["target"][..]),
            ("MapObjectEffectMessage", &["entity"][..]),
        ] {
            if let Some(class) = find(name)
                && let Some(effect) = enum_field(class, "effect")
                && let Some(target) = pick(&eid_fields(class), hints).and_then(EidSlot::new)
            {
                rules.insert(
                    class,
                    Rule::Effect {
                        effect,
                        target,
                        active: class.instance_field("active"),
                        millis: class.instance_field("milliseconds"),
                    },
                );
            }
        }
        if let Some(class) = find("MasterSkillTriggerMessage")
            && let Some(id) = class.instance_field("masterskillid")
        {
            rules.insert(
                class,
                Rule::SkillTriggered {
                    id,
                    remaining: class.instance_field("remainingduration"),
                },
            );
        }
        if let Some(class) = find("MapChangeDelayedMessage")
            && let Some(map) = class.instance_field("mapid")
            && let Some(delay) = class.instance_field("delay")
        {
            rules.insert(class, Rule::MapDelay { map, delay });
        }
        if let Some(class) = find("MapChangeAskMessage")
            && let Some(map) = class.instance_field("mapid")
        {
            rules.insert(
                class,
                Rule::MapAsk {
                    map,
                    kind: enum_field(class, "type"),
                },
            );
        }
        if let Some(class) = find("MapChangeCancelMessage") {
            rules.insert(class, Rule::MapCancel);
        }
        for (name, charged) in [
            ("MasterSkillChargeUpdateMessage", true),
            ("MasterSkillUntriggerUpdateMessage", false),
        ] {
            if let Some(class) = find(name)
                && let Some(id) = class.instance_field("masterskillid")
            {
                let charge = if charged {
                    class.instance_field("currentcharge")
                } else {
                    None
                };
                rules.insert(class, Rule::SkillCharge { id, charge });
            }
        }
        Self { rules }
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn translate(&self, message: Ptr) -> Option<Feedback> {
        let class = Class::of(message)?;
        Some(match self.rules.get(&class)? {
            Rule::Hit { source, target } => Feedback::Hit {
                source: source.read(message)?,
                target: target.read(message)?,
            },
            Rule::Removed(slot) => Feedback::Removed(slot.read(message)?),
            Rule::AttackState(slot) => match slot.read(message)? {
                "START" => Feedback::AttackStarted,
                _ => Feedback::AttackEnded,
            },
            Rule::Aborted => Feedback::AttackAborted,
            Rule::Death => Feedback::OwnDeath,
            Rule::MapChanged => Feedback::MapChanged,
            Rule::Spawned => Feedback::Spawned,
            Rule::Disconnect(slot) => Feedback::Disconnected(
                slot.as_ref()
                    .and_then(|s| s.read(message))
                    .unwrap_or("UNKNOWN")
                    .to_owned(),
            ),
            Rule::Effect {
                effect,
                target,
                active,
                millis,
            } => Feedback::Effect {
                name: effect.read(message)?.to_owned(),
                target: target.read(message)?,
                active: active
                    .and_then(|f| unsafe { get_bool(message, f) })
                    .unwrap_or(true),
                millis: millis
                    .and_then(|f| unsafe { get_i64(message, f) })
                    .and_then(|v| u64::try_from(v).ok()),
            },
            Rule::MapDelay { map, delay } => Feedback::MapDelay {
                map: unsafe { get_i64(message, *map) }? as i16,
                delay_ms: unsafe { get_i64(message, *delay) }?.max(0) as u64,
            },
            Rule::MapAsk { map, kind } => Feedback::MapAsk {
                map: unsafe { get_i64(message, *map) }? as i16,
                // EDGE_MAP_JUMP is the default (0) when the type is unknown.
                confirm: kind
                    .as_ref()
                    .and_then(|k| k.read(message))
                    .is_some_and(|k| k == "CONFIRM_MAP_JUMP"),
            },
            Rule::MapCancel => Feedback::MapCancel,
            Rule::SkillCharge { id, charge } => Feedback::SkillCharge {
                id: unsafe { get_i64(message, *id) }? as i32,
                charge: match charge {
                    Some(f) => unsafe { get_i64(message, *f) }?,
                    None => 0,
                },
            },
            Rule::SkillTriggered { id, remaining } => Feedback::SkillTriggered {
                id: unsafe { get_i64(message, *id) }? as i32,
                remaining_ms: remaining
                    .and_then(|f| unsafe { get_i64(message, f) })
                    .and_then(|v| u64::try_from(v).ok())
                    .unwrap_or(0),
            },
        })
    }
}

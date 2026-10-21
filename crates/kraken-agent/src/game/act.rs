use kraken_proto::Point;

use super::Game;
use crate::engine::world::{Action, Eid};
use crate::il2cpp::{Class, Field, Ptr, field_addr, inline_set, set_num, set_ref};

impl Game {
    fn construct(&self, class: Class) -> Result<Ptr, String> {
        let object = class.new_object();
        if object.is_null() {
            return Err(format!("{} allocation failed", class.name()));
        }
        if let Some(ctor) = class.method(".ctor", 0) {
            ctor.invoke(object, &mut [])?;
        }
        Ok(object)
    }

    pub(super) fn deliver(&mut self, network: Ptr, class: Class, message: Ptr) -> Result<(), String> {
        let send = self
            .sender(class)
            .ok_or_else(|| format!("no sender for {}", class.name()))?;
        send.invoke(network, &mut [message]).map(|_| ())
    }

    pub(super) fn message(&mut self, class: Option<Class>, label: &str) -> Result<(Class, Ptr), String> {
        let class = class.ok_or_else(|| format!("{label} unavailable"))?;
        Ok((class, self.construct(class)?))
    }

    pub(super) fn set_number(&mut self, object: Ptr, name: &'static str, value: f64) -> Result<(), String> {
        let class = Class::of(object).ok_or("null object")?;
        let field = self
            .field(class, name)
            .ok_or_else(|| format!("{}.{name} missing", class.name()))?;
        if unsafe { set_num(object, field, value) } {
            Ok(())
        } else {
            Err(format!("{}.{name} not numeric", class.name()))
        }
    }

    pub(super) fn write_eid(&mut self, object: Ptr, name: &'static str, eid: Eid) -> Result<(), String> {
        let class = Class::of(object).ok_or("null object")?;
        let field = self
            .field(class, name)
            .ok_or_else(|| format!("{}.{name} missing", class.name()))?;
        let ty = field.ty();
        let target = ty.class().ok_or("entity id type unresolved")?;
        let (id, instance) = super::eid_layout(target);
        let id = id.ok_or("entity id layout unresolved")?;
        if ty.is_value() {
            let base = unsafe { field_addr(object, field) };
            unsafe {
                inline_set(base, id, eid.id as f64);
                if let Some(instance) = instance {
                    inline_set(base, instance, eid.instance as f64);
                }
            }
        } else {
            let value = self.construct(target)?;
            unsafe {
                set_num(value, id, eid.id as f64);
                if let Some(instance) = instance {
                    set_num(value, instance, eid.instance as f64);
                }
                set_ref(object, field, value);
            }
        }
        Ok(())
    }

    fn write_position(&mut self, object: Ptr, field: Field, point: Point) -> Result<(), String> {
        let class = field.ty().class().ok_or("position type unresolved")?;
        let x = self.field(class, "x").ok_or("position.x missing")?;
        let y = self.field(class, "y").ok_or("position.y missing")?;
        if field.ty().is_value() {
            let base = unsafe { field_addr(object, field) };
            unsafe {
                inline_set(base, x, f64::from(point.x));
                inline_set(base, y, f64::from(point.y));
            }
        } else {
            let stub = self.construct(class)?;
            unsafe {
                set_num(stub, x, f64::from(point.x));
                set_num(stub, y, f64::from(point.y));
                set_ref(object, field, stub);
            }
        }
        Ok(())
    }

    pub(super) fn set_enum(&mut self, object: Ptr, name: &'static str, literal: &str) -> Result<(), String> {
        let class = Class::of(object).ok_or("null object")?;
        let field = self
            .field(class, name)
            .ok_or_else(|| format!("{}.{name} missing", class.name()))?;
        let value = self
            .enum_value(field.ty(), literal)
            .ok_or_else(|| format!("{literal} not in {}", field.ty().name()))?;
        unsafe { set_num(object, field, value as f64) };
        Ok(())
    }

    pub fn execute(&mut self, network: Ptr, action: Action) -> Result<(), String> {
        match action {
            Action::Move(point) => {
                let (class, message) = self.message(self.messages.moving, "MoveMessage")?;
                let field = self
                    .field(class, "position")
                    .ok_or("MoveMessage.position missing")?;
                self.write_position(message, field, point.rounded())?;
                self.deliver(network, class, message)
            }
            Action::Cannon { target, ammo } => {
                let (class, message) = self.message(self.messages.cannon, "CannonAttackMessage")?;
                self.set_number(message, "ammunitionid", f64::from(ammo))?;
                self.write_eid(message, "entityid", target)?;
                self.deliver(network, class, message)
            }
            Action::Harpoon { target, harpoon } => {
                let (class, message) = self.message(self.messages.harpoon, "HarpoonAttackMessage")?;
                self.set_number(message, "harpoonid", f64::from(harpoon))?;
                self.write_eid(message, "entityid", target)?;
                self.deliver(network, class, message)
            }
            Action::AbortAttack => {
                let (class, message) = self.message(self.messages.abort, "AbortAttackMessage")?;
                self.deliver(network, class, message)
            }
            Action::Repair(start) => {
                let (class, message) = self.message(self.messages.repair, "RepairMessage")?;
                self.set_enum(message, "type", if start { "START" } else { "STOP" })?;
                self.deliver(network, class, message)
            }
            Action::Respawn => {
                // The window's own button sends the same message and also closes it.
                if let Some(window) = self.death_window() {
                    return self.call(window, "OnStandardClicked");
                }
                let (class, message) = self.message(self.messages.respawn, "MapReturnToOldMapMessage")?;
                self.deliver(network, class, message)
            }
            Action::CloseDeathWindow => match self.death_window() {
                Some(window) => self.call(window, "RequestClose"),
                None => Ok(()),
            },
            Action::Select(target) => self.select(network, target),
            Action::AttackSelected => self.attack_selected(network),
            Action::UseItem { item, target } => self.use_item(network, item, target),
            Action::TriggerSkill(id) => self.trigger_skill(network, id),
            Action::EnterMap(map) => self.enter_map(network, map),
            Action::EdgeJump(map) => self.edge_jump(network, map),
            Action::ConfirmJump => self.confirm_jump(network),
            Action::Teleport(map) => self.teleport(network, map),
            Action::LoadTorpedo {
                preset,
                slot,
                torpedo,
            } => self.load_torpedo(network, preset, slot, torpedo),
            Action::Torpedo { target } => self.fire_torpedo(network, target),
            Action::Reconnect => {
                if let Some(method) = self.reconnect {
                    let mut force = true;
                    method.invoke(network, &mut [(&raw mut force).cast()]).map(|_| ())
                } else if let Some(method) = self.connect {
                    method.invoke(network, &mut []).map(|_| ())
                } else {
                    Err("reconnect unavailable".into())
                }
            }
        }
    }
}

//! Atlantis torpedoes. Submarines fire from launcher slots of the active
//! torpedo preset, not with a chosen ammunition:
//!
//! - load: `TorpedoSystem.RequestPresetModification(preset, slot, torpedo)`,
//!   what the torpedo belt does when a torpedo is picked for a slot;
//! - fire: `TorpedoAttackMessage{entityId}`, the target only;
//! - state: `TorpedoModel.ConfigurationsMessage` (active preset, its slots
//!   and what each holds) and `AreAllLaunchersOnCooldown()`;
//! - where: `MapData.isAtlantisMap()`.

use super::{GAME, Game, PROTOCOL};
use crate::engine::world::{Eid, TorpedoState};
use crate::il2cpp::{Class, Method, Ptr, boxed_bool};

#[derive(Clone, Copy, Default)]
pub struct TorpedoBindings {
    attack: Option<Class>,
    modify: Option<Method>,
    cooling: Option<Method>,
    atlantis: Option<Method>,
}

impl TorpedoBindings {
    pub fn resolve(registry: &crate::il2cpp::Registry, missing: &mut Vec<String>) -> Self {
        let system = registry.find("Seafight.Atlantis.TorpedoSystem", Some(GAME));
        let bindings = Self {
            attack: registry.find("TorpedoAttackMessage", Some(PROTOCOL)),
            modify: system.and_then(|c| c.method("RequestPresetModification", 3)),
            cooling: system.and_then(|c| c.method("AreAllLaunchersOnCooldown", 0)),
            atlantis: registry
                .find("Seafight.Map.MapData", Some(GAME))
                .and_then(|c| c.method("isAtlantisMap", 0)),
        };
        for (label, found) in [
            ("TorpedoAttackMessage", bindings.attack.is_some()),
            (
                "TorpedoSystem.RequestPresetModification",
                bindings.modify.is_some(),
            ),
            (
                "TorpedoSystem.AreAllLaunchersOnCooldown",
                bindings.cooling.is_some(),
            ),
            ("MapData.isAtlantisMap", bindings.atlantis.is_some()),
        ] {
            if !found {
                missing.push(label.into());
            }
        }
        bindings
    }
}

impl Game {
    pub fn is_atlantis(&mut self) -> bool {
        let Some(method) = self.torpedo_bindings.atlantis else {
            return false;
        };
        method
            .invoke(std::ptr::null_mut(), &mut [])
            .ok()
            .and_then(boxed_bool)
            .unwrap_or(false)
    }

    fn torpedo_system(&mut self, network: Ptr) -> Ptr {
        let handler = self.obj(network, "userinitmessagehandler");
        let info = self.obj(handler, "playerinfosystem");
        let attack = self.obj(info, "attacksystem");
        self.obj(attack, "torpedosystem")
    }

    /// The active preset's launcher slots and whether all of them are cooling.
    pub fn torpedo_state(&mut self, network: Ptr) -> TorpedoState {
        let system = self.torpedo_system(network);
        if system.is_null() {
            return TorpedoState::default();
        }
        let cooling = self
            .torpedo_bindings
            .cooling
            .and_then(|m| m.invoke(system, &mut []).ok())
            .and_then(boxed_bool)
            .unwrap_or(false);
        let model = self.obj(system, "torpedomodel");
        let message = self.obj(model, "configurationsmessage");
        let Some(active) = self.int(message, "activepreset") else {
            return TorpedoState {
                cooling,
                ..TorpedoState::default()
            };
        };
        let presets = self.obj(message, "presets");
        let mut slots = Vec::new();
        for preset in self.collections.list(presets) {
            let preset = preset.object();
            if self.int(preset, "presetid") != Some(active) {
                continue;
            }
            let configs = self.obj(preset, "configs");
            for launcher in self.collections.list(configs) {
                let launcher = launcher.object();
                if let (Some(slot), Some(torpedo)) =
                    (self.int(launcher, "slotid"), self.int(launcher, "torpedoid"))
                {
                    slots.push((slot as i16, torpedo as i16));
                }
            }
        }
        TorpedoState {
            preset: i32::try_from(active).ok(),
            slots,
            cooling,
        }
    }

    pub fn load_torpedo(&mut self, network: Ptr, preset: i32, slot: i16, torpedo: i16) -> Result<(), String> {
        let method = self
            .torpedo_bindings
            .modify
            .ok_or("RequestPresetModification unavailable")?;
        let system = self.torpedo_system(network);
        if system.is_null() {
            return Err("no torpedo system".into());
        }
        let (mut preset, mut slot, mut torpedo) = (preset, slot, torpedo);
        method
            .invoke(
                system,
                &mut [
                    (&raw mut preset).cast(),
                    (&raw mut slot).cast(),
                    (&raw mut torpedo).cast(),
                ],
            )
            .map(|_| ())
    }

    pub fn fire_torpedo(&mut self, network: Ptr, target: Eid) -> Result<(), String> {
        let (class, message) = self.message(self.torpedo_bindings.attack, "TorpedoAttackMessage")?;
        self.write_eid(message, "entityid", target)?;
        self.deliver(network, class, message)
    }
}

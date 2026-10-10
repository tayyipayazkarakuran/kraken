//! Bonus maps: which ones the hero owns, the wave they are at, how many waves
//! each has, and the game's own way in (`MapChangeRequestMessage`).

use std::collections::HashMap;

use kraken_proto::{Stock, StockKind};

use super::Game;
use crate::engine::world::BonusMap;
use crate::il2cpp::{Class, Field, Ptr, inline_i64};

#[derive(Clone, Copy, Default)]
pub struct BonusBindings {
    /// `MapChangeRequestMessage`, what the bonus map button sends.
    pub enter: Option<Class>,
    /// `DefaultItemConfigurationModel.Items` (static): every item's defaults,
    /// the bonus maps' wave counts among them.
    pub defaults: Option<Field>,
}

impl BonusBindings {
    pub fn resolve(registry: &crate::il2cpp::Registry, missing: &mut Vec<String>) -> Self {
        let enter = registry.find("MapChangeRequestMessage", Some(super::PROTOCOL));
        let defaults = registry
            .find(
                "Seafight.Storage.DefaultItemConfigurationModel",
                Some(super::GAME),
            )
            .and_then(|c| c.field("Items"));
        if enter.is_none() {
            missing.push("MapChangeRequestMessage".into());
        }
        Self { enter, defaults }
    }
}

impl Game {
    /// Wave counts per bonus map, from the item defaults; read once.
    fn bonus_waves(&mut self) -> HashMap<i16, u8> {
        if !self.bonus_waves.is_empty() {
            return self.bonus_waves.clone();
        }
        let Some(field) = self.bonus_bindings.defaults else {
            return HashMap::new();
        };
        let table = field.static_ptr();
        let mut out = HashMap::new();
        let mut bonus_type = None;
        for (key, stub) in self.collections.dict(table) {
            let Some(class) = key.ty.class() else { continue };
            let (Some(kind), Some(id)) = (self.field(class, "type"), self.field(class, "id")) else {
                continue;
            };
            let wanted = *bonus_type.get_or_insert_with(|| self.enum_value(kind.ty(), "BONUSMAPS"));
            let Some(wanted) = wanted else { break };
            if unsafe { inline_i64(key.at, kind) } != Some(wanted) {
                continue;
            }
            let Some(id) = (unsafe { inline_i64(key.at, id) }) else {
                continue;
            };
            let waves = self.int(stub.object(), "waveamount").unwrap_or(0);
            out.insert(id as i16, waves.clamp(0, 255) as u8);
        }
        if !out.is_empty() {
            tracing::info!(maps = out.len(), waves = ?out, "bonus map waves");
            self.bonus_waves.clone_from(&out);
        }
        out
    }

    /// Owned bonus maps with their current wave (`InventorySystem.BonusMapStatus`).
    pub fn bonus_maps(&mut self, inventory: Ptr, stock: &[Stock]) -> Vec<BonusMap> {
        let waves = self.bonus_waves();
        let status = self.obj(inventory, "bonusmapstatus");
        let mut current: HashMap<i16, u8> = HashMap::new();
        for (id, wave) in self.collections.dict(status) {
            if let (Some(id), Some(wave)) = (id.i64(), wave.i64()) {
                current.insert(id as i16, wave.clamp(0, 255) as u8);
            }
        }
        stock
            .iter()
            .filter(|s| s.kind == StockKind::BonusMap)
            .map(|s| BonusMap {
                id: s.id,
                amount: s.amount,
                wave: current.get(&s.id).copied().unwrap_or(0),
                waves: waves.get(&s.id).copied().unwrap_or(0),
            })
            .collect()
    }

    /// The bonus map button's own request.
    pub fn enter_map(&mut self, network: Ptr, map: i16) -> Result<(), String> {
        let (class, message) = self.message(self.bonus_bindings.enter, "MapChangeRequestMessage")?;
        self.set_number(message, "mapid", f64::from(map))?;
        self.deliver(network, class, message)
    }
}

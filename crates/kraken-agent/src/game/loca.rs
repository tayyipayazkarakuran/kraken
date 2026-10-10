use std::collections::HashMap;

use kraken_proto::names::{self, NameTable};

use super::Game;
use crate::il2cpp::{Class, read_string};

const LOCA_IMAGE: &str = "CakeBox";

pub struct Loca {
    pub language: String,
    pub names: NameTable,
}

impl Game {
    /// Reads the game's own localisation dictionary (the language the client
    /// runs in) and keeps the entity and item names.
    pub fn loca(&mut self) -> Option<Loca> {
        let class = self
            .registry
            .find("Bigpoint.Loca.LocaDictionary", Some(LOCA_IMAGE))?;
        let current = class.field("Current")?.static_ptr();
        let instance = Class::of(current)?;
        let language = self
            .field(instance, "language")
            .and_then(|f| {
                let value = unsafe { crate::il2cpp::get_i64(current, f) }?;
                self.enum_name(f.ty(), value)
            })
            .unwrap_or_default();
        let dictionary = self.obj(current, "dictionary");
        let entries = self.collections.dict(dictionary);
        if let Some((key, value)) = entries.first()
            && (key.ty.name() != "System.String" || value.ty.name() != "System.String")
        {
            tracing::warn!(key = %key.ty.name(), value = %value.ty.name(), "unexpected localisation layout");
            return None;
        }
        let mut texts = HashMap::new();
        for (key, value) in entries {
            let key = read_string(key.object());
            if names::relevant(&key) {
                texts.insert(key, read_string(value.object()));
            }
        }
        let names = names::build(&texts);
        tracing::info!(%language, texts = texts.len(), names = names.len(), "localisation read");
        (!names.is_empty()).then_some(Loca { language, names })
    }
}

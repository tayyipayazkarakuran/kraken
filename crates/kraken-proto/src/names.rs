use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// Localisation key prefixes the game uses for entity and item names; the
/// numeric suffix is the type id the game reports at runtime.
const PREFIXES: [(&str, NameKind); 9] = [
    ("seafight.npcnames.npc.", NameKind::Npc),
    ("seafight.labels.Monsters.", NameKind::Monster),
    ("seafight.labels.Boxes.", NameKind::Box),
    ("seafight.labels.BoxType.", NameKind::Box),
    ("seafight.labels.Ammunition.", NameKind::Ammo),
    ("seafight.labels.Harpoons.", NameKind::Harpoon),
    ("seafight.labels.ActionItems.", NameKind::ActionItem),
    ("seafight.labels.Currency.", NameKind::Currency),
    ("seafight.labels.Maps.", NameKind::Map),
];

const MAX_REF_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NameKind {
    Npc,
    Monster,
    Box,
    Ammo,
    Harpoon,
    ActionItem,
    Currency,
    Map,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameTable {
    pub npcs: BTreeMap<i32, String>,
    pub monsters: BTreeMap<i32, String>,
    pub boxes: BTreeMap<i32, String>,
    pub ammo: BTreeMap<i32, String>,
    pub harpoons: BTreeMap<i32, String>,
    #[serde(default)]
    pub items: BTreeMap<i32, String>,
    #[serde(default)]
    pub currencies: BTreeMap<i32, String>,
    #[serde(default)]
    pub maps: BTreeMap<i32, String>,
}

impl NameTable {
    pub fn table(&self, kind: NameKind) -> &BTreeMap<i32, String> {
        match kind {
            NameKind::Npc => &self.npcs,
            NameKind::Monster => &self.monsters,
            NameKind::Box => &self.boxes,
            NameKind::Ammo => &self.ammo,
            NameKind::Harpoon => &self.harpoons,
            NameKind::ActionItem => &self.items,
            NameKind::Currency => &self.currencies,
            NameKind::Map => &self.maps,
        }
    }

    fn table_mut(&mut self, kind: NameKind) -> &mut BTreeMap<i32, String> {
        match kind {
            NameKind::Npc => &mut self.npcs,
            NameKind::Monster => &mut self.monsters,
            NameKind::Box => &mut self.boxes,
            NameKind::Ammo => &mut self.ammo,
            NameKind::Harpoon => &mut self.harpoons,
            NameKind::ActionItem => &mut self.items,
            NameKind::Currency => &mut self.currencies,
            NameKind::Map => &mut self.maps,
        }
    }

    pub fn insert(&mut self, kind: NameKind, id: i32, name: String) {
        self.table_mut(kind).insert(id, name);
    }

    pub fn get(&self, kind: NameKind, id: i32) -> Option<&str> {
        self.table(kind).get(&id).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.npcs.len()
            + self.monsters.len()
            + self.boxes.len()
            + self.ammo.len()
            + self.harpoons.len()
            + self.items.len()
            + self.currencies.len()
            + self.maps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Keys worth keeping while scanning a full localisation dictionary: every
/// name key plus everything a `<ref>` inside one could point at.
pub fn relevant(key: &str) -> bool {
    key.starts_with("seafight.npcnames.") || key.starts_with("seafight.labels.")
}

pub fn classify(key: &str) -> Option<(NameKind, i32)> {
    PREFIXES.iter().find_map(|(prefix, kind)| {
        let id = key.strip_prefix(prefix)?.parse().ok()?;
        Some((*kind, id))
    })
}

/// Builds the name tables from raw `TextId -> Text` pairs, following
/// `<ref>key</ref>` indirections and dropping inline markup.
pub fn build(texts: &HashMap<String, String>) -> NameTable {
    let mut table = NameTable::default();
    let mut keys: Vec<&String> = texts.keys().collect();
    keys.sort();
    for key in keys {
        let Some((kind, id)) = classify(key) else { continue };
        let Some(text) = resolve(texts, key) else { continue };
        let name = clean(&text);
        if !name.chars().any(char::is_alphanumeric) {
            continue;
        }
        let slot = table.table_mut(kind);
        // `Boxes.<id>` is the primary key; `BoxType.<id>` only fills gaps.
        if key.contains(".BoxType.") && slot.contains_key(&id) {
            continue;
        }
        slot.insert(id, name);
    }
    table
}

fn resolve(texts: &HashMap<String, String>, key: &str) -> Option<String> {
    let mut text = texts.get(key)?.clone();
    for _ in 0..MAX_REF_DEPTH {
        let Some(start) = text.find("<ref>") else {
            return Some(text);
        };
        let end = text[start..].find("</ref>")? + start;
        let target = text[start + 5..end].trim();
        let replacement = texts.get(target)?;
        text = format!("{}{}{}", &text[..start], replacement, &text[end + 6..]);
    }
    None
}

fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn builds_tables_and_resolves_refs() {
        let table = build(&texts(&[
            ("seafight.npcnames.npc.1001", "Admiral Jack"),
            ("seafight.labels.Monsters.61", "Kraken"),
            (
                "seafight.labels.Monsters.100",
                "<ref>seafight.labels.Monsters.61</ref>",
            ),
            ("seafight.labels.Boxes.1", "Pırıltı"),
            ("seafight.labels.BoxType.1", "<ref>seafight.labels.Boxes.1</ref>"),
            ("seafight.labels.BoxType.9", "Sandık"),
            (
                "seafight.labels.Ammunition.1",
                "<color=#fff>Zincirli</color>  cephane",
            ),
            ("seafight.labels.Harpoons.18", "Bakır zıpkın"),
            ("seafight.labels.ActionItems.29", "Kartal Gözü"),
            ("seafight.labels.Loop.1", "<ref>seafight.labels.Loop.1</ref>"),
        ]));
        assert_eq!(table.get(NameKind::Npc, 1001), Some("Admiral Jack"));
        assert_eq!(table.get(NameKind::Monster, 100), Some("Kraken"));
        assert_eq!(table.get(NameKind::Box, 1), Some("Pırıltı"));
        assert_eq!(table.get(NameKind::Box, 9), Some("Sandık"));
        assert_eq!(table.get(NameKind::Ammo, 1), Some("Zincirli cephane"));
        assert_eq!(table.get(NameKind::Harpoon, 18), Some("Bakır zıpkın"));
        assert_eq!(table.get(NameKind::ActionItem, 29), Some("Kartal Gözü"));
        assert_eq!(table.len(), 8);
    }

    #[test]
    fn ref_loops_are_dropped() {
        let table = build(&texts(&[(
            "seafight.npcnames.npc.1",
            "<ref>seafight.npcnames.npc.1</ref>",
        )]));
        assert!(table.is_empty());
    }
}

use std::cmp::Ordering;

use kraken_proto::names::NameKind;
use kraken_proto::{CatalogEntry, EntityKind, ItemEntry, Stock, StockKind};

use super::State;
use crate::gamedata::LANGUAGES;

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: i32,
    pub name: String,
    pub nearby: bool,
}

pub fn kind_of(kind: EntityKind) -> Option<NameKind> {
    match kind {
        EntityKind::Npc => Some(NameKind::Npc),
        EntityKind::Monster => Some(NameKind::Monster),
        EntityKind::Box => Some(NameKind::Box),
        _ => None,
    }
}

/// Name in the interface language, then the other language from the game
/// files, then whatever the agent reported.
pub fn lookup(state: &State, kind: NameKind, id: i32) -> Option<String> {
    let preferred = std::iter::once(state.profile.locale.as_str())
        .chain(LANGUAGES.iter().copied().filter(|l| *l != state.profile.locale));
    preferred
        .filter_map(|lang| state.names.get(lang))
        .find_map(|language| language.names.get(kind, id))
        .map(str::to_owned)
}

pub fn name(state: &State, kind: NameKind, id: i32, fallback: &str) -> String {
    lookup(state, kind, id).unwrap_or_else(|| {
        if fallback.is_empty() {
            format!("#{id}")
        } else {
            fallback.to_owned()
        }
    })
}

pub fn nearby(state: &State, kind: NameKind) -> Vec<i32> {
    state.live().map_or_else(Vec::new, |t| match kind {
        NameKind::Npc => t.nearby.npcs.clone(),
        NameKind::Monster => t.nearby.monsters.clone(),
        NameKind::Box => t.nearby.boxes.clone(),
        NameKind::Ammo | NameKind::Harpoon | NameKind::ActionItem | NameKind::Currency | NameKind::Map => {
            Vec::new()
        }
    })
}

/// Case-insensitive key that places the Turkish letters after their base
/// letter (c < ç < d, g < ğ < h, ı < i, o < ö < p, s < ş < t, u < ü < v).
fn collation(text: &str) -> Vec<u32> {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| *c != '\u{307}')
        .map(|c| {
            let (base, step) = match c {
                'ç' => ('c', 1),
                'ğ' => ('g', 1),
                'ı' => ('h', 1),
                'ö' => ('o', 1),
                'ş' => ('s', 1),
                'ü' => ('u', 1),
                other => (other, 0),
            };
            base as u32 * 2 + step
        })
        .collect()
}

fn alphabetical(a: &str, b: &str) -> Ordering {
    collation(a).cmp(&collation(b)).then_with(|| a.cmp(b))
}

/// Nearby entries first, then everything else alphabetically.
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.nearby
            .cmp(&a.nearby)
            .then_with(|| alphabetical(&a.name, &b.name))
            .then(a.id.cmp(&b.id))
    });
}

/// Every known type of `kind`: the game files' full list plus anything the
/// agent has seen that the files do not name.
pub fn entries(state: &State, kind: NameKind, seen: &[CatalogEntry]) -> Vec<Entry> {
    let close = nearby(state, kind);
    let mut ids: Vec<i32> = state
        .names
        .values()
        .flat_map(|l| l.names.table(kind).keys().copied())
        .chain(seen.iter().map(|e| e.type_id))
        .chain(close.iter().copied())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out: Vec<Entry> = ids
        .into_iter()
        .map(|id| {
            let fallback = seen
                .iter()
                .find(|e| e.type_id == id)
                .map_or("", |e| e.name.as_str());
            Entry {
                id,
                name: name(state, kind, id, fallback),
                nearby: close.contains(&id),
            }
        })
        .collect();
    sort(&mut out);
    out
}

/// The name table for an inventory kind; wallet kinds share the currency
/// names (points and tokens fall back to `#id`).
pub fn stock_names(kind: StockKind) -> NameKind {
    match kind {
        StockKind::Ammo => NameKind::Ammo,
        StockKind::Harpoon => NameKind::Harpoon,
        StockKind::Currency | StockKind::Points | StockKind::Token => NameKind::Currency,
        StockKind::BonusMap => NameKind::Map,
    }
}

/// Ammunition or harpoons: stocked items first, then alphabetical.
pub fn items(state: &State, stock_kind: StockKind, seen: &[ItemEntry], stock: &[Stock]) -> Vec<ItemEntry> {
    let kind = stock_names(stock_kind);
    let mut ids: Vec<i16> = state
        .names
        .values()
        .flat_map(|l| {
            l.names
                .table(kind)
                .keys()
                .filter_map(|id| i16::try_from(*id).ok())
        })
        .chain(seen.iter().map(|i| i.id))
        .chain(stock.iter().filter(|s| s.kind == stock_kind).map(|s| s.id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let stocked = |id: i16| {
        stock
            .iter()
            .any(|s| s.kind == stock_kind && s.id == id && s.amount > 0)
    };
    let mut out: Vec<ItemEntry> = ids
        .into_iter()
        .map(|id| {
            let fallback = seen.iter().find(|i| i.id == id).map_or("", |i| i.name.as_str());
            ItemEntry {
                id,
                name: name(state, kind, i32::from(id), fallback),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        stocked(b.id)
            .cmp(&stocked(a.id))
            .then_with(|| alphabetical(&a.name, &b.name))
    });
    out
}

/// Display order for a configured target list without reordering it.
pub fn order(state: &State, kind: NameKind, targets: &[(i32, String)]) -> Vec<(usize, Entry)> {
    let close = nearby(state, kind);
    let mut rows: Vec<(usize, Entry)> = targets
        .iter()
        .enumerate()
        .map(|(index, (id, fallback))| {
            (
                index,
                Entry {
                    id: *id,
                    name: name(state, kind, *id, fallback),
                    nearby: close.contains(id),
                },
            )
        })
        .collect();
    // Stable, alphabetical: rows must not jump while the user clicks them.
    rows.sort_by(|(_, a), (_, b)| alphabetical(&a.name, &b.name).then(a.id.cmp(&b.id)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i32, name: &str, nearby: bool) -> Entry {
        Entry {
            id,
            name: name.into(),
            nearby,
        }
    }

    #[test]
    fn nearby_first_then_alphabetical() {
        let mut list = vec![
            entry(1, "zebra", false),
            entry(2, "Çapa", true),
            entry(3, "ayı", false),
            entry(4, "Balina", true),
        ];
        sort(&mut list);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Balina", "Çapa", "ayı", "zebra"]);
    }

    #[test]
    fn turkish_letters_follow_their_base() {
        let mut words = vec!["dalga", "çapa", "cam", "ılık", "irmik", "hasat"];
        words.sort_by(|a, b| alphabetical(a, b));
        assert_eq!(words, ["cam", "çapa", "dalga", "hasat", "ılık", "irmik"]);
    }
}

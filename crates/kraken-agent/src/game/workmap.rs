//! Work maps: the world map catalogue (which map sits where) and the game's
//! own ways from one map to the next.
//!
//! - Edge: at a map edge the server sends `MapChangeAskMessage{mapId,type}`;
//!   the map switch button answers an `EDGE_MAP_JUMP` offer with
//!   `MapChangeEdgeMessage{mapId}` (`MapSwitchBehaviour.OnMapSwitchButtonClick`)
//!   and a `CONFIRM_MAP_JUMP` one with `ConfirmMapChangeMessage(type)`.
//! - Teleport: the world map's button sends `TeleportMessage{mapId}`
//!   (`WorldMapDataSystem.RequestTeleportation`).
//! - Catalogue: `ThemeBasedWorldMapData.WorldMapDataList`, a list per world
//!   with nine rows of `MapCellData` each. It is a ScriptableObject, found
//!   with `Resources.FindObjectsOfTypeAll` once the game has loaded it, and
//!   cached on disk for later sessions.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{GAME, Game, PROTOCOL};
use crate::engine::world::MapCell;
use crate::il2cpp::{Class, Method, Ptr, array_data, array_len};

/// How often to look for the catalogue while the game has not loaded it.
const ATLAS_EVERY: Duration = Duration::from_secs(10);
const ROWS: usize = 9;

#[derive(Clone, Copy, Default)]
pub struct WorkmapBindings {
    pub edge: Option<Class>,
    pub confirm: Option<Class>,
    pub teleport: Option<Class>,
    world_data: Option<Class>,
    find_all: Option<Method>,
}

impl WorkmapBindings {
    pub fn resolve(registry: &crate::il2cpp::Registry, missing: &mut Vec<String>) -> Self {
        let message = |name: &str| registry.find(name, Some(PROTOCOL));
        let bindings = Self {
            edge: message("MapChangeEdgeMessage"),
            confirm: message("ConfirmMapChangeMessage"),
            teleport: message("TeleportMessage"),
            world_data: registry.find("Seafight.WorldMap.ThemeBasedWorldMapData", Some(GAME)),
            find_all: registry
                .find("UnityEngine.Resources", Some("UnityEngine.CoreModule"))
                .and_then(|c| {
                    c.method_where("FindObjectsOfTypeAll", |m| {
                        m.param_count() == 1 && m.param(0).name() == "System.Type"
                    })
                }),
        };
        for (label, found) in [
            ("MapChangeEdgeMessage", bindings.edge.is_some()),
            ("ConfirmMapChangeMessage", bindings.confirm.is_some()),
            ("TeleportMessage", bindings.teleport.is_some()),
            ("ThemeBasedWorldMapData", bindings.world_data.is_some()),
            ("Resources.FindObjectsOfTypeAll", bindings.find_all.is_some()),
        ] {
            if !found {
                missing.push(label.into());
            }
        }
        bindings
    }
}

#[derive(Default)]
pub struct AtlasCache {
    cells: Vec<MapCell>,
    loaded_file: bool,
    /// Read from the running game (no more looking).
    from_game: bool,
    last_try: Option<Instant>,
}

fn data_path(file: &str) -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join("Kraken").join("gamedata").join(file))
}

fn atlas_path() -> Option<PathBuf> {
    data_path("atlas.tsv")
}

/// Map edges learnt in earlier sessions: (map, edge, the map it leads to).
pub fn load_edges() -> Vec<(i16, usize, i16)> {
    let Some(text) = data_path("edges.tsv").and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t').map(str::trim);
            Some((
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
                parts.next()?.parse().ok()?,
            ))
        })
        .collect()
}

pub fn save_edges(edges: &[(i16, usize, i16)]) {
    let Some(path) = data_path("edges.tsv") else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut lines: Vec<String> = edges.iter().map(|(f, e, t)| format!("{f}\t{e}\t{t}\n")).collect();
    lines.sort();
    if let Err(error) = std::fs::write(&path, lines.concat()) {
        tracing::warn!(%error, "edges not written");
    }
}

fn load_file() -> Vec<MapCell> {
    let Some(text) = atlas_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let mut next = || parts.next()?.trim().parse::<i32>().ok();
            Some(MapCell {
                id: i16::try_from(next()?).ok()?,
                world: u8::try_from(next()?).ok()?,
                row: u8::try_from(next()?).ok()?,
                col: u8::try_from(next()?).ok()?,
                safe_haven: next()? != 0,
                min_level: next()?,
            })
        })
        .collect()
}

fn save_file(cells: &[MapCell]) {
    let Some(path) = atlas_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = cells
        .iter()
        .map(|c| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                c.id,
                c.world,
                c.row,
                c.col,
                u8::from(c.safe_haven),
                c.min_level
            )
        })
        .collect();
    if let Err(error) = std::fs::write(&path, text) {
        tracing::warn!(%error, "atlas cache not written");
    }
}

impl Game {
    /// The world map catalogue: from the game once it has loaded it, until
    /// then from the cache of an earlier session.
    pub fn atlas(&mut self) -> Vec<MapCell> {
        let cache = &mut self.atlas;
        if !cache.loaded_file {
            cache.loaded_file = true;
            cache.cells = load_file();
            if !cache.cells.is_empty() {
                tracing::info!(maps = cache.cells.len(), "atlas from cache");
            }
        }
        // The game's own data replaces the cache (it may be from an older
        // client) as soon as the game has it loaded.
        let due = cache.last_try.is_none_or(|t| t.elapsed() >= ATLAS_EVERY);
        if !cache.from_game && due {
            cache.last_try = Some(Instant::now());
            if let Some(cells) = self.read_atlas() {
                tracing::info!(maps = cells.len(), "atlas read from the game");
                if cells != self.atlas.cells {
                    save_file(&cells);
                }
                self.atlas.cells = cells;
                self.atlas.from_game = true;
            }
        }
        self.atlas.cells.clone()
    }

    fn read_atlas(&mut self) -> Option<Vec<MapCell>> {
        let bindings = self.workmap_bindings;
        let (class, find_all) = (bindings.world_data?, bindings.find_all?);
        let type_object = class.type_object();
        if type_object.is_null() {
            return None;
        }
        let array = find_all
            .invoke(std::ptr::null_mut(), &mut [type_object])
            .ok()
            .filter(|a| !a.is_null())?;
        let len = array_len(array);
        let data = array_data(array).cast::<Ptr>();
        let mut cells = Vec::new();
        for index in 0..len {
            let object = unsafe { *data.add(index) };
            if object.is_null() {
                continue;
            }
            let worlds = self.obj(object, "worldmapdatalist");
            for (world, data) in self.collections.list(worlds).into_iter().enumerate() {
                let data = data.object();
                const ROW_FIELDS: [&str; ROWS] = [
                    "mapcelldatarow1",
                    "mapcelldatarow2",
                    "mapcelldatarow3",
                    "mapcelldatarow4",
                    "mapcelldatarow5",
                    "mapcelldatarow6",
                    "mapcelldatarow7",
                    "mapcelldatarow8",
                    "mapcelldatarow9",
                ];
                for (row, name) in ROW_FIELDS.into_iter().enumerate() {
                    let list = self.obj(data, name);
                    for (col, cell) in self.collections.list(list).into_iter().enumerate() {
                        let cell = cell.object();
                        let Some(kind) = Class::of(cell).and_then(|c| self.field(c, "type")) else {
                            continue;
                        };
                        let value = self.int(cell, "type").unwrap_or(0);
                        let kind = self.enum_name(kind.ty(), value).unwrap_or_default();
                        let id = self.int(cell, "mapid").unwrap_or(0);
                        if kind == "Empty" || id <= 0 {
                            continue;
                        }
                        cells.push(MapCell {
                            id: id as i16,
                            world: world.min(255) as u8,
                            row: row as u8,
                            col: col.min(255) as u8,
                            safe_haven: kind == "SafeHaven",
                            min_level: self.int(cell, "minlevel").unwrap_or(0) as i32,
                        });
                    }
                }
            }
            if !cells.is_empty() {
                break;
            }
        }
        (!cells.is_empty()).then_some(cells)
    }

    pub fn edge_jump(&mut self, network: Ptr, map: i16) -> Result<(), String> {
        let (class, message) = self.message(self.workmap_bindings.edge, "MapChangeEdgeMessage")?;
        self.set_number(message, "mapid", f64::from(map))?;
        self.deliver(network, class, message)
    }

    pub fn confirm_jump(&mut self, network: Ptr) -> Result<(), String> {
        let (class, message) = self.message(self.workmap_bindings.confirm, "ConfirmMapChangeMessage")?;
        self.set_enum(message, "type", "CONFIRM_MAP_JUMP")?;
        self.deliver(network, class, message)
    }

    pub fn teleport(&mut self, network: Ptr, map: i16) -> Result<(), String> {
        let (class, message) = self.message(self.workmap_bindings.teleport, "TeleportMessage")?;
        self.set_number(message, "mapid", f64::from(map))?;
        self.deliver(network, class, message)
    }
}

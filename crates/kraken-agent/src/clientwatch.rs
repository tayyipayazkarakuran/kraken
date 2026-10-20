//! Game updates: when the client's files change, write a fresh type dump and
//! compare it with the one from the previous client, so an update that
//! touches what Kraken uses shows up in the log at once (and the report is
//! there to read).
//!
//! Kraken finds every class, field and method by name at run time, never by
//! address, so most updates need nothing; this is the early warning for the
//! ones that rename or remove something.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::il2cpp::Registry;

/// The client build: size and change time of its code and metadata.
fn fingerprint(dir: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for file in [
        dir.join("GameAssembly.dll"),
        dir.join("Seafight_Data")
            .join("il2cpp_data")
            .join("Metadata")
            .join("global-metadata.dat"),
    ] {
        let meta = std::fs::metadata(&file).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        parts.push(format!("{}-{modified}", meta.len()));
    }
    Some(parts.join("_"))
}

fn client_dir() -> Option<PathBuf> {
    let module = crate::il2cpp::game_module()?;
    let mut buffer = [0u16; 1024];
    let len = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            module,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    let path = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
    path.parent().map(Path::to_path_buf)
}

/// Class name -> its member lines, without addresses (those move with every
/// build and say nothing).
fn members(dump: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut class = String::new();
    for line in dump.lines() {
        if let Some(name) = line.strip_prefix("class ") {
            class = name.to_owned();
            out.entry(class.clone()).or_default();
        } else if line.starts_with("  ") && !class.is_empty() {
            let line = line.split(" rva=").next().unwrap_or(line).trim();
            out.entry(class.clone()).or_default().insert(line.to_owned());
        }
    }
    out
}

/// What changed between two dumps, and which of the changes touch names the
/// bot depends on.
fn compare(old: &str, new: &str, watched: &[&str]) -> (String, Vec<String>) {
    let (old, new) = (members(old), members(new));
    let mut report = String::new();
    let mut hits = Vec::new();
    let touches = |text: &str| {
        let lower = text.to_lowercase();
        watched.iter().any(|w| lower.contains(&w.to_lowercase()))
    };
    for name in old.keys().filter(|k| !new.contains_key(*k)) {
        let _ = writeln!(report, "- class {name}");
        if touches(name) {
            hits.push(format!("class removed: {name}"));
        }
    }
    for name in new.keys().filter(|k| !old.contains_key(*k)) {
        let _ = writeln!(report, "+ class {name}");
    }
    for (name, now) in &new {
        let Some(before) = old.get(name) else { continue };
        let gone: Vec<_> = before.difference(now).collect();
        let added: Vec<_> = now.difference(before).collect();
        if gone.is_empty() && added.is_empty() {
            continue;
        }
        let _ = writeln!(report, "~ class {name}");
        for line in &gone {
            let _ = writeln!(report, "    - {line}");
            if touches(name) {
                hits.push(format!("{name}: {line}"));
            }
        }
        for line in &added {
            let _ = writeln!(report, "    + {line}");
        }
    }
    (report, hits)
}

/// The classes the bot reads or calls (by the names it looks them up with).
const WATCHED: &[&str] = &[
    "NetworkManager",
    "SeafightDispatcher",
    "MapData",
    "MapView",
    "MapUtils",
    "MovementBehaviour",
    "GameActorModel",
    "GameActorLayer",
    "GameActorBehaviour",
    "UserInitMessageHandler",
    "PlayerInfoSystem",
    "AttackSystem",
    "InventorySystem",
    "SkillTreeSystem",
    "MasterSkillDataStub",
    "DefaultItemConfigurationModel",
    "DefaultMasterSkill",
    "ActionItemUtils",
    "ActionMenuPanel",
    "DeathWindowBehaviour",
    "UISystem",
    "LocaDictionary",
    "ThemeBasedWorldMapData",
    "MapCellData",
    "WorldMapData",
    "MoveMessage",
    "CannonAttackMessage",
    "HarpoonAttackMessage",
    "AbortAttackMessage",
    "RepairMessage",
    "MapReturnToOldMapMessage",
    "MapChangeRequestMessage",
    "MapChangeEdgeMessage",
    "ConfirmMapChangeMessage",
    "MapChangeAskMessage",
    "MapChangeDelayedMessage",
    "TeleportMessage",
    "DisplayHitMessage",
    "EntityEffectMessage",
    "TimedEffectMessage",
    "MasterSkillTriggerMessage",
    "DisconnectMessage",
    "UserInitMessage",
];

/// Runs once per agent start, on the boot thread (the dump calls into IL2CPP).
pub fn check(registry: &Registry) {
    let Some(dir) = client_dir() else { return };
    let Some(print) = fingerprint(&dir) else { return };
    let state = crate::dump::data_dir().join("clients");
    let stamp = state.join("current.txt");
    let previous = std::fs::read_to_string(&stamp).unwrap_or_default();
    let mut lines = previous.lines();
    let (last_print, last_dump) = (lines.next().unwrap_or(""), lines.next().unwrap_or(""));
    if last_print == print {
        tracing::info!(client = %print, "game client unchanged");
        return;
    }
    tracing::info!(client = %print, previous = %last_print, "game client changed; dumping types");
    let dump = match crate::dump::write(registry) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "type dump failed");
            return;
        }
    };
    let _ = std::fs::create_dir_all(&state);
    if !last_dump.is_empty()
        && let (Ok(old), Ok(new)) = (std::fs::read_to_string(last_dump), std::fs::read_to_string(&dump))
    {
        let (report, hits) = compare(&old, &new, WATCHED);
        let path = state.join(format!("changes-{print}.txt"));
        let _ = std::fs::write(
            &path,
            format!(
                "# {last_print} -> {print}\n# {last_dump}\n# {}\n{report}",
                dump.display()
            ),
        );
        if hits.is_empty() {
            tracing::info!(report = %path.display(), changed = report.lines().count(), "client update: nothing the bot uses changed");
        } else {
            tracing::warn!(report = %path.display(), ?hits, "client update touches what the bot uses");
        }
    }
    let _ = std::fs::write(&stamp, format!("{print}\n{}\n", dump.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "class A.MoveMessage\n  position : P @8\n  Write(X) rva=0x10\nclass B\n  x : I @8\n";
    const NEW: &str = "class A.MoveMessage\n  Write(X) rva=0x20\nclass B\n  x : I @8\n  y : I @12\nclass C\n";

    #[test]
    fn addresses_are_not_changes() {
        let (report, hits) = compare(OLD, OLD.replace("0x10", "0x99").as_str(), WATCHED);
        assert!(report.is_empty() && hits.is_empty());
    }

    #[test]
    fn removed_members_of_watched_classes_are_flagged() {
        let (report, hits) = compare(OLD, NEW, WATCHED);
        assert!(report.contains("- position : P @8"));
        assert!(report.contains("+ y : I @12"));
        assert!(report.contains("+ class C"));
        assert_eq!(hits, vec!["A.MoveMessage: position : P @8".to_owned()]);
    }
}

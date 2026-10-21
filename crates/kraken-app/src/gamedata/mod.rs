//! Entity and item names straight from the installed game's localisation
//! bundles. The bundle file names carry a content hash, so a game update is
//! picked up automatically and parsed once into a small cache.

mod reader;
mod serialized;
mod unityfs;

use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use kraken_proto::names::{self, NameKind, NameTable};

use crate::profile::data_dir;

pub const LANGUAGES: [&str; 2] = ["tr", "en"];
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CACHE_VERSION: u32 = 6;
const DEFAULT_CDN: &str =
    "https://alicdn-prod.seafight.com/unityclient/SeafightWindows/Seafight_Data/StreamingAssets/aa";

#[derive(Clone, Debug)]
pub struct Language {
    pub hash: String,
    pub texts: usize,
    pub names: NameTable,
}

#[derive(Debug)]
pub enum Update {
    Loaded(String, Box<Language>),
    Failed(String, String),
}

pub fn spawn(ctx: eframe::egui::Context) -> Receiver<Update> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("kraken-gamedata".into())
        .spawn(move || {
            let client = client_dir();
            for lang in LANGUAGES {
                let update = match client
                    .as_deref()
                    .ok_or_else(|| "Seafight client not found".to_owned())
                {
                    Ok(client) => match load(client, lang) {
                        Ok(language) => Update::Loaded(lang.to_owned(), Box::new(language)),
                        Err(error) => Update::Failed(lang.to_owned(), error),
                    },
                    Err(error) => Update::Failed(lang.to_owned(), error),
                };
                if let Update::Failed(lang, error) = &update {
                    tracing::warn!(%lang, %error, "game names unavailable");
                }
                let _ = tx.send(update);
                ctx.request_repaint();
            }
        })
        .ok();
    rx
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// The newest installed launcher version's client folder.
fn client_dir() -> Option<PathBuf> {
    let root = local_app_data()?.join("Seafight");
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("app-"))
        .map(|e| e.path().join("launchpad").join("client"))
        .filter(|p| p.join("Seafight_Data").is_dir())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

fn bundle_name(client: &Path, lang: &str) -> Result<String, String> {
    let catalog = client.join("Seafight_Data/StreamingAssets/aa/catalog.bin");
    let bytes = std::fs::read(&catalog).map_err(|e| format!("{}: {e}", catalog.display()))?;
    let prefix = format!("loca_{lang}_assets_all_");
    let needle = prefix.as_bytes();
    bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .find_map(|(at, _)| {
            let rest = bytes.get(at + needle.len()..at + needle.len() + 39)?;
            let (hash, ext) = rest.split_at(32);
            (hash.iter().all(u8::is_ascii_hexdigit) && ext == b".bundle")
                .then(|| format!("{prefix}{}.bundle", String::from_utf8_lossy(hash)))
        })
        .ok_or_else(|| format!("no {lang} localisation bundle in the catalog"))
}

fn hash_of(bundle: &str) -> &str {
    bundle
        .trim_end_matches(".bundle")
        .rsplit('_')
        .next()
        .unwrap_or_default()
}

fn cache_dir() -> PathBuf {
    data_dir().join("gamedata")
}

fn load(client: &Path, lang: &str) -> Result<Language, String> {
    let bundle = bundle_name(client, lang)?;
    let hash = hash_of(&bundle).to_owned();
    let cache = cache_dir().join(format!("names-{lang}-{hash}.tsv"));
    if let Some(language) = read_cache(&cache, &hash) {
        return Ok(language);
    }
    let bytes = bundle_bytes(&bundle, &hash)?;
    let texts = texts(&bytes)?;
    let language = Language {
        hash,
        texts: texts.len(),
        names: names::build(&texts),
    };
    if language.names.is_empty() {
        return Err(format!("{bundle} holds no entity names"));
    }
    write_cache(&cache, lang, &language);
    tracing::info!(%lang, texts = language.texts, names = language.names.len(), "game names parsed");
    Ok(language)
}

/// The bundle as Unity cached it, or else a fresh copy from the game's CDN.
fn bundle_bytes(bundle: &str, hash: &str) -> Result<Vec<u8>, String> {
    if let Some(cached) = unity_cache(hash) {
        return std::fs::read(cached).map_err(|e| e.to_string());
    }
    let target = cache_dir().join(bundle);
    if let Ok(bytes) = std::fs::read(&target) {
        return Ok(bytes);
    }
    std::fs::create_dir_all(cache_dir()).map_err(|e| e.to_string())?;
    let partial = target.with_extension("part");
    let url = format!("{}/StandaloneWindows/{bundle}", cdn_root());
    let status = std::process::Command::new("curl.exe")
        .args(["-sSfL", "--max-time", "120", "-o"])
        .arg(&partial)
        .arg(&url)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("download of {bundle} failed ({status})"));
    }
    std::fs::rename(&partial, &target).map_err(|e| e.to_string())?;
    remove_stale(bundle);
    std::fs::read(&target).map_err(|e| e.to_string())
}

fn unity_cache(hash: &str) -> Option<PathBuf> {
    let root = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)?
        .join("AppData/LocalLow/Unity/Bigpoint GmbH_Seafight");
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path().join(hash).join("__data"))
        .find(|p| p.is_file())
}

/// Reads only the CDN root from the client log; nothing else is kept.
fn cdn_root() -> String {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("AppData/LocalLow/Bigpoint GmbH/Seafight/Player.log"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|log| {
            log.lines().find_map(|line| {
                let (_, url) = line.split_once("StreamingAssetsLoadURL ")?;
                let url = url.trim();
                url.starts_with("https://")
                    .then(|| url.trim_end_matches('/').to_owned())
            })
        })
        .unwrap_or_else(|| DEFAULT_CDN.to_owned())
}

fn remove_stale(current: &str) {
    let Some(prefix) = current.rsplit_once('_').map(|(p, _)| p.to_owned()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name != current {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn texts(bundle: &[u8]) -> Result<HashMap<String, String>, String> {
    for node in unityfs::open(bundle)? {
        if node.path.ends_with(".resS") || node.path.ends_with(".resource") {
            continue;
        }
        for object in serialized::mono_behaviours(&node.data)? {
            let Some(items) = object.field("TextList").and_then(serialized::Value::items) else {
                continue;
            };
            let mut out = HashMap::with_capacity(items.len());
            for item in items {
                if let (Some(id), Some(text)) = (
                    item.field("TextId").and_then(serialized::Value::text),
                    item.field("Text").and_then(serialized::Value::text),
                ) {
                    out.insert(id, text);
                }
            }
            return Ok(out);
        }
    }
    Err("no localisation table in bundle".into())
}

const KINDS: [(NameKind, &str); 9] = [
    (NameKind::Npc, "npc"),
    (NameKind::Monster, "monster"),
    (NameKind::Box, "box"),
    (NameKind::Ammo, "ammo"),
    (NameKind::Harpoon, "harpoon"),
    (NameKind::Torpedo, "torpedo"),
    (NameKind::ActionItem, "item"),
    (NameKind::Currency, "currency"),
    (NameKind::Map, "map"),
];

fn write_cache(path: &Path, lang: &str, language: &Language) {
    let mut out = format!(
        "#kraken-names\t{CACHE_VERSION}\t{}\t{}\n",
        language.hash, language.texts
    );
    for (kind, label) in KINDS {
        for (id, name) in language.names.table(kind) {
            out.push_str(&format!("{label}\t{id}\t{}\n", name.replace(['\t', '\n'], " ")));
        }
    }
    let _ = std::fs::create_dir_all(cache_dir());
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, path).is_ok() {
        remove_stale_caches(lang, path);
    }
}

fn remove_stale_caches(lang: &str, keep: &Path) {
    let prefix = format!("names-{lang}-");
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && entry.path() != keep {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn read_cache(path: &Path, hash: &str) -> Option<Language> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let mut header = lines.next()?.split('\t');
    let (Some("#kraken-names"), Some(version), Some(stored), Some(texts)) =
        (header.next(), header.next(), header.next(), header.next())
    else {
        return None;
    };
    if version != CACHE_VERSION.to_string() || stored != hash {
        return None;
    }
    let mut names = NameTable::default();
    for line in lines {
        let mut parts = line.splitn(3, '\t');
        let (Some(label), Some(id), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Some((kind, _)) = KINDS.iter().find(|(_, l)| *l == label) else {
            continue;
        };
        let Ok(id) = id.parse() else { continue };
        names.insert(*kind, id, name.to_owned());
    }
    Some(Language {
        hash: hash.to_owned(),
        texts: texts.parse().unwrap_or(0),
        names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_bundle(lang: &str) -> Option<Vec<u8>> {
        let dir = local_app_data()?.join("Seafight/wiki-research/data/bundles");
        let entry = std::fs::read_dir(dir).ok()?.flatten().find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("loca_{lang}_"))
        })?;
        std::fs::read(entry.path()).ok()
    }

    #[test]
    fn parses_turkish_bundle() {
        let Some(bytes) = saved_bundle("tr") else {
            eprintln!("no saved bundle on this machine; skipped");
            return;
        };
        let texts = texts(&bytes).unwrap();
        assert!(texts.len() > 30_000, "{}", texts.len());
        let names = names::build(&texts);
        assert_eq!(names.get(NameKind::Npc, 1001), Some("Admiral Jack"));
        assert_eq!(names.get(NameKind::Ammo, 1), Some("Zincirli cephane"));
        assert_eq!(names.get(NameKind::Harpoon, 18), Some("Bakır zıpkın"));
        assert!(names.boxes.len() > 50);
    }

    #[test]
    fn cache_round_trips() {
        let mut names = NameTable::default();
        names.insert(NameKind::Npc, 7, "Kara Korsan".into());
        names.insert(NameKind::Box, 1, "Pırıltı".into());
        let language = Language {
            hash: "abc".into(),
            texts: 2,
            names,
        };
        let path = std::env::temp_dir().join(format!("kraken-names-test-{}.tsv", std::process::id()));
        let mut out = format!("#kraken-names\t{CACHE_VERSION}\tabc\t2\n");
        for (kind, label) in KINDS {
            for (id, name) in language.names.table(kind) {
                out.push_str(&format!("{label}\t{id}\t{name}\n"));
            }
        }
        std::fs::write(&path, out).unwrap();
        let back = read_cache(&path, "abc").unwrap();
        assert_eq!(back.names, language.names);
        assert!(read_cache(&path, "other").is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn finds_hash_in_bundle_name() {
        assert_eq!(
            hash_of("loca_tr_assets_all_c7278594490841f6175b524fb37aa495.bundle"),
            "c7278594490841f6175b524fb37aa495"
        );
    }
}

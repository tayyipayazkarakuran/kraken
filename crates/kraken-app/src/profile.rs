use std::path::PathBuf;
use std::time::{Duration, Instant};

use kraken_proto::Config;
use serde::{Deserialize, Serialize};

pub const PROFILE_VERSION: u32 = 1;
const SAVE_DELAY: Duration = Duration::from_millis(600);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlaySettings {
    /// Draw the interface inside the game.
    pub enabled: bool,
    pub hud: bool,
    pub markers: bool,
    /// The cannon range circle around the ship.
    pub range_ring: bool,
    /// Size of the in-game panel relative to the game's pixels.
    pub scale: f32,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            hud: true,
            markers: true,
            range_ring: true,
            scale: 0.85,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub version: u32,
    pub locale: String,
    pub auto_attach: bool,
    /// Kraken starts the game itself (each instance its own client).
    pub launch: bool,
    /// The launcher is started under explorer.exe, so the process tree
    /// looks like the one a double-click produces. Experimental; if the
    /// spoof cannot be set up the launcher still opens normally.
    pub parent_spoof: bool,
    pub overlay: OverlaySettings,
    /// The account name, for listing account files.
    pub account: String,
    pub config: Config,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            version: PROFILE_VERSION,
            locale: "en".into(),
            auto_attach: true,
            launch: true,
            parent_spoof: true,
            overlay: OverlaySettings::default(),
            account: String::new(),
            config: Config::default(),
        }
    }
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut profile: Self = toml::from_str(text)?;
        profile.migrate();
        Ok(profile)
    }

    fn migrate(&mut self) {
        if !matches!(self.locale.as_str(), "en" | "tr") {
            self.locale = "en".into();
        }
        let safety = &mut self.config.safety;
        safety.repair_start = safety.repair_start.clamp(5, 95);
        safety.repair_end = safety.repair_end.clamp(safety.repair_start + 1, 100);
        self.config.npc.range_percent = self.config.npc.range_percent.clamp(60, 95);
        if !(0.6..=1.5).contains(&self.overlay.scale) {
            self.overlay.scale = 0.85;
        }
        self.version = PROFILE_VERSION;
    }
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("Kraken")
}

/// Per-account settings, keyed by the game's user id.
pub fn account_path(user_id: i64) -> PathBuf {
    data_dir().join("accounts").join(format!("{user_id}.toml"))
}

/// Every saved account: user id and name.
pub fn accounts() -> Vec<(i64, String)> {
    let Ok(entries) = std::fs::read_dir(data_dir().join("accounts")) else {
        return Vec::new();
    };
    let mut out: Vec<(i64, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.parse().ok()?;
            let text = std::fs::read_to_string(&path).ok()?;
            let profile = Profile::parse(&text).ok()?;
            Some((id, profile.account))
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// The settings saved for another account.
pub fn account_config(user_id: i64) -> Option<Config> {
    let text = std::fs::read_to_string(account_path(user_id)).ok()?;
    Profile::parse(&text).ok().map(|p| p.config)
}

pub struct Store {
    path: PathBuf,
    saved: String,
    pending: Option<Instant>,
}

impl Store {
    pub fn open(path: PathBuf) -> (Self, Profile) {
        let profile = match std::fs::read_to_string(&path) {
            Ok(text) => Profile::parse(&text).unwrap_or_else(|error| {
                tracing::warn!(%error, "profile unreadable, keeping a backup and starting fresh");
                let _ = std::fs::copy(&path, path.with_extension("toml.bak"));
                Profile::default()
            }),
            Err(_) => Profile::default(),
        };
        let saved = toml::to_string_pretty(&profile).unwrap_or_default();
        (
            Self {
                path,
                saved,
                pending: None,
            },
            profile,
        )
    }

    pub fn track(&mut self, profile: &Profile, now: Instant) -> bool {
        let Ok(text) = toml::to_string_pretty(profile) else {
            return false;
        };
        if text == self.saved {
            self.pending = None;
            return false;
        }
        let due = *self.pending.get_or_insert(now);
        if now.duration_since(due) < SAVE_DELAY {
            return false;
        }
        self.write(text)
    }

    pub fn flush(&mut self, profile: &Profile) {
        if let Ok(text) = toml::to_string_pretty(profile)
            && text != self.saved
        {
            self.write(text);
        }
    }

    fn write(&mut self, text: String) -> bool {
        self.pending = None;
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = self.path.with_extension("toml.tmp");
        let result = std::fs::write(&temp, &text).and_then(|()| std::fs::rename(&temp, &self.path));
        match result {
            Ok(()) => {
                self.saved = text;
                true
            }
            Err(error) => {
                tracing::warn!(%error, "profile save failed");
                false
            }
        }
    }

    pub fn wants_repaint(&self) -> Option<Duration> {
        self.pending.map(|_| SAVE_DELAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kraken_proto::{NpcTarget, RangeStyle, Route};

    #[test]
    fn round_trips_full_profile() {
        let mut profile = Profile {
            locale: "tr".into(),
            ..Profile::default()
        };
        profile.config.npc.targets.push(NpcTarget {
            type_id: 12,
            name: "Pirate".into(),
            priority: 3,
            ammo: Some(4),
            range: RangeStyle::Keep,
            items: Vec::new(),
        });
        profile.config.navigation.routes.push(Route {
            map_id: 43,
            points: vec![kraken_proto::Point::new(10.0, 20.0)],
        });
        let text = toml::to_string_pretty(&profile).unwrap();
        assert_eq!(Profile::parse(&text).unwrap(), profile);
    }

    #[test]
    fn migrates_bad_values() {
        let text = "locale = \"de\"\n[config.safety]\nrepair_start = 99\nrepair_end = 10\n";
        let profile = Profile::parse(text).unwrap();
        assert_eq!(profile.locale, "en");
        assert_eq!(profile.config.safety.repair_start, 95);
        assert_eq!(profile.config.safety.repair_end, 96);
        assert!(profile.config.safety.respawn);
    }

    #[test]
    fn debounces_saves() {
        let dir = std::env::temp_dir().join(format!("kraken-profile-{}", std::process::id()));
        let path = dir.join("profile.toml");
        let (mut store, mut profile) = Store::open(path.clone());
        let start = Instant::now();
        profile.auto_attach = false;
        assert!(!store.track(&profile, start));
        assert!(!store.track(&profile, start + Duration::from_millis(100)));
        assert!(store.track(&profile, start + Duration::from_millis(700)));
        assert!(
            !Profile::parse(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .auto_attach
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

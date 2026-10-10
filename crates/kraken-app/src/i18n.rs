use kraken_proto::{
    Activity, BonusInfo, BonusPhase, Condition, ElDoradoInfo, ElDoradoPhase, EnemyScope, Event, EventCode,
    ItemWhen, MixedPriority, RangeStyle, RoamMode, RunState, Scene, SessionState, WorkMode,
};
use rust_i18n::t;

use crate::supervisor::{Fault, Link};

pub const LOCALES: [(&str, &str); 2] = [("en", "English"), ("tr", "Türkçe")];

pub fn apply(locale: &str) {
    rust_i18n::set_locale(locale);
}

pub fn activity(value: Activity) -> String {
    match value {
        Activity::Idle => t!("activity.idle"),
        Activity::Syncing => t!("activity.syncing"),
        Activity::Roaming => t!("activity.roaming"),
        Activity::Npc => t!("activity.npc"),
        Activity::Monster => t!("activity.monster"),
        Activity::Collecting => t!("activity.collecting"),
        Activity::Repairing => t!("activity.repairing"),
        Activity::Dead => t!("activity.dead"),
        Activity::Recovering => t!("activity.recovering"),
        Activity::War => t!("activity.war"),
        Activity::ElDorado => t!("activity.eldorado"),
        Activity::Bonus => t!("activity.bonus"),
        Activity::Travelling => t!("activity.travelling"),
    }
    .into_owned()
}

pub fn run_state(value: RunState) -> String {
    match value {
        RunState::Stopped => t!("run.stopped"),
        RunState::Running => t!("run.running"),
        RunState::Paused => t!("run.paused"),
    }
    .into_owned()
}

pub fn session(value: SessionState) -> String {
    match value {
        SessionState::Loading => t!("session.loading"),
        SessionState::Ready => t!("session.ready"),
        SessionState::Disconnected => t!("session.disconnected"),
        SessionState::Reconnecting => t!("session.reconnecting"),
    }
    .into_owned()
}

pub fn link(value: Link) -> String {
    match value {
        Link::Idle => t!("link.idle").into_owned(),
        Link::Searching => t!("link.searching").into_owned(),
        Link::Launching => t!("link.launching").into_owned(),
        Link::Injecting(pid) => t!("link.injecting", pid = pid).into_owned(),
        Link::Connecting(pid) => t!("link.connecting", pid = pid).into_owned(),
        Link::Connected(pid) => t!("link.connected", pid = pid).into_owned(),
        Link::Failed(fault) => fault_text(fault),
    }
}

pub fn fault_text(fault: Fault) -> String {
    match fault {
        Fault::MissingFiles => t!("fault.missing_files").into_owned(),
        Fault::AccessDenied => t!("fault.access_denied").into_owned(),
        Fault::NotGame => t!("fault.not_game").into_owned(),
        Fault::InjectFailed(code) => t!("fault.inject_failed", code = code).into_owned(),
        Fault::LaunchFailed(code) => t!("fault.launch_failed", code = code).into_owned(),
        Fault::Protocol(version) => t!("fault.protocol", version = version).into_owned(),
    }
}

pub fn work_mode(value: WorkMode) -> String {
    match value {
        WorkMode::Collect => t!("mode.collect"),
        WorkMode::Combat => t!("mode.combat"),
        WorkMode::Mixed => t!("mode.mixed"),
        WorkMode::War => t!("mode.war"),
    }
    .into_owned()
}

/// `target`: the scene belongs to an NPC or monster target's own rules.
pub fn scene(value: Scene, target: bool) -> String {
    match (value, target) {
        (Scene::Always, _) => t!("scene.always"),
        (Scene::Collecting, _) => t!("scene.collecting"),
        (Scene::Roaming, _) => t!("scene.roaming"),
        (Scene::FightingNpc, false) => t!("scene.fighting_npc"),
        (Scene::FightingMonster, false) => t!("scene.fighting_monster"),
        (Scene::FightingNpc | Scene::FightingMonster, true) => t!("scene.fighting_this"),
        (Scene::InSight, false) => t!("scene.in_sight"),
        (Scene::InSight, true) => t!("scene.this_in_sight"),
        (Scene::War, _) => t!("scene.war"),
        (Scene::ElDorado, _) => t!("scene.eldorado"),
        (Scene::BonusMap, _) => t!("scene.bonus"),
    }
    .into_owned()
}

pub fn condition(value: Condition) -> String {
    match value {
        Condition::None => t!("condition.none"),
        Condition::HpBelow(_) => t!("condition.hp"),
        Condition::VpBelow(_) => t!("condition.vp"),
        Condition::TargetHpBelow(_) => t!("condition.target_hp"),
        Condition::Held => t!("condition.held"),
    }
    .into_owned()
}

pub fn enemy_scope(value: EnemyScope) -> String {
    match value {
        EnemyScope::AllPlayers => t!("war.scope_all"),
        EnemyScope::ByRelation => t!("war.scope_relation"),
    }
    .into_owned()
}

pub fn item_when(value: ItemWhen) -> String {
    match value {
        ItemWhen::TargetNear { .. } => t!("war.when_near"),
        ItemWhen::TargetFar { .. } => t!("war.when_far"),
        ItemWhen::InCombat => t!("war.when_combat"),
        ItemWhen::Always => t!("war.when_always"),
        ItemWhen::HpBelow(_) => t!("war.when_hp"),
        ItemWhen::VpBelow(_) => t!("war.when_vp"),
        ItemWhen::FriendHpBelow(_) => t!("war.when_friend_hp"),
        ItemWhen::FriendNear { .. } => t!("war.when_friend_near"),
        ItemWhen::Immobilized => t!("war.when_immobilized"),
    }
    .into_owned()
}

pub fn mixed_priority(value: MixedPriority) -> String {
    match value {
        MixedPriority::Combat => t!("mode.combat_first"),
        MixedPriority::Collect => t!("mode.collect_first"),
    }
    .into_owned()
}

pub fn roam(value: RoamMode) -> String {
    match value {
        RoamMode::Random => t!("roam.random"),
        RoamMode::Corners => t!("roam.corners"),
        RoamMode::Lanes => t!("roam.lanes"),
        RoamMode::Route => t!("roam.route"),
        RoamMode::Idle => t!("roam.idle"),
    }
    .into_owned()
}

pub fn range(value: RangeStyle) -> String {
    match value {
        RangeStyle::Default => t!("range.default"),
        RangeStyle::Approach => t!("range.approach"),
        RangeStyle::Keep => t!("range.keep"),
    }
    .into_owned()
}

pub fn event(event: &Event) -> String {
    let base = match event.code {
        EventCode::AgentReady => t!("event.agent_ready"),
        EventCode::BindingMissing => t!("event.binding_missing"),
        EventCode::SessionReady => t!("event.session_ready"),
        EventCode::MapChanged => t!("event.map_changed"),
        EventCode::TargetEngaged => t!("event.target_engaged"),
        EventCode::TargetKilled => t!("event.target_killed"),
        EventCode::TargetSkipped => t!("event.target_skipped"),
        EventCode::BoxCollected => t!("event.box_collected"),
        EventCode::BoxSkipped => t!("event.box_skipped"),
        EventCode::NoAmmo => t!("event.no_ammo"),
        EventCode::Stuck => t!("event.stuck"),
        EventCode::RouteInvalid => t!("event.route_invalid"),
        EventCode::RepairStarted => t!("event.repair_started"),
        EventCode::RepairFinished => t!("event.repair_finished"),
        EventCode::RepairStalled => t!("event.repair_stalled"),
        EventCode::Died => t!("event.died"),
        EventCode::Respawning => t!("event.respawning"),
        EventCode::RespawnFailed => t!("event.respawn_failed"),
        EventCode::Disconnected => t!("event.disconnected"),
        EventCode::Reconnecting => t!("event.reconnecting"),
        EventCode::ReconnectBlocked => t!("event.reconnect_blocked"),
        EventCode::Reconnected => t!("event.reconnected"),
        EventCode::TypesDumped => t!("event.types_dumped"),
        EventCode::Fault => t!("event.fault"),
        EventCode::Unloading => t!("event.unloading"),
        EventCode::SafeHaven => t!("event.safe_haven"),
        EventCode::NoHarpoonRange => t!("event.no_harpoon_range"),
        EventCode::WarTarget => t!("event.war_target"),
        EventCode::ItemUsed => t!("event.item_used"),
        EventCode::ItemOff => t!("event.item_off"),
        EventCode::ElDoradoReady => t!("event.eldorado_ready"),
        EventCode::ElDoradoTriggered => t!("event.eldorado_triggered"),
        EventCode::ElDoradoEntered => t!("event.eldorado_entered"),
        EventCode::ElDoradoLeft => t!("event.eldorado_left"),
        EventCode::ElDoradoFailed => t!("event.eldorado_failed"),
        EventCode::BonusEntering => t!("event.bonus_entering"),
        EventCode::BonusEntered => t!("event.bonus_entered"),
        EventCode::BonusWave => t!("event.bonus_wave"),
        EventCode::BonusFinished => t!("event.bonus_finished"),
        EventCode::BonusFailed => t!("event.bonus_failed"),
        EventCode::BonusQueueDone => t!("event.bonus_queue_done"),
        EventCode::ClockAnomaly => t!("event.clock_anomaly"),
        EventCode::WorkmapTravel => t!("event.workmap_travel"),
        EventCode::WorkmapTeleport => t!("event.workmap_teleport"),
        EventCode::WorkmapArrived => t!("event.workmap_arrived"),
        EventCode::WorkmapFailed => t!("event.workmap_failed"),
    };
    let args: Vec<String> = event
        .args
        .iter()
        .map(|arg| match (event.code, arg.as_str()) {
            (EventCode::RepairStalled, "no_carpenter") => t!("repair_reason.no_carpenter").into_owned(),
            (EventCode::RepairStalled, "cooldown") => t!("repair_reason.cooldown").into_owned(),
            (EventCode::RepairStalled, "breach") => t!("repair_reason.breach").into_owned(),
            (EventCode::WorkmapFailed, "atlas") => t!("workmap_reason.atlas").into_owned(),
            (EventCode::WorkmapFailed, "route") => t!("workmap_reason.route").into_owned(),
            (EventCode::WorkmapFailed, "teleport") => t!("workmap_reason.teleport").into_owned(),
            (EventCode::WorkmapFailed, "jump") => t!("workmap_reason.jump").into_owned(),
            (EventCode::WorkmapFailed, "edges") => t!("workmap_reason.edges").into_owned(),
            _ => arg.clone(),
        })
        .collect();
    if args.is_empty() {
        base.into_owned()
    } else {
        format!("{base} · {}", args.join(" · "))
    }
}

/// "Charging 87%", "Sailing to a corner", "On El Dorado · 0:42 · 12 chests"…
pub fn eldorado_status(info: &ElDoradoInfo) -> String {
    match info.phase {
        ElDoradoPhase::Waiting if info.charge >= 100 => t!("eldorado.ready").into_owned(),
        ElDoradoPhase::Waiting => t!("eldorado.charging", charge = info.charge).into_owned(),
        ElDoradoPhase::Finishing => t!("eldorado.finishing").into_owned(),
        ElDoradoPhase::Retreat => t!("eldorado.retreating").into_owned(),
        ElDoradoPhase::Countdown => t!("eldorado.countdown").into_owned(),
        ElDoradoPhase::Run => {
            let secs = info.remaining_ms / 1_000;
            let left = if secs > 0 {
                format!(" · {}:{:02}", secs / 60, secs % 60)
            } else {
                String::new()
            };
            format!(
                "{}{left} · {}",
                t!("eldorado.running"),
                t!("eldorado.chests", count = info.chests)
            )
        }
    }
}

/// "Virgo map · wave 3/10", "Entering Virgo map · 0:07"…
pub fn bonus_status(info: &BonusInfo, name: impl Fn(i16) -> String) -> String {
    let map = info.map.map(&name).unwrap_or_default();
    match info.phase {
        BonusPhase::Waiting => t!("bonus.waiting").into_owned(),
        BonusPhase::Finishing => t!("bonus.finishing", map = map).into_owned(),
        BonusPhase::Retreat => t!("bonus.retreating", map = map).into_owned(),
        BonusPhase::Countdown => {
            let secs = info.delay_ms.div_ceil(1_000);
            let left = if secs > 0 {
                format!(" · {}:{:02}", secs / 60, secs % 60)
            } else {
                String::new()
            };
            format!("{}{left}", t!("bonus.countdown", map = map))
        }
        BonusPhase::Fighting if info.waves > 0 => t!(
            "bonus.fighting_waves",
            map = map,
            wave = info.wave,
            waves = info.waves
        )
        .into_owned(),
        BonusPhase::Fighting => t!("bonus.fighting", map = map, wave = info.wave).into_owned(),
    }
}

pub fn duration(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    fn flatten(prefix: &str, value: &toml::Value, out: &mut BTreeSet<String>) {
        match value {
            toml::Value::Table(table) => {
                for (key, value) in table {
                    if key.starts_with('_') {
                        continue;
                    }
                    let name = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}.{key}")
                    };
                    flatten(&name, value, out);
                }
            }
            _ => {
                out.insert(prefix.to_owned());
            }
        }
    }

    fn keys(locale: &str) -> BTreeSet<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("locales")
            .join(format!("{locale}.toml"));
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut out = BTreeSet::new();
        flatten("", &value, &mut out);
        out
    }

    fn used_keys(dir: &Path, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                used_keys(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for (at, marker) in text.match_indices("t!(\"") {
                    let standalone = text[..at]
                        .chars()
                        .next_back()
                        .is_none_or(|c| !c.is_alphanumeric() && c != '_');
                    let rest = &text[at + marker.len()..];
                    if standalone && let Some(end) = rest.find('"') {
                        out.insert(rest[..end].to_owned());
                    }
                }
            }
        }
    }

    #[test]
    fn locales_have_same_keys() {
        let en = keys("en");
        let tr = keys("tr");
        let only_en: Vec<_> = en.difference(&tr).collect();
        let only_tr: Vec<_> = tr.difference(&en).collect();
        assert!(
            only_en.is_empty() && only_tr.is_empty(),
            "en only: {only_en:?}, tr only: {only_tr:?}"
        );
    }

    #[test]
    fn every_used_key_exists() {
        let en = keys("en");
        let mut used = BTreeSet::new();
        used_keys(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut used);
        let missing: Vec<_> = used.difference(&en).collect();
        assert!(missing.is_empty(), "missing keys: {missing:?}");
    }
}

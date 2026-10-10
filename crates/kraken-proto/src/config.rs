use serde::{Deserialize, Serialize};

use crate::Point;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mode: WorkMode,
    pub mixed_priority: MixedPriority,
    pub npc: NpcConfig,
    pub monster: MonsterConfig,
    pub collector: CollectorConfig,
    pub navigation: NavigationConfig,
    pub safety: SafetyConfig,
    pub war: WarConfig,
    /// Item rules for every mode (the Items tab).
    pub items: Vec<ItemRule>,
    pub eldorado: ElDoradoConfig,
    pub bonus: BonusConfig,
    pub humanize: HumanizeConfig,
    pub workmap: WorkmapConfig,
}

/// Work maps: the bot works on each listed map for a while, then travels to
/// the next one (teleport first, else from map edge to map edge).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkmapConfig {
    pub enabled: bool,
    /// Visited in this order, then from the top again.
    pub maps: Vec<WorkmapEntry>,
    /// Time spent working on each map.
    pub minutes: u16,
    /// Use a teleport item when one is in stock.
    pub teleport: bool,
    /// Safe havens may be crossed on the way.
    pub safe_havens: bool,
}

impl Default for WorkmapConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            maps: Vec::new(),
            minutes: 30,
            teleport: true,
            safe_havens: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkmapEntry {
    pub map: i16,
    pub name: String,
}

/// Bonus maps (zodiac maps and the like), played from a queue: enter one,
/// fight its waves, and go on with the next until the queue is done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BonusConfig {
    pub enabled: bool,
    /// Sail to the nearest map corner before the entry countdown.
    pub retreat: bool,
    /// Ammunition on bonus maps; `None` uses the NPC default.
    pub ammo: Option<i16>,
    /// Played in this order.
    pub queue: Vec<BonusEntry>,
}

impl Default for BonusConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retreat: true,
            ammo: None,
            queue: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonusEntry {
    /// The bonus map (its inventory id, which is also its map id).
    pub map: i16,
    pub name: String,
    /// How many to play; `None`: until none are left.
    pub count: Option<u16>,
}

/// The El Dorado master skill: once charged, retreat to a corner, jump to the
/// bonus map and collect its chests as fast as possible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ElDoradoConfig {
    pub enabled: bool,
    /// Sail to the nearest map corner before the jump countdown.
    pub retreat: bool,
    /// Keep a Swiftstone running on the bonus map.
    pub swiftstone: bool,
    /// Finish the fight in progress first (no new targets meanwhile).
    pub finish_target: bool,
}

impl Default for ElDoradoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retreat: true,
            swiftstone: true,
            finish_target: true,
        }
    }
}

/// `seafight.labels.MasterSkills.7`.
pub const ELDORADO_SKILL: i32 = 7;
/// `seafight.labels.Maps.151`, the El Dorado bonus map.
pub const ELDORADO_MAP: i32 = 151;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkMode {
    Collect,
    Combat,
    #[default]
    Mixed,
    /// PvP assist: the player steers, Kraken only picks targets, fires and
    /// uses action items.
    War,
}

impl WorkMode {
    pub const ALL: [Self; 4] = [Self::Collect, Self::Combat, Self::Mixed, Self::War];

    pub fn war(self) -> bool {
        self == Self::War
    }

    pub fn combat(self) -> bool {
        matches!(self, Self::Combat | Self::Mixed)
    }

    pub fn collect(self) -> bool {
        matches!(self, Self::Collect | Self::Mixed)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixedPriority {
    #[default]
    Combat,
    Collect,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeStyle {
    #[default]
    Default,
    Approach,
    Keep,
}

impl RangeStyle {
    pub const ALL: [Self; 3] = [Self::Default, Self::Approach, Self::Keep];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcConfig {
    pub enabled: bool,
    pub attack_all: bool,
    pub default_ammo: Option<i16>,
    pub keep_range: bool,
    pub range_percent: u8,
    pub targets: Vec<NpcTarget>,
}

impl Default for NpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            attack_all: false,
            default_ammo: None,
            keep_range: false,
            range_percent: 90,
            targets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcTarget {
    pub type_id: i32,
    pub name: String,
    pub priority: u8,
    pub ammo: Option<i16>,
    pub range: RangeStyle,
    /// Item rules that only concern this NPC type.
    #[serde(default)]
    pub items: Vec<ItemRule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MonsterConfig {
    pub enabled: bool,
    pub attack_all: bool,
    pub default_harpoon: Option<i16>,
    pub targets: Vec<MonsterTarget>,
}

impl Default for MonsterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            attack_all: false,
            default_harpoon: None,
            targets: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonsterTarget {
    pub type_id: i32,
    pub name: String,
    pub priority: u8,
    pub harpoon: Option<i16>,
    /// Item rules that only concern this monster type.
    #[serde(default)]
    pub items: Vec<ItemRule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectorConfig {
    pub enabled: bool,
    pub collect_all: bool,
    pub boxes: Vec<BoxTarget>,
    pub during_combat: bool,
    pub detour_radius: u16,
    pub detour_secs: u16,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            collect_all: false,
            boxes: Vec::new(),
            during_combat: false,
            detour_radius: 0,
            detour_secs: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxTarget {
    pub type_id: i32,
    pub name: String,
    pub priority: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoamMode {
    #[default]
    Random,
    Corners,
    Lanes,
    Route,
    Idle,
}

impl RoamMode {
    pub const ALL: [Self; 5] = [Self::Random, Self::Corners, Self::Lanes, Self::Route, Self::Idle];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationConfig {
    pub roam: RoamMode,
    pub routes: Vec<Route>,
    pub avoid_safe_haven: bool,
}

impl Default for NavigationConfig {
    fn default() -> Self {
        Self {
            roam: RoamMode::Random,
            routes: Vec::new(),
            avoid_safe_haven: true,
        }
    }
}

impl NavigationConfig {
    pub fn route_for(&self, map_id: i32) -> Option<&Route> {
        self.routes
            .iter()
            .find(|r| r.map_id == map_id && !r.points.is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub map_id: i32,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyConfig {
    pub repair: bool,
    pub repair_start: u8,
    pub repair_end: u8,
    pub respawn: bool,
    pub reconnect: bool,
    /// Minutes to wait before reconnecting on a long outage (a server reset
    /// or maintenance, or when quick retries keep failing). Short blips still
    /// reconnect fast; only a lasting outage waits this long.
    pub reconnect_wait_mins: u16,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            repair: true,
            repair_start: 35,
            repair_end: 90,
            respawn: true,
            reconnect: true,
            reconnect_wait_mins: 15,
        }
    }
}

/// Server-side evasion: makes the bot's pacing look human instead of a fixed
/// machine cadence. Draws from its own RNG so movement stays deterministic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HumanizeConfig {
    /// Master switch for every behaviour below.
    pub enabled: bool,
    /// Random spread, in percent, on refire and item-use intervals (0 = off).
    pub jitter_pct: u8,
    /// A random extra delay, up to this many ms, before the first shot at a
    /// freshly engaged target (a human's reaction time).
    pub reaction_ms: u16,
    /// Pause the bot when the game clock jumps backwards (an anti-cheat /
    /// replay signal), the way AnkerBot's "time went backwards" check does.
    pub clock_guard: bool,
}

impl Default for HumanizeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            jitter_pct: 30,
            reaction_ms: 500,
            clock_guard: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyScope {
    /// Every player outside the own guild, group and friend list.
    #[default]
    AllPlayers,
    /// Only players whose guild relation is selected in `WarRelations`.
    ByRelation,
}

impl EnemyScope {
    pub const ALL: [Self; 2] = [Self::AllPlayers, Self::ByRelation];
}

/// Guild colours that count as enemies under `EnemyScope::ByRelation`.
/// PEACE never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WarRelations {
    pub war: bool,
    pub neutral: bool,
    pub custom1: bool,
    pub custom2: bool,
}

impl Default for WarRelations {
    fn default() -> Self {
        Self {
            war: true,
            neutral: true,
            custom1: false,
            custom2: false,
        }
    }
}

/// When a war item rule wants its item active (toggle items) or used
/// (one-shot items). Percentages of distance refer to the hero's base
/// cannon range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemWhen {
    TargetNear {
        max_pct: u16,
    },
    TargetFar {
        min_pct: u16,
    },
    InCombat,
    Always,
    HpBelow(u8),
    VpBelow(u8),
    FriendHpBelow(u8),
    FriendNear {
        max_pct: u16,
    },
    /// The hero is held in place (smoke bomb, freeze), e.g. for Unstoppable.
    Immobilized,
}

/// Hailstorm (splash, shorter range) and Windstorm (longer range).
pub const ITEM_HAILSTORM: i16 = 28;
pub const ITEM_WINDSTORM: i16 = 29;
/// Unstoppable: frees the ship from movement debuffs such as a smoke bomb.
pub const ITEM_UNSTOPPABLE: i16 = 52;

impl ItemWhen {
    /// The rule an item gets when it is added, and which items are added on
    /// their own the first time the owned items are known.
    pub fn default_for(item: i16) -> (Self, bool) {
        match item {
            ITEM_HAILSTORM => (Self::TargetNear { max_pct: 50 }, true),
            ITEM_WINDSTORM => (Self::TargetFar { min_pct: 100 }, true),
            ITEM_UNSTOPPABLE => (Self::Immobilized, true),
            _ => (Self::InCombat, false),
        }
    }

    pub const KINDS: [Self; 9] = [
        Self::TargetNear { max_pct: 50 },
        Self::TargetFar { min_pct: 100 },
        Self::InCombat,
        Self::Always,
        Self::HpBelow(50),
        Self::VpBelow(50),
        Self::FriendHpBelow(50),
        Self::FriendNear { max_pct: 100 },
        Self::Immobilized,
    ];

    /// Rules that target a friend instead of the hero or the enemy.
    pub fn support(self) -> bool {
        matches!(self, Self::FriendHpBelow(_) | Self::FriendNear { .. })
    }

    pub fn same_kind(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarItemRule {
    pub item: i16,
    pub name: String,
    pub enabled: bool,
    pub when: ItemWhen,
    pub off_delay_ms: u16,
}

impl WarItemRule {
    pub fn new(item: i16, name: impl Into<String>, when: ItemWhen) -> Self {
        Self {
            item,
            name: name.into(),
            enabled: true,
            when,
            off_delay_ms: 3000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WarConfig {
    pub scope: EnemyScope,
    pub relations: WarRelations,
    /// Players without a guild count as enemies.
    pub guildless: bool,
    /// Player names that are never targeted and may receive support items.
    pub friends: Vec<String>,
    /// Auto-target radius, percent of the base cannon range.
    pub sight_pct: u16,
    /// Use support rules on friends.
    pub support: bool,
    pub items: Vec<WarItemRule>,
    /// The default rules (`ItemWhen::default_for`) were added once; later
    /// removals by the player stick.
    pub defaults_added: bool,
}

impl WarConfig {
    /// Adds the default rules for the owned items once. Returns whether
    /// anything changed.
    pub fn add_defaults<'a>(&mut self, owned: impl IntoIterator<Item = (i16, &'a str)>) -> bool {
        if self.defaults_added {
            return false;
        }
        let mut seen = false;
        for (item, name) in owned {
            seen = true;
            let (when, auto) = ItemWhen::default_for(item);
            if auto && !self.items.iter().any(|r| r.item == item) {
                self.items.push(WarItemRule::new(item, name, when));
            }
        }
        self.defaults_added = seen;
        seen
    }
}

impl Default for WarConfig {
    fn default() -> Self {
        Self {
            scope: EnemyScope::AllPlayers,
            relations: WarRelations::default(),
            guildless: true,
            friends: Vec::new(),
            sight_pct: 200,
            support: false,
            items: Vec::new(),
            defaults_added: false,
        }
    }
}

impl WarConfig {
    pub fn is_friend(&self, name: &str) -> bool {
        let name = name.trim();
        !name.is_empty()
            && self
                .friends
                .iter()
                .any(|f| f.trim().eq_ignore_ascii_case(name) || f.trim() == name)
    }
}

/// When a general item rule applies. In a rule attached to an NPC or monster
/// target, `Fighting*` and `InSight` mean that target type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scene {
    #[default]
    Always,
    Collecting,
    Roaming,
    FightingNpc,
    FightingMonster,
    InSight,
    War,
    /// On the El Dorado bonus map.
    ElDorado,
    /// On a wave bonus map (zodiac maps…).
    BonusMap,
}

impl Scene {
    /// Scenes offered on the Items tab.
    pub const GLOBAL: [Self; 8] = [
        Self::Always,
        Self::Collecting,
        Self::Roaming,
        Self::FightingNpc,
        Self::FightingMonster,
        Self::War,
        Self::ElDorado,
        Self::BonusMap,
    ];
    /// Scenes offered in an NPC target's settings.
    pub const NPC: [Self; 2] = [Self::FightingNpc, Self::InSight];
    /// Scenes offered in a monster target's settings.
    pub const MONSTER: [Self; 2] = [Self::FightingMonster, Self::InSight];
}

/// An extra condition on top of the scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    #[default]
    None,
    HpBelow(u8),
    VpBelow(u8),
    /// The current combat target's hull.
    TargetHpBelow(u8),
    /// Held in place (smoke bomb, freeze).
    Held,
}

impl Condition {
    pub const KINDS: [Self; 5] = [
        Self::None,
        Self::HpBelow(50),
        Self::VpBelow(50),
        Self::TargetHpBelow(50),
        Self::Held,
    ];

    pub fn same_kind(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// Use `item` while `scene` and `condition` hold. Items that switch on and
/// off are switched off again `off_delay_ms` after that ends; one-shot items
/// are used at most every `every_secs` (0: whenever off cooldown).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemRule {
    pub item: i16,
    pub name: String,
    pub enabled: bool,
    pub scene: Scene,
    pub condition: Condition,
    pub every_secs: u16,
    pub off_delay_ms: u16,
}

impl Default for ItemRule {
    fn default() -> Self {
        Self {
            item: 0,
            name: String::new(),
            enabled: true,
            scene: Scene::Always,
            condition: Condition::None,
            every_secs: 0,
            // Never switched off unless asked: re-using a timed item could
            // spend another one instead of turning it off.
            off_delay_ms: 0,
        }
    }
}

impl ItemRule {
    pub fn new(item: i16, name: impl Into<String>, scene: Scene, condition: Condition) -> Self {
        Self {
            item,
            name: name.into(),
            scene,
            condition,
            ..Self::default()
        }
    }
}

/// Swiftstone: faster sailing for a while.
pub const ITEM_SWIFTSTONE: i16 = 27;
/// Legba Amulet: restores voodoo points.
pub const ITEM_LEGBA: i16 = 45;

/// One-click suggestions on the Items tab.
pub fn suggested_rules() -> [(i16, Scene, Condition, u16); 3] {
    [
        (ITEM_SWIFTSTONE, Scene::Collecting, Condition::None, 30),
        (ITEM_LEGBA, Scene::Always, Condition::VpBelow(30), 0),
        (ITEM_UNSTOPPABLE, Scene::Always, Condition::Held, 0),
    ]
}

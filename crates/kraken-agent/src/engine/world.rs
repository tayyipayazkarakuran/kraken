use kraken_proto::{EntityKind, Point, Stock, StockKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Eid {
    pub id: i64,
    pub instance: i64,
}

impl Eid {
    pub const fn new(id: i64, instance: i64) -> Self {
        Self { id, instance }
    }

    pub fn matches(self, other: Self) -> bool {
        self.id == other.id && (self.instance == other.instance || self.instance == 0 || other.instance == 0)
    }

    pub fn is_set(self) -> bool {
        self.id != 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapKind {
    #[default]
    Normal,
    SafeHaven,
    Death,
    Special,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapState {
    pub id: i32,
    pub kind: MapKind,
    pub kind_name: String,
    pub width: f32,
    pub height: f32,
}

/// Why the game's own repair button would refuse right now
/// (`ActionMenuPanel.CanRepair`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairBlock {
    HealthFull,
    NoCarpenter,
    Breach,
    Attacking,
    Cooldown,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeroState {
    pub id: Eid,
    pub name: String,
    pub pos: Point,
    pub hp: f64,
    pub hp_max: f64,
    pub hp_known: bool,
    pub moving: Option<bool>,
    pub movement_enabled: Option<bool>,
    pub cannon_range: f32,
    pub harpoon_range: f32,
    pub repairing: Option<bool>,
    pub repair_block: Option<RepairBlock>,
    /// Cannon range without temporary buffs (`amspermanentvalues`).
    pub base_cannon_range: f32,
    pub vp: f64,
    pub vp_max: f64,
    /// What the player has selected in the game (`GameActorModel.TargetId`).
    pub selected: Option<Eid>,
}

impl HeroState {
    pub fn hp_percent(&self) -> f32 {
        if self.hp_max <= 0.0 {
            100.0
        } else {
            (self.hp / self.hp_max * 100.0) as f32
        }
    }

    pub fn vp_percent(&self) -> Option<f32> {
        (self.vp_max > 0.0).then(|| (self.vp / self.vp_max * 100.0) as f32)
    }

    /// Distance thresholds of the war bot are percentages of this.
    pub fn base_range(&self) -> f32 {
        if self.base_cannon_range > 0.0 {
            self.base_cannon_range
        } else {
            self.cannon_range
        }
    }

    pub fn alive(&self) -> bool {
        !self.dead()
    }

    pub fn dead(&self) -> bool {
        self.hp_known && self.hp_max > 0.0 && self.hp <= 0.0
    }
}

/// How a player stands to the hero, from guild membership, guild relation
/// colours and the group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Relation {
    /// Not a player, or nothing known.
    #[default]
    Unknown,
    OwnGuild,
    Group,
    Peace,
    War,
    Neutral,
    Custom1,
    Custom2,
    NoGuild,
}

impl Relation {
    pub fn friendly(self) -> bool {
        matches!(self, Self::OwnGuild | Self::Group | Self::Peace)
    }
}

/// An action item the hero owns (`DefaultItemConfigurationModel.ActionItems`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionItem {
    pub id: i16,
    pub amount: i32,
    pub active: bool,
    pub range: f32,
    /// Off cooldown, per `CooldownSystem.IsAtLeastOneStackReady`.
    pub ready: bool,
}

/// An owned bonus map and how far its waves have got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BonusMap {
    pub id: i16,
    pub amount: i64,
    /// Current wave (`InventorySystem.BonusMapStatus`).
    pub wave: u8,
    /// Waves in all (`DefaultBonusMapStub.waveAmount`); 0 if unknown.
    pub waves: u8,
}

/// A master skill of the skill tree (`MasterSkillDataStub`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MasterSkill {
    pub id: i32,
    /// Percent of `max_charge`; the skill can be used at 100.
    pub charge: u8,
    /// The charge as the game keeps it (points).
    pub points: i64,
    pub max_charge: i64,
    /// Learnt and usable with the current preset.
    pub active: bool,
    /// While triggered: time left.
    pub remaining_ms: u64,
    /// This is the El Dorado map jump.
    pub eldorado: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actor {
    pub id: Eid,
    pub kind: EntityKind,
    pub type_id: i32,
    pub name: String,
    pub pos: Point,
    pub hp_percent: Option<f32>,
    pub visible: bool,
    pub relation: Relation,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct World {
    pub connected: bool,
    pub ready: bool,
    pub map: Option<MapState>,
    pub hero: Option<HeroState>,
    pub actors: Vec<Actor>,
    pub stock: Vec<Stock>,
    pub stock_known: bool,
    pub items: Vec<ActionItem>,
    pub skills: Vec<MasterSkill>,
    pub bonus: Vec<BonusMap>,
    /// The world map, once the game has it loaded (or from the cache).
    pub atlas: Vec<MapCell>,
    /// Teleport items in stock (action item 34) and whether one is ready.
    pub teleports: i32,
    pub teleport_ready: bool,
}

impl World {
    pub fn eldorado(&self) -> Option<&MasterSkill> {
        self.skills.iter().find(|s| s.eldorado)
    }

    pub fn actor(&self, id: Eid) -> Option<&Actor> {
        self.actors.iter().find(|a| a.id.matches(id))
    }

    pub fn item(&self, id: i16) -> Option<&ActionItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn stock(&self, kind: StockKind, id: i16) -> Option<i64> {
        if !self.stock_known {
            return None;
        }
        Some(
            self.stock
                .iter()
                .find(|s| s.kind == kind && s.id == id)
                .map_or(0, |s| s.amount),
        )
    }

    pub fn playable(&self) -> bool {
        self.connected && self.ready && self.map.is_some() && self.hero.is_some()
    }
}

pub trait Terrain {
    fn blocked(&self, point: Point) -> Option<bool>;
}

pub struct OpenWater;

impl Terrain for OpenWater {
    fn blocked(&self, _: Point) -> Option<bool> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisconnectKind {
    Retry,
    Wait,
    Fatal,
}

impl DisconnectKind {
    pub fn classify(reason: &str) -> Self {
        match reason {
            "MAINTENANCE" | "SERVER_FULL" | "SHUTDOWN" => Self::Wait,
            "BANNED"
            | "KICKED"
            | "AUTH_FAILED"
            | "INVALID_UID"
            | "USER_AREADY_LOGGED_IN"
            | "USER_ALREADY_LOGGED_IN"
            | "TOO_MANY_USERS_FROM_SINGLE_IP"
            | "LOGOUT" => Self::Fatal,
            _ => Self::Retry,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Feedback {
    Hit {
        source: Eid,
        target: Eid,
    },
    Removed(Eid),
    AttackStarted,
    AttackEnded,
    AttackAborted,
    OwnDeath,
    MapChanged,
    Spawned,
    Disconnected(String),
    /// A visual/status effect on an entity (`module.effect.Effect` name),
    /// e.g. `SMOKEBOMB` when a smoke bomb holds a ship in place.
    Effect {
        name: String,
        target: Eid,
        active: bool,
        millis: Option<u64>,
    },
    /// `MasterSkillTriggerMessage`: a master skill went off.
    SkillTriggered {
        id: i32,
        remaining_ms: u64,
    },
    /// The server's own charge in percent: `MasterSkillChargeUpdateMessage`,
    /// or 0 after `MasterSkillUntriggerUpdateMessage`.
    SkillCharge {
        id: i32,
        charge: i64,
    },
    /// `MapChangeDelayedMessage`: a requested map change happens after `delay_ms`.
    MapDelay {
        map: i16,
        delay_ms: u64,
    },
    /// `MapChangeAskMessage`: at a map edge, the server offers `map`;
    /// `confirm` marks a jump that needs `ConfirmMapChangeMessage`.
    MapAsk {
        map: i16,
        confirm: bool,
    },
    /// `MapChangeCancelMessage`: the offer or the countdown is off.
    MapCancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Move(Point),
    Cannon {
        target: Eid,
        ammo: i16,
    },
    Harpoon {
        target: Eid,
        harpoon: i16,
    },
    AbortAttack,
    Repair(bool),
    Respawn,
    CloseDeathWindow,
    Reconnect,
    /// Sets the game's own selection, as a click on the ship would.
    Select(Eid),
    /// `AttackSystem.Attack()`: fires at the selection with the ammunition the
    /// player picked in the game.
    AttackSelected,
    /// `ActionItemUtils.useActionItem`; with a target, the selection is moved
    /// to it for the call and then restored.
    UseItem {
        item: i16,
        target: Option<Eid>,
    },
    /// `SkillTreeSystem.RequestMasterSkillTrigger`, as the skill button does.
    TriggerSkill(i32),
    /// `MapChangeRequestMessage`, as a bonus map's button does.
    EnterMap(i16),
    /// `MapChangeEdgeMessage`, as the map switch button does after an edge
    /// offer (`MapSwitchBehaviour.OnMapSwitchButtonClick`).
    EdgeJump(i16),
    /// `ConfirmMapChangeMessage(CONFIRM_MAP_JUMP)`
    /// (`MapSwitchBehaviour.OnMapSwitchConfirm`).
    ConfirmJump,
    /// `TeleportMessage`, as the world map's teleport button does
    /// (`WorldMapDataSystem.RequestTeleportation`).
    Teleport(i16),
}

/// A map on the game's world map (`Seafight.WorldMap.MapCellData`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapCell {
    pub id: i16,
    pub world: u8,
    pub row: u8,
    pub col: u8,
    pub safe_haven: bool,
    pub min_level: i32,
    /// The world map's own label: "30/1", "SH-6", "RÖK-1", "DH-1"…
    pub label: String,
}

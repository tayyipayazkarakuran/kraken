use serde::{Deserialize, Serialize};

use crate::{Config, Point};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AppMessage {
    Hello {
        version: u32,
    },
    Configure(Box<Config>),
    Control(Control),
    RequestCatalog,
    DumpTypes,
    Unload,
    /// The in-game interface, already laid out and tessellated by the app.
    Frame(Box<UiFrame>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    Start,
    Pause,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AgentMessage {
    Hello {
        version: u32,
        pid: u32,
    },
    Bindings(BindingReport),
    Telemetry(Box<Telemetry>),
    Catalog(Box<Catalog>),
    Event(Event),
    /// Mouse and keyboard from the game window, for the in-game interface.
    Input(UiInput),
}

/// One rendered frame of the in-game interface, in client-area pixels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiFrame {
    /// Client size the frame was laid out for.
    pub width: u32,
    pub height: u32,
    pub textures: Vec<UiTexture>,
    pub freed: Vec<u64>,
    pub meshes: Vec<UiMesh>,
    /// Where the interface takes the mouse away from the game.
    pub capture: Vec<UiRect>,
    /// A text field has focus: keys go to the interface, not the game.
    pub keyboard: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiRect {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl UiRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.min[0] && x < self.max[0] && y >= self.min[1] && y < self.max[1]
    }
}

/// A texture upload: the whole image (`origin: None`) or a patch of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiTexture {
    pub id: u64,
    pub origin: Option<[u32; 2]>,
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8.
    pub rgba: Vec<u8>,
    /// Sample with linear filtering (fonts) or nearest.
    pub linear: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiMesh {
    pub texture: u64,
    pub clip: UiRect,
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    /// Premultiplied RGBA8.
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiInput {
    /// Client size changed (also sent once when the overlay starts).
    Resized {
        width: u32,
        height: u32,
    },
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerLeft,
    /// 0 left, 1 right, 2 middle.
    Button {
        x: f32,
        y: f32,
        button: u8,
        pressed: bool,
    },
    /// Wheel notches; positive is up / right.
    Wheel {
        x: f32,
        y: f32,
    },
    /// Windows virtual-key code with modifiers (bit 0 shift, 1 ctrl, 2 alt).
    Key {
        vk: u16,
        pressed: bool,
        modifiers: u8,
    },
    Text(String),
    /// F9 in the game window: show or hide the panel.
    TogglePanel,
    Focus(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingReport {
    pub missing: Vec<String>,
    pub capabilities: Capabilities,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub world: bool,
    pub movement: bool,
    pub cannon: bool,
    pub harpoon: bool,
    pub abort_attack: bool,
    pub repair: bool,
    pub respawn: bool,
    pub reconnect: bool,
    pub terrain: bool,
    pub screen: bool,
    pub select_target: bool,
    pub use_item: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunState {
    #[default]
    Stopped,
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    #[default]
    Loading,
    Ready,
    Disconnected,
    Reconnecting,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activity {
    #[default]
    Idle,
    Syncing,
    Roaming,
    Npc,
    Monster,
    Collecting,
    Repairing,
    Dead,
    Recovering,
    War,
    /// Getting ready for the El Dorado jump (corner, countdown).
    ElDorado,
    /// Getting ready to enter a bonus map (corner, countdown).
    Bonus,
    /// Travelling to the next work map.
    Travelling,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    #[default]
    Unknown,
    Player,
    Npc,
    Monster,
    Box,
    AreaEffect,
    Tower,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MapInfo {
    pub id: i32,
    pub kind: String,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hero {
    /// `PlayerInfoSystem.UserId`; identifies the account.
    pub user_id: i64,
    pub name: String,
    pub position: Point,
    pub hp: f64,
    pub hp_max: f64,
    pub moving: bool,
    pub cannon_range: f32,
    pub harpoon_range: f32,
    pub vp: f64,
    pub vp_max: f64,
}

impl Hero {
    pub fn vp_percent(&self) -> Option<f32> {
        (self.vp_max > 0.0).then(|| (self.vp / self.vp_max * 100.0) as f32)
    }

    pub fn hp_percent(&self) -> f32 {
        if self.hp_max <= 0.0 {
            0.0
        } else {
            (self.hp / self.hp_max * 100.0) as f32
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TargetInfo {
    pub kind: EntityKind,
    pub type_id: i32,
    pub name: String,
    pub distance: f32,
    pub hp_percent: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub npc_kills: u32,
    pub monster_kills: u32,
    pub boxes: u32,
    pub deaths: u32,
    pub run_secs: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCounts {
    pub players: u16,
    pub npcs: u16,
    pub monsters: u16,
    pub boxes: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearbyIds {
    pub npcs: Vec<i32>,
    pub monsters: Vec<i32>,
    pub boxes: Vec<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StockKind {
    Ammo,
    Harpoon,
    /// Inventory `CURRENCY` (gold, pearls, crowns, Yulong, Safanad…).
    Currency,
    /// Inventory `POINTS`.
    Points,
    /// Inventory `TOKENS`.
    Token,
    /// Inventory `BONUSMAPS`: bonus maps owned, by map id.
    BonusMap,
}

impl StockKind {
    /// Kinds counted as earnings on the statistics page.
    pub fn wallet(self) -> bool {
        matches!(self, Self::Currency | Self::Points | Self::Token)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stock {
    pub kind: StockKind,
    pub id: i16,
    pub amount: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScreenMarkers {
    pub width: f32,
    pub height: f32,
    pub hero: Option<Point>,
    pub target: Option<Point>,
    pub waypoint: Option<Point>,
    /// Cannon range around the hero, projected to the screen (closed ring).
    pub range_ring: Vec<Point>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Telemetry {
    pub run: RunState,
    pub session: SessionState,
    pub activity: Activity,
    pub map: Option<MapInfo>,
    pub hero: Option<Hero>,
    pub target: Option<TargetInfo>,
    pub waypoint: Option<Point>,
    pub counters: Counters,
    pub visible: VisibleCounts,
    pub nearby: NearbyIds,
    pub stock: Vec<Stock>,
    pub screen: Option<ScreenMarkers>,
    pub war: Option<WarInfo>,
    pub eldorado: Option<ElDoradoInfo>,
    pub bonus: Option<BonusInfo>,
    pub workmap: Option<WorkmapInfo>,
    pub frame_us: u32,
    pub scan_us: u32,
}

/// Where the El Dorado automation stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElDoradoPhase {
    /// Charging, or switched off.
    #[default]
    Waiting,
    /// Charged; finishing the current fight.
    Finishing,
    /// Sailing to a corner.
    Retreat,
    /// Skill used; waiting for the jump.
    Countdown,
    /// On the bonus map, collecting.
    Run,
}

/// Where the bonus map queue stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BonusPhase {
    /// Nothing to play, or switched off.
    #[default]
    Waiting,
    /// A map is next; finishing the current fight.
    Finishing,
    /// Sailing to a corner.
    Retreat,
    /// Entry requested; waiting for the jump.
    Countdown,
    /// On the bonus map, fighting its waves.
    Fighting,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonusInfo {
    pub phase: BonusPhase,
    /// The map being entered or played.
    pub map: Option<i16>,
    pub wave: u8,
    pub waves: u8,
    /// Maps finished since the bot started, by map id.
    pub done: Vec<(i16, u32)>,
    /// Time left before the jump, when the game told us.
    pub delay_ms: u64,
}

/// Where the work map rotation stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkmapPhase {
    /// Switched off, or nothing listed.
    #[default]
    Idle,
    /// On a work map, working until the time is up.
    Working,
    /// Teleport requested; waiting for the jump.
    Teleport,
    /// Sailing to the map edge towards the next hop.
    Sailing,
    /// At the edge; the map change was confirmed, waiting for the jump.
    Crossing,
    /// Could not get on; waiting before trying again.
    Blocked,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkmapInfo {
    pub phase: WorkmapPhase,
    /// The work map being worked on or travelled to.
    pub target: Option<i16>,
    /// The next map on the way there.
    pub next_hop: Option<i16>,
    /// Time left on the current work map.
    pub remaining_secs: u64,
    /// The world map catalogue is known (routes between maps are possible).
    pub atlas: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElDoradoInfo {
    /// The skill is learnt and usable (`MasterSkillDataStub.active`).
    pub available: bool,
    /// Charge in percent (100 = ready).
    pub charge: u8,
    pub phase: ElDoradoPhase,
    /// Time left on the bonus map, when known.
    pub remaining_ms: u64,
    /// Chests collected in this run.
    pub chests: u32,
    /// Finished runs since the bot started.
    pub runs: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WarInfo {
    pub target: Option<String>,
    pub distance: f32,
    /// The player picked this target themselves.
    pub locked: bool,
    /// Ids of action items that are switched on.
    pub active_items: Vec<i16>,
    pub enemies: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub language: String,
    pub npcs: Vec<CatalogEntry>,
    pub monsters: Vec<CatalogEntry>,
    pub boxes: Vec<CatalogEntry>,
    pub ammo: Vec<ItemEntry>,
    pub harpoons: Vec<ItemEntry>,
    /// Action items the hero owns.
    pub items: Vec<ActionItemEntry>,
    /// Bonus maps the hero owns.
    pub bonus_maps: Vec<BonusMapEntry>,
    /// The world map: every map the world map window shows.
    pub atlas: Vec<AtlasEntry>,
}

/// A map cell of the game's world map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtlasEntry {
    pub id: i16,
    pub name: String,
    /// 0: Nautica (surface), 1: Atlantis and so on, as the game orders them.
    pub world: u8,
    pub row: u8,
    pub col: u8,
    pub safe_haven: bool,
    pub min_level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonusMapEntry {
    pub id: i16,
    pub name: String,
    pub amount: i64,
    pub waves: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItemEntry {
    pub id: i16,
    pub name: String,
    pub amount: i32,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub type_id: i32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemEntry {
    pub id: i16,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventCode {
    AgentReady,
    BindingMissing,
    SessionReady,
    MapChanged,
    TargetEngaged,
    TargetKilled,
    TargetSkipped,
    BoxCollected,
    BoxSkipped,
    NoAmmo,
    Stuck,
    RouteInvalid,
    RepairStarted,
    RepairFinished,
    RepairStalled,
    Died,
    Respawning,
    RespawnFailed,
    Disconnected,
    Reconnecting,
    ReconnectBlocked,
    Reconnected,
    TypesDumped,
    Fault,
    Unloading,
    SafeHaven,
    NoHarpoonRange,
    WarTarget,
    ItemUsed,
    ItemOff,
    ElDoradoReady,
    ElDoradoTriggered,
    ElDoradoEntered,
    ElDoradoLeft,
    ElDoradoFailed,
    BonusEntering,
    BonusEntered,
    BonusWave,
    BonusFinished,
    BonusFailed,
    BonusQueueDone,
    WorkmapTravel,
    WorkmapTeleport,
    WorkmapArrived,
    WorkmapFailed,
    /// The game clock ran backwards; the bot paused (anti-cheat signal).
    ClockAnomaly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub level: Level,
    pub code: EventCode,
    pub args: Vec<String>,
}

impl Event {
    pub fn new(level: Level, code: EventCode) -> Self {
        Self {
            level,
            code,
            args: Vec::new(),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn arg(mut self, value: impl ToString) -> Self {
        self.args.push(value.to_string());
        self
    }
}

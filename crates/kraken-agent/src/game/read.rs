use std::collections::HashMap;

use kraken_proto::{EntityKind, Point, Stock, StockKind};

use super::Game;
use super::projection::Affine;
use crate::engine::world::{Actor, Eid, HeroState, MapKind, MapState, Relation, RepairBlock, Terrain, World};
use crate::il2cpp::{Class, Method, Ptr, Slot, boxed_bool, read_prim_pair, read_string, unbox};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Default)]
pub struct Scan {
    pub world: World,
    pub hero_world: Option<Vec3>,
    pub actor_world: HashMap<Eid, Vec3>,
    pub map_to_world: Option<Affine>,
    pub diag: Vec<String>,
}

struct Components {
    type_id: Option<i64>,
    name: Option<String>,
    position: Option<Point>,
    repairing: Option<bool>,
    health_full: Option<bool>,
    carpenter: Option<bool>,
    guild: Option<super::war::GuildKey>,
}

struct Attributes {
    hp: i64,
    cannon: Option<i64>,
    harpoon: Option<i64>,
    vp: Option<i64>,
}

#[derive(Clone, Copy, Default)]
struct Values {
    hp: Option<f64>,
    cannon: Option<f64>,
    harpoon: Option<f64>,
    vp: Option<f64>,
}

fn kind_of(name: &str) -> EntityKind {
    match name {
        "User" => EntityKind::Player,
        "Npc" => EntityKind::Npc,
        "Monster" => EntityKind::Monster,
        "Box" => EntityKind::Box,
        "AreaEffect" => EntityKind::AreaEffect,
        "Tower" => EntityKind::Tower,
        _ => EntityKind::Unknown,
    }
}

fn map_kind(name: &str) -> MapKind {
    match name {
        "SAFEHAVEN" => MapKind::SafeHaven,
        "DEATH" => MapKind::Death,
        "TUTORIAL" | "MINIGAME" => MapKind::Special,
        _ => MapKind::Normal,
    }
}

impl Game {
    fn invoke(&self, method: Method, instance: Ptr, args: &mut [Ptr]) -> Ptr {
        method.invoke(instance, args).unwrap_or(std::ptr::null_mut())
    }

    pub fn connected(&self, network: Ptr) -> bool {
        boxed_bool(self.invoke(self.is_connected, network, &mut [])).unwrap_or(false)
    }

    fn world_position(&self, view: Ptr) -> Option<Vec3> {
        let transform = self.invoke(self.unity.get_transform, view, &mut []);
        if transform.is_null() {
            return None;
        }
        let boxed = self.invoke(self.unity.get_position, transform, &mut []);
        if boxed.is_null() {
            return None;
        }
        let raw = unbox(boxed).cast::<f32>();
        unsafe {
            Some(Vec3 {
                x: raw.read_unaligned(),
                y: raw.add(1).read_unaligned(),
                z: raw.add(2).read_unaligned(),
            })
        }
    }

    fn map_point(&self, world: Vec3) -> Option<Point> {
        let mut vector = [world.x, world.y];
        let boxed = self.invoke(
            self.map_to_field,
            std::ptr::null_mut(),
            &mut [vector.as_mut_ptr().cast()],
        );
        let class = Class::of(boxed)?;
        let (x, y) = read_prim_pair(unbox(boxed), class)?;
        Some(Point::new(x as f32, y as f32))
    }

    pub fn screen_point(&self, world: Vec3) -> Option<Point> {
        let main = self.unity.camera_main?;
        let project = self.unity.world_to_screen?;
        let camera = self.invoke(main, std::ptr::null_mut(), &mut []);
        if camera.is_null() {
            return None;
        }
        let mut vector = [world.x, world.y, world.z];
        let boxed = self.invoke(project, camera, &mut [vector.as_mut_ptr().cast()]);
        if boxed.is_null() {
            return None;
        }
        let raw = unbox(boxed).cast::<f32>();
        let (x, y, z) = unsafe {
            (
                raw.read_unaligned(),
                raw.add(1).read_unaligned(),
                raw.add(2).read_unaligned(),
            )
        };
        (z >= 0.0).then_some(Point::new(x, y))
    }

    pub fn screen_size(&self) -> Option<(f32, f32)> {
        let read = |m: Option<Method>| -> Option<f32> {
            let boxed = self.invoke(m?, std::ptr::null_mut(), &mut []);
            (!boxed.is_null()).then(|| unsafe { unbox(boxed).cast::<i32>().read_unaligned() } as f32)
        };
        Some((read(self.unity.screen_width)?, read(self.unity.screen_height)?))
    }

    fn components(&mut self, actor: Ptr) -> Components {
        let mut out = Components {
            type_id: None,
            name: None,
            position: None,
            repairing: None,
            health_full: None,
            carpenter: None,
            guild: None,
        };
        let table = self.obj(actor, "components");
        let entries = self.collections.dict(table);
        for (_, value) in entries {
            let component = value.object();
            let Some(class) = Class::of(component) else {
                continue;
            };
            match class.name() {
                "NpcData" => out.type_id = self.int(component, "npcid"),
                "MonsterData" => out.type_id = self.int(component, "monsterid"),
                "BoxData" => {
                    out.type_id = self.int(component, "boxtypeid");
                    if out.name.is_none() {
                        out.name = out.type_id.and_then(|id| self.box_name(id));
                    }
                }
                "GameActorNameData" => {
                    let name = read_string(self.obj(component, "name"));
                    if !name.is_empty() {
                        out.name = Some(name);
                    }
                }
                "GameActorRepairData" => {
                    out.repairing = self.int(component, "isrepairing").map(|v| v != 0);
                    out.health_full = self.int(component, "healthfull").map(|v| v != 0);
                    out.carpenter = self.int(component, "hasanycarpenter").map(|v| v != 0);
                }
                "GameActorGuildData" => out.guild = self.actor_guild(component),
                "PositionStub" => {
                    if let (Some(x), Some(y)) = (self.num(component, "x"), self.num(component, "y")) {
                        out.position = Some(Point::new(x as f32, y as f32));
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn box_name(&mut self, id: i64) -> Option<String> {
        let class = self.box_types?;
        self.enum_name(class.ty(), id).map(|n| super::pretty(&n))
    }

    fn attributes(&mut self, values: Ptr, codes: &Attributes) -> Values {
        let mut out = Values::default();
        for (key, value) in self.collections.dict(values) {
            let Some(code) = key.i64() else { continue };
            let number = value.f64().or_else(|| {
                let object = value.object();
                self.num(object, "value")
            });
            if code == codes.hp {
                out.hp = number;
            } else if Some(code) == codes.cannon {
                out.cannon = number;
            } else if Some(code) == codes.harpoon {
                out.harpoon = number;
            } else if Some(code) == codes.vp {
                out.vp = number;
            }
        }
        out
    }

    fn attribute_codes(&mut self, sample: Option<Slot>) -> Option<Attributes> {
        let ty = sample?.ty;
        Some(Attributes {
            hp: self.enum_value(ty, "HITPOINTS")?,
            cannon: self.enum_value(ty, "CANNONRANGE"),
            harpoon: self.enum_value(ty, "HARPOONRANGE"),
            vp: self.enum_value(ty, "VOODOOPOINTS"),
        })
    }

    fn ams_table(&mut self, ams: Ptr, name: &'static str) -> Vec<(Eid, Ptr)> {
        let table = self.obj(ams, name);
        let entries = self.collections.dict(table);
        entries
            .into_iter()
            .filter_map(|(k, v)| Some((self.read_eid(k)?, v.object())))
            .collect()
    }

    fn stock(&mut self, inventory: Ptr) -> Option<Vec<Stock>> {
        let table = self.obj(inventory, "inventory");
        if table.is_null() {
            return None;
        }
        let mut out = Vec::new();
        for (kind, items) in self.collections.dict(table) {
            let Some(code) = kind.i64() else { continue };
            let stock_kind = match self.enum_name(kind.ty, code).as_deref() {
                Some("BALLS") => StockKind::Ammo,
                Some("HARPOONS") => StockKind::Harpoon,
                Some("CURRENCY") => StockKind::Currency,
                Some("POINTS") => StockKind::Points,
                Some("TOKENS") => StockKind::Token,
                Some("BONUSMAPS") => StockKind::BonusMap,
                _ => continue,
            };
            for (id, amount) in self.collections.dict(items.object()) {
                let (Some(id), Some(amount)) =
                    (id.i64(), amount.i64().or_else(|| amount.f64().map(|v| v as i64)))
                else {
                    continue;
                };
                out.push(Stock {
                    kind: stock_kind,
                    id: id as i16,
                    amount,
                });
            }
        }
        Some(out)
    }

    /// Mirrors `ActionMenuPanel.CanRepair`: full hull, no carpenter, a hull
    /// breach, an attack in progress or the repair cooldown all refuse.
    fn repair_block(
        &mut self,
        info: Ptr,
        repairing: Option<bool>,
        health_full: Option<bool>,
        carpenter: Option<bool>,
    ) -> Option<RepairBlock> {
        if health_full == Some(true) {
            return Some(RepairBlock::HealthFull);
        }
        if carpenter == Some(false) {
            return Some(RepairBlock::NoCarpenter);
        }
        if self.int(info, "isrepairpreventedbybreach") == Some(1) {
            return Some(RepairBlock::Breach);
        }
        let attack = self.obj(info, "attacksystem");
        if self.int(attack, "attackinprogress") == Some(1) {
            return Some(RepairBlock::Attacking);
        }
        if repairing != Some(true) && self.repair_cooling_down() == Some(true) {
            return Some(RepairBlock::Cooldown);
        }
        None
    }

    fn movement(&mut self, view: Ptr) -> (Option<bool>, Option<bool>) {
        let (Some(get), Some(class)) = (self.unity.get_component, self.movement_class) else {
            return (None, None);
        };
        let component = self.invoke(get, view, &mut [class.type_object()]);
        if component.is_null() {
            return (None, None);
        }
        let moving = self.int(component, "ismoving").map(|v| v != 0);
        let enabled = self.int(component, "ismovementenabled").map(|v| v != 0);
        (moving, enabled)
    }

    pub fn scan(&mut self, network: Ptr) -> Scan {
        let mut scan = Scan::default();
        scan.world.connected = self.connected(network);
        if !scan.world.connected {
            return scan;
        }

        let map = self.map_current.static_ptr();
        if !map.is_null() {
            let kind_name = Class::of(map)
                .and_then(|c| self.field(c, "type"))
                .and_then(|f| {
                    let value = unsafe { crate::il2cpp::get_i64(map, f) }?;
                    self.enum_name(f.ty(), value)
                })
                .unwrap_or_default();
            scan.world.map = Some(MapState {
                id: self.int(map, "mapid").unwrap_or(0) as i32,
                kind: map_kind(&kind_name),
                kind_name,
                width: self.num(map, "width").unwrap_or(0.0) as f32,
                height: self.num(map, "height").unwrap_or(0.0) as f32,
            });
        }

        let handler = self.obj(network, "userinitmessagehandler");
        let model = self.obj(handler, "gameactormodel");
        let info = self.obj(handler, "playerinfosystem");
        scan.diag.push(format!(
            "handler={} model={} info={}",
            !handler.is_null(),
            !model.is_null(),
            !info.is_null()
        ));
        if model.is_null() || info.is_null() {
            return scan;
        }
        let Some(hero_id) = Class::of(info)
            .and_then(|c| self.field(c, "userid"))
            .and_then(|f| self.eid_field(info, f))
        else {
            scan.diag.push("hero_id=none".into());
            return scan;
        };
        scan.diag.push(format!("hero_id={hero_id:?}"));

        let actors_table = self.obj(model, "actors");
        let mut actors: HashMap<Eid, (EntityKind, Components)> = HashMap::new();
        for (key, value) in self.collections.dict(actors_table) {
            let Some(id) = self.read_eid(key) else { continue };
            let actor = value.object();
            let Some(class) = Class::of(actor) else { continue };
            let kind = self
                .field(class, "gameactortype")
                .and_then(|f| {
                    let value = unsafe { crate::il2cpp::get_i64(actor, f) }?;
                    self.enum_name(f.ty(), value)
                })
                .map_or(EntityKind::Unknown, |n| kind_of(&n));
            if !matches!(
                kind,
                EntityKind::Player | EntityKind::Npc | EntityKind::Monster | EntityKind::Box
            ) {
                continue;
            }
            let components = self.components(actor);
            actors.insert(id, (kind, components));
        }

        let mut kinds: HashMap<EntityKind, usize> = HashMap::new();
        for (kind, _) in actors.values() {
            *kinds.entry(*kind).or_default() += 1;
        }
        let mut kinds: Vec<String> = kinds.into_iter().map(|(k, n)| format!("{k:?}:{n}")).collect();
        kinds.sort();
        scan.diag.push(format!(
            "actor_table={} actors={} [{}] has_hero={}",
            !actors_table.is_null(),
            actors.len(),
            kinds.join(","),
            actors.contains_key(&hero_id)
        ));
        let map_view = self.obj(model, "mapview");
        let layer = self.obj(map_view, "gameactorlayer");
        let views_table = self.obj(layer, "actors");
        let mut pairs = Vec::new();
        let mut hero_view = std::ptr::null_mut();
        let mut positions: HashMap<Eid, Point> = HashMap::new();
        let mut visible: HashMap<Eid, bool> = HashMap::new();
        for (key, value) in self.collections.dict(views_table) {
            let Some(id) = self.read_eid(key) else { continue };
            let is_hero = id.matches(hero_id);
            let entry = actors.get(&id);
            if entry.is_none() && !is_hero {
                continue;
            }
            let view = value.object();
            if let Some((kind, components)) = entry {
                let shown = self.int(view, "isgameactorvisible").is_none_or(|v| v != 0);
                visible.insert(id, shown);
                if !is_hero && *kind == EntityKind::Box && components.position.is_some() {
                    continue;
                }
            }
            let Some(world) = self.world_position(view) else {
                continue;
            };
            let Some(point) = self.map_point(world) else {
                continue;
            };
            if is_hero {
                hero_view = view;
                scan.hero_world = Some(world);
            }
            scan.actor_world.insert(id, world);
            pairs.push((point, Point::new(world.x, world.y)));
            positions.insert(id, point);
        }
        scan.map_to_world = Affine::fit(&pairs);
        scan.diag.push(format!(
            "views_table={} positions={} visible={}/{} hero_view={} fit={}",
            !views_table.is_null(),
            positions.len(),
            visible.values().filter(|v| **v).count(),
            visible.len(),
            !hero_view.is_null(),
            scan.map_to_world.is_some()
        ));

        let ams = self.obj(handler, "amsvaluestorage");
        let current = self.ams_table(ams, "amscurrentvalues");
        let permanent = self.ams_table(ams, "amspermanentvalues");
        let sample = current
            .iter()
            .find_map(|(_, inner)| self.collections.dict(*inner).first().map(|(k, _)| *k));
        let codes = self.attribute_codes(sample);
        scan.diag.push(format!(
            "ams={} current={} permanent={} sample_ty={} codes={:?} hero_in_current={}",
            !ams.is_null(),
            current.len(),
            permanent.len(),
            sample.map_or_else(|| "none".to_owned(), |k| k.ty.name()),
            codes.as_ref().map(|c| (c.hp, c.cannon, c.harpoon)),
            current.iter().any(|(id, _)| id.matches(hero_id))
        ));
        let mut health: HashMap<Eid, (Option<f64>, Option<f64>)> = HashMap::new();
        let mut hero_now = Values::default();
        let mut hero_base = Values::default();
        if let Some(codes) = codes {
            for (id, inner) in &current {
                if !id.matches(hero_id) && !actors.contains_key(id) {
                    continue;
                }
                let values = self.attributes(*inner, &codes);
                health.entry(*id).or_default().0 = values.hp;
                if id.matches(hero_id) {
                    hero_now = values;
                }
            }
            for (id, inner) in &permanent {
                if !id.matches(hero_id) && !actors.contains_key(id) {
                    continue;
                }
                let values = self.attributes(*inner, &codes);
                health.entry(*id).or_default().1 = values.hp;
                if id.matches(hero_id) {
                    hero_base = values;
                }
            }
        }

        if let Some(hero_pos) = positions.get(&hero_id).copied() {
            let (hp, hp_max) = health.get(&hero_id).copied().unwrap_or_default();
            let (moving, movement_enabled) = if hero_view.is_null() {
                (None, None)
            } else {
                self.movement(hero_view)
            };
            let username = self.obj(info, "username");
            let name = read_string(self.obj(username, "value"));
            let hero_parts = actors
                .get(&hero_id)
                .map(|(_, c)| (c.repairing, c.health_full, c.carpenter));
            let (repairing, health_full, carpenter) = hero_parts.unwrap_or_default();
            let repair_block = self.repair_block(info, repairing, health_full, carpenter);
            let hp_known = hp.is_some() && hp_max.is_some_and(|m| m > 0.0);
            let hp = hp.unwrap_or(0.0);
            let range = |current: Option<f64>, base: Option<f64>| {
                current.filter(|v| *v > 0.0).or(base).unwrap_or(0.0) as f32
            };
            scan.world.hero = Some(HeroState {
                id: hero_id,
                name,
                pos: hero_pos,
                hp,
                hp_max: hp_max.unwrap_or(0.0).max(hp),
                hp_known,
                moving,
                movement_enabled,
                cannon_range: range(hero_now.cannon, hero_base.cannon),
                harpoon_range: range(hero_now.harpoon, hero_base.harpoon),
                repairing,
                repair_block,
                base_cannon_range: hero_base.cannon.unwrap_or(0.0) as f32,
                vp: hero_now.vp.unwrap_or(0.0),
                vp_max: hero_base.vp.or(hero_now.vp).unwrap_or(0.0),
                selected: self.selected(model),
            });
        }

        let players = actors.values().any(|(k, _)| *k == EntityKind::Player);
        let (guilds, group) = if players {
            (self.guilds(model), self.group(hero_view))
        } else {
            Default::default()
        };

        for (id, (kind, components)) in actors {
            if id.matches(hero_id) {
                continue;
            }
            let Some(pos) = positions.get(&id).copied().or(components.position) else {
                continue;
            };
            let type_id = components.type_id.unwrap_or(0) as i32;
            let hp_percent = health.get(&id).and_then(|(hp, max)| {
                let (hp, max) = ((*hp)?, (*max)?);
                (max > 0.0).then(|| (hp / max * 100.0) as f32)
            });
            scan.world.actors.push(Actor {
                id,
                kind,
                type_id,
                name: components.name.unwrap_or_else(|| format!("#{type_id}")),
                pos,
                hp_percent,
                visible: visible.get(&id).copied().unwrap_or(false),
                relation: if kind == EntityKind::Player {
                    Self::relation(&guilds, &group, id, components.guild)
                } else {
                    Relation::Unknown
                },
            });
        }

        let inventory = self.obj(handler, "inventorysystem");
        if let Some(stock) = self.stock(inventory) {
            scan.world.stock = stock;
            scan.world.stock_known = !scan.world.stock.is_empty();
        }
        let attack = self.obj(info, "attacksystem");
        scan.world.items = self.cached_items(attack);
        scan.world.skills = self.cached_skills(network);
        scan.world.bonus = self.bonus_maps(inventory, &scan.world.stock);
        (scan.world.teleports, scan.world.teleport_ready) = self.teleport_stock;
        scan.world.atlas = self.atlas();
        scan.world.ready = scan.world.map.is_some() && scan.world.hero.is_some();
        scan
    }

    pub fn terrain(&self) -> GameTerrain<'_> {
        let instance = self
            .map_view_instance
            .map_or(std::ptr::null_mut(), |f| f.static_ptr());
        GameTerrain { game: self, instance }
    }
}

pub struct GameTerrain<'a> {
    game: &'a Game,
    instance: Ptr,
}

impl Terrain for GameTerrain<'_> {
    fn blocked(&self, point: Point) -> Option<bool> {
        let method = self.game.coord_blocked?;
        if self.instance.is_null() {
            return None;
        }
        let mut cell = [point.x.round() as i32, point.y.round() as i32];
        let boxed = method
            .invoke(self.instance, &mut [cell.as_mut_ptr().cast()])
            .ok()?;
        boxed_bool(boxed)
    }
}

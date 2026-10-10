//! Master skills (skill tree): their charge, and the game's own trigger call.
//! El Dorado is the one jumping to the bonus map (`MAP_JUMP`).

use std::time::{Duration, Instant};

use kraken_proto::ELDORADO_SKILL;

use super::Game;
use crate::engine::world::MasterSkill;
use crate::il2cpp::{Field, Method, Ptr};

/// Charge changes slowly; read it this often.
const SKILLS_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Default)]
pub struct SkillBindings {
    /// `SkillTreeSystem.RequestMasterSkillTrigger(int)`, what the skill button calls.
    pub trigger: Option<Method>,
    /// `DefaultMasterSkillsConfigurationModel.DefaultMasterSkillStubs` (static).
    pub defaults: Option<Field>,
}

impl SkillBindings {
    pub fn resolve(registry: &crate::il2cpp::Registry, missing: &mut Vec<String>) -> Self {
        let find = |name: &str| registry.find(name, Some(super::GAME));
        let trigger =
            find("Seafight.SkillTree.SkillTreeSystem").and_then(|c| c.method("RequestMasterSkillTrigger", 1));
        let defaults = find("Seafight.Storage.DefaultMasterSkillsConfigurationModel")
            .and_then(|c| c.field("DefaultMasterSkillStubs"));
        if trigger.is_none() {
            missing.push("SkillTreeSystem.RequestMasterSkillTrigger".into());
        }
        Self { trigger, defaults }
    }
}

#[derive(Default)]
pub struct SkillCache {
    read_at: Option<Instant>,
    skills: Vec<MasterSkill>,
    /// The skill whose effect is `MAP_JUMP`; looked up once.
    eldorado: Option<i32>,
    /// `DefaultMasterSkillStub.maxCharge` per skill: the charge is in points.
    max_charge: std::collections::HashMap<i32, i64>,
}

impl Game {
    fn skill_system(&mut self, network: Ptr) -> Ptr {
        let handler = self.obj(network, "userinitmessagehandler");
        self.obj(handler, "skilltreesystem")
    }

    /// The map-jump skill from the static skill configuration, else the
    /// known El Dorado id.
    fn eldorado_id(&mut self) -> i32 {
        if let Some(id) = self.skills.eldorado {
            return id;
        }
        let Some(field) = self.skill_bindings.defaults else {
            return ELDORADO_SKILL;
        };
        let table = field.static_ptr();
        let mut found = None;
        for (key, stub) in self.collections.dict(table) {
            let (Some(id), stub) = (key.i64(), stub.object()) else {
                continue;
            };
            if let Some(max) = self.int(stub, "maxcharge").filter(|m| *m > 0) {
                self.skills.max_charge.insert(id as i32, max);
            }
            let effects = self.obj(stub, "effectsdata");
            let jumps = self.collections.list(effects).into_iter().any(|effect| {
                let effect = effect.object();
                let Some(class) = crate::il2cpp::Class::of(effect) else {
                    return false;
                };
                let Some(field) = self.field(class, "effecttype") else {
                    return false;
                };
                let value = unsafe { crate::il2cpp::get_i64(effect, field) };
                value.and_then(|v| self.enum_name(field.ty(), v)).as_deref() == Some("MAP_JUMP")
            });
            if jumps {
                found = Some(id as i32);
                break;
            }
        }
        if let Some(id) = found {
            tracing::info!(id, "El Dorado skill (MAP_JUMP)");
            self.skills.eldorado = Some(id);
            id
        } else {
            // Not cached: the configuration may still be loading.
            ELDORADO_SKILL
        }
    }

    fn read_skills(&mut self, network: Ptr) -> Vec<MasterSkill> {
        let system = self.skill_system(network);
        let model = self.obj(system, "skilltreemodel");
        let table = self.obj(model, "masterskilldatastubs");
        let eldorado = self.eldorado_id();
        let mut out = Vec::new();
        for (key, stub) in self.collections.dict(table) {
            let stub = stub.object();
            let Some(id) = key.i64().or_else(|| self.int(stub, "masterskillid")) else {
                continue;
            };
            let id = id as i32;
            // Points; the skill button shows them as a share of `maxCharge`.
            let points = self.int(stub, "currentcharge").unwrap_or(0).max(0);
            let max = self.skills.max_charge.get(&id).copied().unwrap_or(0);
            let charge = if max > 0 {
                (points.saturating_mul(100) / max).clamp(0, 100) as u8
            } else {
                0
            };
            out.push(MasterSkill {
                id,
                charge,
                points,
                max_charge: max,
                active: self.int(stub, "active").is_some_and(|v| v != 0),
                remaining_ms: self.int(stub, "remainingtriggerduration").unwrap_or(0).max(0) as u64,
                eldorado: id == eldorado,
            });
        }
        out.sort_by_key(|s| s.id);
        out
    }

    pub fn cached_skills(&mut self, network: Ptr) -> Vec<MasterSkill> {
        if self.skills.read_at.is_none_or(|t| t.elapsed() >= SKILLS_EVERY) {
            let skills = self.read_skills(network);
            self.skills.read_at = Some(Instant::now());
            let eldorado = |list: &[MasterSkill]| list.iter().find(|s| s.eldorado).copied();
            let (before, after) = (eldorado(&self.skills.skills), eldorado(&skills));
            if before != after {
                tracing::info!(?after, "El Dorado skill state");
            }
            self.skills.skills = skills;
        }
        self.skills.skills.clone()
    }

    /// The same call as the skill's button.
    pub fn trigger_skill(&mut self, network: Ptr, id: i32) -> Result<(), String> {
        let method = self
            .skill_bindings
            .trigger
            .ok_or("RequestMasterSkillTrigger unavailable")?;
        let system = self.skill_system(network);
        if system.is_null() {
            return Err("no skill tree system".into());
        }
        let mut raw = id;
        let result = method.invoke(system, &mut [(&raw mut raw).cast()]).map(|_| ());
        self.skills.read_at = None;
        result
    }
}

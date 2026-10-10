use super::Game;
use crate::il2cpp::{Class, Ptr};

const UI_IMAGE: &str = "CakeBox";

impl Game {
    /// Every view currently open on any of the game's UI layers.
    fn active_views(&mut self) -> Vec<Ptr> {
        let Some(system) = self
            .registry
            .find("CakeBox.ScreenManagement.UISystem", Some(UI_IMAGE))
        else {
            return Vec::new();
        };
        let Some(layers) = system.field("uiLayersDictionary").map(|f| f.static_ptr()) else {
            return Vec::new();
        };
        let mut views = Vec::new();
        for (_, layer) in self.collections.dict(layers) {
            let active = self.obj(layer.object(), "activeviews");
            views.extend(
                self.collections
                    .dict(active)
                    .into_iter()
                    .map(|(_, view)| view.object()),
            );
        }
        views
    }

    pub fn view(&mut self, class_name: &str) -> Option<Ptr> {
        self.active_views()
            .into_iter()
            .find(|v| Class::of(*v).is_some_and(|c| c.name() == class_name))
    }

    /// The "your ship sank" window, while it is open.
    pub fn death_window(&mut self) -> Option<Ptr> {
        let window = self.view("DeathWindowBehaviour")?;
        self.int(window, "isopen")
            .is_none_or(|v| v != 0)
            .then_some(window)
    }

    pub fn call(&self, object: Ptr, name: &str) -> Result<(), String> {
        let class = Class::of(object).ok_or("null view")?;
        let method = class
            .method(name, 0)
            .ok_or_else(|| format!("{}.{name} missing", class.name()))?;
        method.invoke(object, &mut []).map(|_| ())
    }

    /// The action panel's repair cooldown, as the repair button shows it.
    pub fn repair_cooling_down(&mut self) -> Option<bool> {
        let panel = self.view("ActionMenuPanel")?;
        let cooldown = self.obj(panel, "repaircooldown");
        self.int(cooldown, "iscoolingdown").map(|v| v != 0)
    }
}

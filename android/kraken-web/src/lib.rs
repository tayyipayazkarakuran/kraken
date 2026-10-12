//! Kraken for the browser build of the game (Android WebView, desktop
//! Chrome). The decision engine is the same one the Windows agent runs; what
//! differs is how the world is read and how actions reach the game, which on
//! the web goes through the Unity module's linear memory and function table
//! (see `js/kraken-boot.js`).
//!
//! Phase 1: the engine is wired up and driven from JS, but the world scan is
//! still empty, so a running engine only idles.

use kraken_agent::engine::Engine;
use kraken_agent::engine::world::{Action, OpenWater, World};
use kraken_proto::{Capabilities, Config, Control, RunState};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[wasm_bindgen]
pub struct Kraken {
    engine: Engine,
    world: World,
    caps: Capabilities,
}

#[wasm_bindgen]
impl Kraken {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64) -> Self {
        Self {
            engine: Engine::new(seed),
            world: World::default(),
            caps: Capabilities::default(),
        }
    }

    /// Applies a postcard-encoded `Config`, the same bytes the Windows app
    /// sends to its agent.
    pub fn configure(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let config: Config = postcard::from_bytes(bytes)?;
        self.engine.configure(config);
        Ok(())
    }

    /// `"start"`, `"pause"` or `"stop"`; returns the actions to carry out.
    pub fn control(&mut self, now: u64, command: &str) -> Result<Vec<String>, JsError> {
        let control = match command {
            "start" => Control::Start,
            "pause" => Control::Pause,
            "stop" => Control::Stop,
            other => return Err(JsError::new(&format!("unknown control {other:?}"))),
        };
        Ok(describe(&self.engine.control(now, control, &self.world)))
    }

    /// One engine step on the last scanned world; returns the actions to
    /// carry out.
    pub fn tick(&mut self, now: u64) -> Vec<String> {
        describe(&self.engine.step(now, &self.world, &self.caps, &OpenWater))
    }

    #[wasm_bindgen(getter)]
    pub fn running(&self) -> bool {
        self.engine.run_state() == RunState::Running
    }
}

/// Until the web action layer exists, actions go back to JS as text for the
/// console.
fn describe(actions: &[Action]) -> Vec<String> {
    actions.iter().map(|action| format!("{action:?}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_runs_on_an_empty_world() {
        let mut kraken = Kraken::new(7);
        let config = postcard::to_stdvec(&Config::default()).unwrap();
        kraken.configure(&config).unwrap();
        kraken.control(0, "start").unwrap();
        assert!(kraken.running());
        for now in (0..2_000).step_by(100) {
            kraken.tick(now);
        }
        kraken.control(2_000, "stop").unwrap();
        assert!(!kraken.running());
    }
}

//! In-game interface. The app lays out and tessellates the egui interface
//! and sends the meshes; this module draws them into the game's own D3D11
//! swap chain (a `Present` hook) and forwards the game window's mouse and
//! keyboard back to the app (a window-procedure subclass).

mod hook;
mod input;
mod render;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, LazyLock};

use kraken_proto::{AgentMessage, UiFrame, UiInput, UiMesh, UiRect, UiTexture};
use parking_lot::Mutex;

/// The latest frame from the app, shared between the pipe reader, the
/// render thread and the window thread.
#[derive(Default)]
struct Scene {
    width: u32,
    height: u32,
    meshes: Arc<Vec<UiMesh>>,
    /// Bumped whenever `meshes` changes, so the renderer re-uploads.
    version: u64,
    capture: Vec<UiRect>,
    keyboard: bool,
    textures: Vec<UiTexture>,
    freed: Vec<u64>,
}

static SCENE: LazyLock<Mutex<Scene>> = LazyLock::new(Mutex::default);
static OUTBOX: Mutex<Option<SyncSender<AgentMessage>>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Hooks the swap chain. Failure only costs the in-game interface.
pub fn install(outbox: SyncSender<AgentMessage>) {
    *OUTBOX.lock() = Some(outbox);
    match hook::install() {
        Ok(()) => {
            ACTIVE.store(true, Ordering::Release);
            tracing::info!("overlay hooks installed");
        }
        Err(error) => tracing::warn!(%error, "overlay unavailable"),
    }
}

pub fn shutdown() {
    ACTIVE.store(false, Ordering::Release);
    input::detach();
    hook::uninstall();
    render::release();
}

/// A new frame from the app.
pub fn submit(frame: UiFrame) {
    let mut scene = SCENE.lock();
    scene.textures.extend(frame.textures);
    scene.freed.extend(frame.freed);
    scene.width = frame.width;
    scene.height = frame.height;
    scene.meshes = Arc::new(frame.meshes);
    scene.version = scene.version.wrapping_add(1);
    scene.capture = frame.capture;
    scene.keyboard = frame.keyboard;
}

/// Forgets the interface (the app went away): nothing is drawn or captured.
pub fn clear() {
    let mut scene = SCENE.lock();
    scene.meshes = Arc::new(Vec::new());
    scene.version = scene.version.wrapping_add(1);
    scene.capture.clear();
    scene.keyboard = false;
}

fn send(input: UiInput) {
    if let Some(outbox) = OUTBOX.lock().as_ref() {
        let _ = outbox.try_send(AgentMessage::Input(input));
    }
}

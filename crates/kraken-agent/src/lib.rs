#![allow(
    clippy::missing_safety_doc,
    clippy::missing_transmute_annotations,
    clippy::not_unsafe_ptr_arg_deref,
    reason = "thin FFI layer over pointers owned by the IL2CPP runtime"
)]

pub mod engine;
pub mod game;
pub mod il2cpp;

mod dump;
mod hide;
mod ipc;
mod overlay;
mod pump;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use kraken_proto::{AgentMessage, AppMessage, BindingReport, PROTO_VERSION};
use windows_sys::Win32::Foundation::{HMODULE, TRUE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

const OUTBOX: usize = 1024;

fn logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = dump::data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let file = tracing_appender::rolling::never(&dir, format!("agent-{}.log", std::process::id()));
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false)
        .try_init()
        .ok()?;
    Some(guard)
}

fn wait_for<T>(timeout: Duration, interval: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let started = Instant::now();
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if started.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(interval);
    }
}

fn fallback(
    inbox: &mpsc::Receiver<AppMessage>,
    outbox: &mpsc::SyncSender<AgentMessage>,
    missing: Vec<String>,
) {
    let report = BindingReport {
        missing,
        ..BindingReport::default()
    };
    while let Ok(message) = inbox.recv() {
        if let AppMessage::Hello { .. } = message {
            let _ = outbox.try_send(AgentMessage::Hello {
                version: PROTO_VERSION,
                pid: std::process::id(),
            });
            let _ = outbox.try_send(AgentMessage::Bindings(report.clone()));
        }
    }
}

fn boot(module: usize) {
    let guard = logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "agent starting");
    let (inbox_tx, inbox) = mpsc::channel();
    let (outbox, outbox_rx) = mpsc::sync_channel(OUTBOX);
    let pid = std::process::id();
    std::thread::spawn(move || ipc::serve(pid, &inbox_tx, &outbox_rx));

    let ready = wait_for(Duration::from_secs(600), Duration::from_millis(500), || {
        let api = il2cpp::load().ok()?;
        let domain = unsafe { (api.il2cpp_domain_get)() };
        if domain.is_null() {
            return None;
        }
        unsafe { (api.il2cpp_thread_attach)(domain) };
        let registry = il2cpp::Registry::build();
        registry
            .find("Seafight.Network.NetworkManager", Some("Seafight"))
            .map(|_| registry)
    });
    let Some(registry) = ready else {
        tracing::error!("il2cpp runtime never became ready");
        fallback(&inbox, &outbox, vec!["il2cpp runtime".into()]);
        return;
    };
    tracing::info!(classes = registry.len(), "il2cpp registry built");
    let messages = game::messages::MessageMap::build(&registry);
    tracing::info!(rules = messages.len(), "message rules resolved");

    let game = match game::Game::resolve(registry) {
        Ok(game) => game,
        Err(missing) => {
            tracing::error!(?missing, "required bindings missing");
            fallback(&inbox, &outbox, missing);
            return;
        }
    };
    let (tick, dispatch) = (game.tick, game.dispatch);
    overlay::install(outbox.clone());
    let agent = pump::Agent::new(game, inbox, outbox);
    match pump::install(agent, messages, tick, dispatch) {
        Ok(()) => tracing::info!("hooks installed"),
        Err(error) => tracing::error!(%error, "hook installation failed"),
    }
    // Experimental: take the agent out of the loader's three module lists so
    // a scan next to the game's own DLLs no longer sees it. Done only after
    // the overlay and hooks have warmed up, so any module-base resolution
    // they do (the D3D hook building trampolines, DXGI resolving addresses)
    // already happened while the entry was still linked. This has crashed
    // the client before; a failure here only loses the hiding, not the bot.
    // Hide the agent from the loader's module lists once the overlay and
    // hooks have warmed up (verified stable live 2026-10-10 after the PEB
    // offset fix). On by default; drop an empty `hide.off` into the Kraken
    // data dir and re-inject to turn it back off.
    if !dump::data_dir().join("hide.off").exists() {
        std::thread::sleep(Duration::from_secs(3));
        hide::hide(module);
    }
    std::mem::forget(guard);
}

pub fn unload() {
    tracing::info!("agent unloading");
    std::thread::sleep(Duration::from_millis(300));
    overlay::shutdown();
    pump::shutdown();
    ipc::shutdown(std::process::id());
    tracing::info!("agent dormant");
}

/// Runtime helper names that belong to no game and no helper DLL: the
/// agent's exports look like the leftovers of a long-gone C runtime, which
/// is what a module list shows of it.
#[unsafe(no_mangle)]
pub extern "system" fn _dtoul3_legacy() {}

#[unsafe(no_mangle)]
pub extern "system" fn _ftoul2_legacy() {}

#[unsafe(no_mangle)]
pub extern "system" fn _ftoul3_legacy() {}

#[unsafe(no_mangle)]
pub extern "system" fn _ltof3() {}

#[unsafe(no_mangle)]
pub extern "system" fn _ultof3() {}

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut core::ffi::c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        unsafe { DisableThreadLibraryCalls(module) };
        let module = module as usize;
        std::thread::spawn(move || boot(module));
    }
    TRUE
}

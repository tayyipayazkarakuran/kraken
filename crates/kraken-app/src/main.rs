#![windows_subsystem = "windows"]

mod app;
mod clock;
mod gamedata;
mod i18n;
mod process;
mod profile;
mod remote;
mod supervisor;
mod theme;
mod ui;

rust_i18n::i18n!("locales", fallback = "en");

fn logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = profile::data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("kraken")
        .filename_suffix("log")
        .max_log_files(16)
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_env("KRAKEN_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok()?;
    Some(guard)
}

use std::iter::once;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
};

const INSTANCE: &str = "Kraken-SingleInstance";

/// Brings the running Kraken's window to the front; it starts hidden, so a
/// second start must show it rather than trust it is visible.
fn show_running() {
    let title: Vec<u16> = "Kraken".encode_utf16().chain(once(0)).collect();
    let window: HWND = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if window.is_null() {
        return;
    }
    unsafe {
        ShowWindowAsync(window, SW_RESTORE);
        SetForegroundWindow(window);
    }
}

/// One Kraken at a time: a second start shows the running one and stops.
/// Without this, every hidden start would open another game client.
fn single_instance() -> Option<HANDLE> {
    let name: Vec<u16> = INSTANCE.encode_utf16().chain(once(0)).collect();
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr()) };
    if mutex.is_null() {
        return Some(std::ptr::null_mut());
    }
    let taken = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
    if taken {
        return Some(mutex);
    }
    tracing::warn!("another Kraken is already running; showing its window");
    show_running();
    unsafe { windows_sys::Win32::Foundation::CloseHandle(mutex) };
    None
}

fn main() -> eframe::Result {
    let Some(_instance) = single_instance() else {
        return Ok(());
    };
    let _guard = logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "kraken starting");
    // Kraken runs without a window or console: a crash must at least reach the log.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(%info, %backtrace, "panic");
        default_hook(info);
    }));
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Kraken")
            // The interface lives inside the game; this window only appears
            // for faults the in-game panel cannot show.
            .with_visible(false)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([720.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Kraken",
        options,
        Box::new(|cc| Ok(Box::new(app::KrakenApp::new(cc)))),
    )
}

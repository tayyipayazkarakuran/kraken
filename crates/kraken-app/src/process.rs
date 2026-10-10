use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    GetProcessTimes, InitializeProcThreadAttributeList, OpenProcess, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
    PROCESS_CREATE_PROCESS, PROCESS_DUP_HANDLE, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, STARTUPINFOEXW, UpdateProcThreadAttribute,
};

pub const GAME_EXE: &str = "seafight.exe";
const STILL_ACTIVE: u32 = 259;
/// The right to wait on a handle (part of `STANDARD_RIGHTS_REQUIRED`).
const SYNCHRONIZE: u32 = 0x0010_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GameProcess {
    pub pid: u32,
    pub started: u64,
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32) -> Option<Handle> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    (!handle.is_null()).then_some(Handle(handle))
}

fn start_time(process: &Handle) -> Option<u64> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    let ok = unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) };
    (ok != 0).then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn image_path(process: &Handle) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut size) };
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
}

pub fn is_game_folder(exe: &Path) -> bool {
    let (Some(dir), Some(stem)) = (exe.parent(), exe.file_stem().and_then(|s| s.to_str())) else {
        return false;
    };
    dir.join("GameAssembly.dll").is_file()
        && dir
            .join(format!("{stem}_Data"))
            .join("il2cpp_data")
            .join("Metadata")
            .join("global-metadata.dat")
            .is_file()
}

fn game_process(pid: u32) -> Option<GameProcess> {
    let process = open(pid)?;
    if !is_game_folder(&image_path(&process)?) {
        return None;
    }
    Some(GameProcess {
        pid,
        started: start_time(&process)?,
    })
}

pub fn games() -> Vec<GameProcess> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case(GAME_EXE)
            && let Some(game) = game_process(entry.th32ProcessID)
        {
            found.push(game);
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    found.sort_by_key(|g| g.started);
    found
}

/// The login window the user starts from the Start menu: Squirrel's stub,
/// which opens the current `app-*` launcher. Pressing Play there starts the
/// game client, which the supervisor then finds like any other.
pub fn launcher_path() -> Option<PathBuf> {
    let path = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)?
        .join("Seafight")
        .join("Seafight.exe");
    path.is_file().then_some(path)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// explorer.exe: everything a user opens becomes its child, so the launcher
/// started under it looks like a double-clicked one. The handle carries the
/// rights `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` needs.
fn explorer_handle() -> Option<Handle> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case("explorer.exe") {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_CREATE_PROCESS | PROCESS_DUP_HANDLE | SYNCHRONIZE,
                    0,
                    entry.th32ProcessID,
                )
            };
            if !handle.is_null() {
                return Some(Handle(handle));
            }
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    None
}

/// Starts `exe` with explorer.exe as its parent process, so a watcher sees
/// the same tree a double-click produces. Returns false (nothing started)
/// when the spoof cannot be set up, so the caller can fall back to a plain
/// start; experimental, and failure only loses the disguise.
fn spawn_under_explorer(exe: &Path) -> bool {
    let Some(parent) = explorer_handle() else {
        return false;
    };
    let application = wide(&exe.to_string_lossy());
    let working = wide(
        &exe.parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let mut size = 0usize;
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    if size == 0 {
        return false;
    }
    let Ok(layout) = std::alloc::Layout::from_size_align(size, std::mem::align_of::<usize>()) else {
        return false;
    };
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        return false;
    }
    let list = buffer.cast::<c_void>();
    let parent_handle = parent.0;
    let started = unsafe {
        let ok = InitializeProcThreadAttributeList(list, 1, 0, &mut size) != 0
            && UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
                std::ptr::addr_of!(parent_handle).cast(),
                size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) != 0;
        let started = ok && {
            let mut startup: STARTUPINFOEXW = std::mem::zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = list;
            let mut info: PROCESS_INFORMATION = std::mem::zeroed();
            let created = CreateProcessW(
                application.as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                working.as_ptr(),
                std::ptr::addr_of!(startup.StartupInfo),
                &mut info,
            ) != 0;
            if created {
                CloseHandle(info.hProcess);
                CloseHandle(info.hThread);
            }
            created
        };
        DeleteProcThreadAttributeList(list);
        started
    };
    unsafe { std::alloc::dealloc(buffer, layout) };
    started
}

/// Opens the Seafight launcher. With `parent_spoof`, it is started under
/// explorer.exe; if that fails for any reason the launcher still opens
/// normally, so the login window always appears.
pub fn start_launcher(exe: &Path, parent_spoof: bool) -> std::io::Result<()> {
    if parent_spoof && spawn_under_explorer(exe) {
        return Ok(());
    }
    std::process::Command::new(exe).spawn().map(|_| ())
}

/// Any Seafight.exe that is not a game client is the (Electron) launcher.
pub fn launcher_running() -> bool {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return false;
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
        if name.eq_ignore_ascii_case(GAME_EXE)
            && let Some(path) = open(entry.th32ProcessID).and_then(|p| image_path(&p))
            && !is_game_folder(&path)
        {
            return true;
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    false
}

pub fn alive(game: GameProcess) -> bool {
    let Some(process) = open(game.pid) else {
        return false;
    };
    let mut code = 0u32;
    let running = unsafe { GetExitCodeProcess(process.0, &mut code) } != 0 && code == STILL_ACTIVE;
    if !running {
        return false;
    }
    // A client just started may not report its start time yet: the PID is
    // alive, so it stays until the next scan has it.
    match start_time(&process) {
        Some(started) => game.started == 0 || started == game.started,
        None => true,
    }
}

/// The start stamp of any process, for a client Kraken just started.
pub fn started_of(pid: u32) -> Option<u64> {
    let process = open(pid)?;
    start_time(&process)
}

struct OwnWindow {
    pid: u32,
    found: Vec<windows_sys::Win32::Foundation::HWND>,
}

unsafe extern "system" fn own_window(hwnd: windows_sys::Win32::Foundation::HWND, data: isize) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow, GetWindowThreadProcessId};
    let search = unsafe { &mut *(data as *mut OwnWindow) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == search.pid && unsafe { GetWindow(hwnd, GW_OWNER) }.is_null() {
        search.found.push(hwnd);
    }
    1
}

/// Shows or hides this process's own top-level windows (the Kraken window
/// only appears for faults; eframe insists on showing it after the first
/// frame).
pub fn show_own_windows(show: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, IsWindowVisible, SW_HIDE, SW_SHOW, ShowWindowAsync,
    };
    let mut search = OwnWindow {
        pid: std::process::id(),
        found: Vec::new(),
    };
    unsafe { EnumWindows(Some(own_window), &mut search as *mut OwnWindow as isize) };
    for hwnd in search.found {
        let visible = unsafe { IsWindowVisible(hwnd) } != 0;
        if visible != show {
            // Async: the window thread may be waiting for the core lock that
            // the caller holds; a synchronous call would deadlock.
            unsafe { ShowWindowAsync(hwnd, if show { SW_SHOW } else { SW_HIDE }) };
        }
    }
}

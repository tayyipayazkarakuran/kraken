use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_CREATE_PROCESS, PROCESS_CREATE_THREAD, PROCESS_DUP_HANDLE,
    PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_WRITE,
    QueryFullProcessImageNameW,
};

pub const GAME_EXE: &str = "Seafight.exe";
/// `STANDARD_RIGHTS_REQUIRED` territory: the right to wait on a handle.
const SYNCHRONIZE: u32 = 0x0010_0000;

pub struct Handle(pub HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// `1` usage, `2` access, `3` not the game, `4` not ready yet, `5` memory,
/// `6` thread, `7` load, `8` launch, `9` parent. The supervisor retries or
/// gives up by this number.
#[derive(Debug)]
pub enum Failure {
    Usage = 1,
    Open = 2,
    NotGame = 3,
    NotReady = 4,
    Memory = 5,
    Thread = 6,
    Load = 7,
    Launch = 8,
    Parent = 9,
}

impl Failure {
    pub fn code(self) -> u8 {
        self as u8
    }
}

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn process_name(entry: &PROCESSENTRY32W) -> String {
    let len = entry
        .szExeFile
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..len])
}

fn image_path(process: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) };
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

pub fn modules(pid: u32) -> Vec<String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snapshot = Handle(snapshot);
    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut names = Vec::new();
    let mut more = unsafe { Module32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        let len = entry
            .szModule
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szModule.len());
        names.push(String::from_utf16_lossy(&entry.szModule[..len]).to_ascii_lowercase());
        more = unsafe { Module32NextW(snapshot.0, &mut entry) } != 0;
    }
    names
}

pub fn is_agent(module: &str) -> bool {
    module.starts_with("kraken-agent") || module.starts_with("kraken_agent")
}

pub fn process_access() -> u32 {
    PROCESS_QUERY_INFORMATION
        | PROCESS_QUERY_LIMITED_INFORMATION
        | PROCESS_CREATE_THREAD
        | PROCESS_VM_OPERATION
        | PROCESS_VM_WRITE
}

/// explorer.exe: every game a user opens becomes its child, so a client
/// started under it is indistinguishable from a double-clicked one.
pub fn explorer_handle() -> Option<HANDLE> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let snapshot = Handle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while more {
        if process_name(&entry).eq_ignore_ascii_case("explorer.exe") {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_CREATE_PROCESS | PROCESS_DUP_HANDLE | SYNCHRONIZE,
                    0,
                    entry.th32ProcessID,
                )
            };
            if !handle.is_null() {
                return Some(handle);
            }
        }
        more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    None
}

/// The launcher keeps the current client in `%LOCALAPPDATA%\Seafight\app-*`;
/// the newest of those folders is the one the user plays.
pub fn find_client() -> Option<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)?
        .join("Seafight");
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("app-"))
        .map(|entry| entry.path().join("launchpad").join("client"))
        .filter(|dir| dir.join(GAME_EXE).is_file())
        .max_by_key(|dir| dir.metadata().and_then(|m| m.modified()).ok())
        .map(|dir| dir.join(GAME_EXE))
}

/// A running client the agent can enter, or one it is already inside.
pub enum Target {
    Fresh(Handle),
    Already,
}

/// The supervisor's choice: a Seafight client with its runtime loaded, and
/// no agent inside it yet.
pub fn open_target(pid: u32, force: bool) -> Result<Target, Failure> {
    let process = unsafe { OpenProcess(process_access(), 0, pid) };
    if process.is_null() {
        return Err(Failure::Open);
    }
    let process = Handle(process);
    if !is_game_folder(&image_path(process.0).ok_or(Failure::Open)?) {
        return Err(Failure::NotGame);
    }
    let loaded = modules(pid);
    if !loaded.iter().any(|m| m == "gameassembly.dll") {
        return Err(Failure::NotReady);
    }
    if !force && loaded.iter().any(|m| is_agent(m)) {
        return Ok(Target::Already);
    }
    Ok(Target::Fresh(process))
}

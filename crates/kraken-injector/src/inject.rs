use std::ffi::c_void;
use std::path::Path;

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, QueueUserAPC, WaitForSingleObject,
};

use crate::game::{Failure, Handle, wide};

fn write_path(process: HANDLE, dll: &Path) -> Result<*mut c_void, Failure> {
    let path = wide(&dll.to_string_lossy());
    let bytes = path.len() * 2;
    let remote = unsafe {
        VirtualAllocEx(
            process,
            std::ptr::null_mut(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        )
    };
    if remote.is_null() {
        return Err(Failure::Memory);
    }
    let mut written = 0usize;
    let ok = unsafe { WriteProcessMemory(process, remote, path.as_ptr().cast(), bytes, &mut written) };
    if ok == 0 || written != bytes {
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        return Err(Failure::Memory);
    }
    Ok(remote)
}

/// `kernel32!LoadLibraryW`, typed for the call that follows it. Kept as a
/// plain `Option` so both callers transmute it to their own signature.
fn load_library_address() -> Option<unsafe extern "system" fn() -> isize> {
    let kernel = unsafe { GetModuleHandleW(wide("kernel32.dll").as_ptr()) };
    if kernel.is_null() {
        return None;
    }
    unsafe { GetProcAddress(kernel, windows_sys::core::s!("LoadLibraryW")) }
}

/// The agent into a client that is already running: a thread of its own,
/// like any other plugin the game loads while it plays.
pub fn attach(process: HANDLE, dll: &Path) -> Result<&'static str, Failure> {
    let remote = write_path(process, dll)?;
    let Some(start) = load_library_address() else {
        return Err(Failure::Thread);
    };
    let start: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { std::mem::transmute(start) };
    let thread = unsafe {
        CreateRemoteThread(
            process,
            std::ptr::null_mut(),
            0,
            Some(start),
            remote,
            0,
            std::ptr::null_mut(),
        )
    };
    if thread.is_null() {
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        return Err(Failure::Thread);
    }
    let thread = Handle(thread);
    if unsafe { WaitForSingleObject(thread.0, 15_000) } != WAIT_OBJECT_0 {
        return Err(Failure::Thread);
    }
    let mut code = 0u32;
    unsafe { GetExitCodeThread(thread.0, &mut code) };
    if code == 0 {
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        Err(Failure::Load)
    } else {
        Ok("injected")
    }
}

/// The agent into a client that has not started yet: queued on its main
/// thread, it runs before the game's first instruction. The path buffer is
/// left behind on purpose — freeing it before the APC has run would unmap
/// the string it is about to read.
pub fn attach_suspended(process: HANDLE, thread: HANDLE, dll: &Path) -> Result<(), Failure> {
    let remote = write_path(process, dll)?;
    let Some(start) = load_library_address() else {
        return Err(Failure::Thread);
    };
    let start: unsafe extern "system" fn(usize) = unsafe { std::mem::transmute(start) };
    if unsafe { QueueUserAPC(Some(start), thread, remote as usize) } == 0 {
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        return Err(Failure::Thread);
    }
    Ok(())
}

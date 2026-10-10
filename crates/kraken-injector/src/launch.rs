use std::ffi::c_void;
use std::path::Path;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, PROCESS_INFORMATION,
    ResumeThread, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
};

use crate::game::{Failure, wide};

/// The created game process: both handles belong to the launcher only, the
/// process itself keeps running after they close.
pub struct Child {
    pub process: HANDLE,
    pub thread: HANDLE,
    pub pid: u32,
}

/// The client starts suspended, and — when we have a handle for it — under
/// explorer.exe as its parent, so a watcher sees the same process tree a
/// double-click produces. Nothing in its image runs until the caller
/// resumes the main thread.
pub fn start(exe: &Path, parent: Option<HANDLE>) -> Result<Child, Failure> {
    let application = wide(&exe.to_string_lossy());
    let working = wide(
        &exe.parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let mut child = PROCESS_INFORMATION {
        hProcess: std::ptr::null_mut(),
        hThread: std::ptr::null_mut(),
        dwProcessId: 0,
        dwThreadId: 0,
    };
    match parent {
        Some(parent) => match start_with_parent(&application, &working, parent, &mut child) {
            Ok(()) => Ok(()),
            // Windows 11 refuses parent spoofing against system processes:
            // start it under us instead of letting the game fail to open.
            Err(Failure::Parent) => start_alone(&application, &working, &mut child),
            Err(failure) => Err(failure),
        },
        None => start_alone(&application, &working, &mut child),
    }?;
    Ok(Child {
        process: child.hProcess,
        thread: child.hThread,
        pid: child.dwProcessId,
    })
}

fn start_with_parent(
    application: &[u16],
    working: &[u16],
    parent: HANDLE,
    child: &mut PROCESS_INFORMATION,
) -> Result<(), Failure> {
    let mut attribute_size = 0usize;
    if unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attribute_size) } == 0 {
        return Err(Failure::Parent);
    }
    let layout = std::alloc::Layout::from_size_align(attribute_size, std::mem::align_of::<usize>())
        .map_err(|_| Failure::Parent)?;
    let buffer = unsafe { std::alloc::alloc(layout) };
    if buffer.is_null() {
        return Err(Failure::Parent);
    }
    let attribute_list = buffer.cast::<c_void>();
    let result = create(
        application,
        working,
        parent,
        attribute_list,
        &mut attribute_size,
        child,
    );
    unsafe { DeleteProcThreadAttributeList(attribute_list) };
    unsafe { std::alloc::dealloc(buffer, layout) };
    result
}

fn create(
    application: &[u16],
    working: &[u16],
    parent: HANDLE,
    attribute_list: *mut c_void,
    attribute_size: &mut usize,
    child: &mut PROCESS_INFORMATION,
) -> Result<(), Failure> {
    if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, attribute_size) } == 0 {
        return Err(Failure::Parent);
    }
    if unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
            std::ptr::addr_of!(parent).cast(),
            size_of::<HANDLE>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    } == 0
    {
        return Err(Failure::Parent);
    }
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attribute_list;
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
            std::ptr::null(),
            working.as_ptr(),
            std::ptr::addr_of!(startup.StartupInfo),
            child,
        )
    };
    (ok != 0).then_some(()).ok_or(Failure::Launch)
}

fn start_alone(application: &[u16], working: &[u16], child: &mut PROCESS_INFORMATION) -> Result<(), Failure> {
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null(),
            working.as_ptr(),
            std::ptr::addr_of!(startup),
            child,
        )
    };
    (ok != 0).then_some(()).ok_or(Failure::Launch)
}

/// The game's first moment: the agent's `LoadLibraryW` is already queued,
/// this lets the game begin with the agent inside.
pub fn resume(thread: HANDLE) -> Result<(), Failure> {
    if unsafe { ResumeThread(thread) } == u32::MAX {
        return Err(Failure::Load);
    }
    Ok(())
}

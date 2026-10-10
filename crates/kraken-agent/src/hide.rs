//! Hiding the agent from the game's module list: the loader keeps every
//! loaded image in three doubly-linked lists in the PEB. Any scan that
//! looks for a module next to the game's own DLLs reads them, so the agent
//! takes its entry out of all three right after it has loaded.
//!
//! Only the lists are touched. Windows also keeps hashed lookups for name
//! resolution; the shapes of those tables change between releases, and a
//! wrong unlink there is worse than the entry left in it.

#[cfg(target_arch = "x86")]
fn peb() -> usize {
    let peb: u32;
    unsafe { core::arch::asm!("mov {value}, dword ptr fs:[0x30]", value = out(reg) peb) };
    peb as usize
}

#[cfg(target_arch = "x86_64")]
fn peb() -> usize {
    let peb: u64;
    unsafe { core::arch::asm!("mov {value}, qword ptr gs:[0x60]", value = out(reg) peb) };
    peb as usize
}

#[repr(C)]
struct ListEntry {
    flink: usize,
    blink: usize,
}

/// The loader's lists, inside the PEB and its `PEB_LDR_DATA`. Every one of
/// these differs between x86 and x64; a wrong value here reads a garbage
/// list head and the walk dereferences into nothing (this was the ~8 s
/// crash: `PEB_LDR` was 0x0a on both, instead of 0x0C on x86 / 0x18 on x64).
const PEB_LDR: usize = if cfg!(target_arch = "x86") { 0x0c } else { 0x18 };
const LDR_IN_LOAD_ORDER: usize = if cfg!(target_arch = "x86") { 0x0c } else { 0x10 };
const LDR_IN_MEMORY_ORDER: usize = if cfg!(target_arch = "x86") { 0x14 } else { 0x20 };
const LDR_IN_INIT_ORDER: usize = if cfg!(target_arch = "x86") { 0x1c } else { 0x30 };
/// `LDR_DATA_TABLE_ENTRY` offsets — stable for Windows 7..11 on both
/// architectures.
const ENTRY_DLL_BASE: usize = if cfg!(target_arch = "x86") { 0x18 } else { 0x30 };
const ENTRY_IN_LOAD_ORDER: usize = 0x00;
const ENTRY_IN_MEMORY_ORDER: usize = if cfg!(target_arch = "x86") { 0x08 } else { 0x10 };
const ENTRY_IN_INIT_ORDER: usize = if cfg!(target_arch = "x86") { 0x10 } else { 0x20 };

fn read(address: usize) -> usize {
    unsafe { *(address as *const usize) }
}

/// Takes our own entry out of one of the loader's three lists: the module
/// stops being visible there, its code stays mapped. Returns whether the
/// entry was found, which only happens when the offsets are right. The walk
/// is bounded so a corrupt or unexpected list can never spin.
fn unlink(list: usize, link_offset: usize, module: usize) -> bool {
    let mut flink = read(list);
    let mut steps = 0;
    while flink != list && steps < 4096 {
        let entry = flink - link_offset;
        if read(entry + ENTRY_DLL_BASE) == module {
            let node = unsafe { &mut *((entry + link_offset) as *mut ListEntry) };
            let previous = unsafe { &mut *(node.blink as *mut ListEntry) };
            let next = unsafe { &mut *(node.flink as *mut ListEntry) };
            previous.flink = node.flink;
            next.blink = node.blink;
            // Self-referencing: anything walking on stops here.
            node.flink = entry + link_offset;
            node.blink = entry + link_offset;
            return true;
        }
        flink = read(entry + link_offset);
        steps += 1;
    }
    false
}

/// `module` is the agent's own image base, as `DllMain` was given it.
pub fn hide(module: usize) {
    let ldr = read(peb() + PEB_LDR);
    tracing::info!(
        module = format!("{module:#x}"),
        ldr = format!("{ldr:#x}"),
        "peb unlink"
    );
    if ldr == 0 || module == 0 {
        return;
    }
    for (name, list, link) in [
        ("load", LDR_IN_LOAD_ORDER, ENTRY_IN_LOAD_ORDER),
        ("memory", LDR_IN_MEMORY_ORDER, ENTRY_IN_MEMORY_ORDER),
        ("init", LDR_IN_INIT_ORDER, ENTRY_IN_INIT_ORDER),
    ] {
        let found = unlink(ldr + list, link, module);
        tracing::info!(list = name, found, "unlinked from loader list");
    }
}

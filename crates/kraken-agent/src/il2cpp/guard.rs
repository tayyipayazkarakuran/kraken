use std::cell::RefCell;

use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY,
    PAGE_GUARD, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQuery,
};

const READABLE: u32 = PAGE_READONLY
    | PAGE_READWRITE
    | PAGE_WRITECOPY
    | PAGE_EXECUTE_READ
    | PAGE_EXECUTE_READWRITE
    | PAGE_EXECUTE_WRITECOPY;
const LOWEST: usize = 0x1_0000;
const MAX_REGIONS: usize = 64;

thread_local! {
    static REGIONS: RefCell<Vec<(usize, usize)>> = const { RefCell::new(Vec::new()) };
}

fn query(addr: usize) -> Option<(usize, usize)> {
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let size = size_of::<MEMORY_BASIC_INFORMATION>();
    if unsafe { VirtualQuery(addr as *const _, &mut info, size) } != size {
        return None;
    }
    let protect = info.Protect;
    let ok =
        info.State == MEM_COMMIT && protect & READABLE != 0 && protect & (PAGE_GUARD | PAGE_NOACCESS) == 0;
    let start = info.BaseAddress as usize;
    ok.then_some((start, start.saturating_add(info.RegionSize)))
}

pub fn readable(addr: *const u8, len: usize) -> bool {
    let start = addr as usize;
    let Some(end) = start.checked_add(len.max(1)) else {
        return false;
    };
    if start < LOWEST {
        return false;
    }
    REGIONS.with_borrow_mut(|regions| {
        let mut at = start;
        while at < end {
            let region = match regions.iter().find(|(s, e)| *s <= at && at < *e) {
                Some(region) => *region,
                None => {
                    let Some(region) = query(at) else { return false };
                    if regions.len() >= MAX_REGIONS {
                        regions.remove(0);
                    }
                    regions.push(region);
                    region
                }
            };
            at = region.1;
        }
        true
    })
}

pub fn pointer(ptr: *const u8) -> bool {
    (ptr as usize).is_multiple_of(align_of::<usize>()) && readable(ptr, 2 * size_of::<usize>())
}

pub fn reset() {
    REGIONS.with_borrow_mut(Vec::clear);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_live_memory_and_rejects_garbage() {
        let value = Box::new([0usize; 4]);
        assert!(pointer(value.as_ptr().cast()));
        assert!(!pointer(std::ptr::null()));
        assert!(!pointer(usize::MAX as *const u8));
        assert!(!pointer(0xafe5 as *const u8));
        assert!(!pointer(unsafe { value.as_ptr().cast::<u8>().add(1) }));
        reset();
        assert!(readable(value.as_ptr().cast(), 32));
    }
}

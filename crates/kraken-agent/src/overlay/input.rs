//! Window-procedure subclass of the game window. Mouse and keyboard go to
//! the app; while the pointer is over the interface (or a text field has
//! focus) the game does not see them.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::time::Instant;

use kraken_proto::UiInput;
use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_F9, VK_MENU, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, GetWindowLongPtrW, SetWindowLongPtrW, WM_CHAR, WM_INPUT,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WNDPROC,
};

/// Not exported by the `windows` crate.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Pointer moves are coalesced to about this rate.
const MOVE_EVERY_US: u128 = 8_000;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
static LAST_X: AtomicI32 = AtomicI32::new(-1);
static LAST_Y: AtomicI32 = AtomicI32::new(-1);
/// A button went down over the interface; its release belongs to it too.
static CAPTURING: AtomicBool = AtomicBool::new(false);
static LAST_MOVE: Mutex<Option<Instant>> = Mutex::new(None);
static HIGH_SURROGATE: Mutex<Option<u16>> = Mutex::new(None);

pub fn attach(window: HWND) {
    if WINDOW.load(Ordering::Acquire) != 0 {
        return;
    }
    let previous = unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, wndproc as *const () as usize as _) };
    if previous == 0 {
        tracing::warn!("overlay could not subclass the game window");
        return;
    }
    ORIGINAL.store(previous as isize, Ordering::Release);
    WINDOW.store(window.0 as isize, Ordering::Release);
    tracing::info!("overlay input attached");
}

pub fn detach() {
    let window = WINDOW.swap(0, Ordering::AcqRel);
    if window == 0 {
        return;
    }
    let window = HWND(window as *mut _);
    let current = unsafe { GetWindowLongPtrW(window, GWLP_WNDPROC) };
    // Only put the old procedure back if nobody subclassed after us; else
    // ours stays in the chain and passes everything through.
    if current as usize == wndproc as *const () as usize {
        unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, ORIGINAL.load(Ordering::Acquire) as _) };
    }
}

fn xy(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    (i32::from(raw as u16 as i16), i32::from((raw >> 16) as u16 as i16))
}

fn over(x: i32, y: i32) -> bool {
    super::SCENE
        .lock()
        .capture
        .iter()
        .any(|r| r.contains(x as f32, y as f32))
}

fn modifiers() -> u8 {
    let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
    u8::from(down(VK_SHIFT.0)) | (u8::from(down(VK_CONTROL.0)) << 1) | (u8::from(down(VK_MENU.0)) << 2)
}

enum Verdict {
    Pass,
    /// Handled here; the game must not see it.
    Swallow,
    /// Swallowed, but `DefWindowProc` must still run (raw input clean-up).
    Default,
}

fn handle(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Verdict {
    let pointer = || (LAST_X.load(Ordering::Relaxed), LAST_Y.load(Ordering::Relaxed));
    match msg {
        WM_MOUSEMOVE => {
            let (x, y) = xy(lparam);
            LAST_X.store(x, Ordering::Relaxed);
            LAST_Y.store(y, Ordering::Relaxed);
            let mut last = LAST_MOVE.lock();
            if last.is_none_or(|t| t.elapsed().as_micros() >= MOVE_EVERY_US) {
                *last = Some(Instant::now());
                super::send(UiInput::PointerMoved {
                    x: x as f32,
                    y: y as f32,
                });
            }
            if CAPTURING.load(Ordering::Relaxed) || over(x, y) {
                Verdict::Swallow
            } else {
                Verdict::Pass
            }
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
            let (x, y) = xy(lparam);
            let (button, pressed) = match msg {
                WM_LBUTTONDOWN => (0, true),
                WM_LBUTTONUP => (0, false),
                WM_RBUTTONDOWN => (1, true),
                WM_RBUTTONUP => (1, false),
                WM_MBUTTONDOWN => (2, true),
                _ => (2, false),
            };
            super::send(UiInput::PointerMoved {
                x: x as f32,
                y: y as f32,
            });
            super::send(UiInput::Button {
                x: x as f32,
                y: y as f32,
                button,
                pressed,
            });
            let ours = if pressed {
                let hit = over(x, y);
                CAPTURING.store(hit, Ordering::Relaxed);
                hit
            } else {
                CAPTURING.swap(false, Ordering::Relaxed)
            };
            if ours { Verdict::Swallow } else { Verdict::Pass }
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let notches = f32::from((wparam.0 >> 16) as u16 as i16) / 120.0;
            let (x, y) = pointer();
            if !over(x, y) {
                return Verdict::Pass;
            }
            let (dx, dy) = if msg == WM_MOUSEWHEEL {
                (0.0, notches)
            } else {
                (notches, 0.0)
            };
            super::send(UiInput::Wheel { x: dx, y: dy });
            Verdict::Swallow
        }
        WM_MOUSELEAVE => {
            super::send(UiInput::PointerLeft);
            Verdict::Pass
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let vk = wparam.0 as u16;
            let pressed = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
            if vk == VK_F9.0 {
                // Auto-repeat would flicker the panel.
                let repeat = lparam.0 as u32 & (1 << 30) != 0;
                if pressed && !repeat {
                    super::send(UiInput::TogglePanel);
                }
                return Verdict::Swallow;
            }
            if !super::SCENE.lock().keyboard {
                return Verdict::Pass;
            }
            super::send(UiInput::Key {
                vk,
                pressed,
                modifiers: modifiers(),
            });
            Verdict::Swallow
        }
        WM_CHAR => {
            if !super::SCENE.lock().keyboard {
                return Verdict::Pass;
            }
            let unit = wparam.0 as u16;
            let mut high = HIGH_SURROGATE.lock();
            let text = if (0xD800..0xDC00).contains(&unit) {
                *high = Some(unit);
                None
            } else if let Some(first) = high.take() {
                String::from_utf16(&[first, unit]).ok()
            } else {
                String::from_utf16(&[unit]).ok()
            };
            if let Some(text) = text.filter(|t| t.chars().all(|c| !c.is_control())) {
                super::send(UiInput::Text(text));
            }
            Verdict::Swallow
        }
        WM_SETFOCUS => {
            super::send(UiInput::Focus(true));
            Verdict::Pass
        }
        WM_KILLFOCUS => {
            CAPTURING.store(false, Ordering::Relaxed);
            super::send(UiInput::Focus(false));
            Verdict::Pass
        }
        WM_INPUT => {
            let (x, y) = pointer();
            if CAPTURING.load(Ordering::Relaxed) || over(x, y) {
                Verdict::Default
            } else {
                Verdict::Pass
            }
        }
        _ => Verdict::Pass,
    }
}

unsafe extern "system" fn wndproc(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if super::active() {
        let verdict = catch_unwind(AssertUnwindSafe(|| handle(msg, wparam, lparam))).unwrap_or(Verdict::Pass);
        match verdict {
            Verdict::Swallow => return LRESULT(0),
            Verdict::Default => return unsafe { DefWindowProcW(window, msg, wparam, lparam) },
            Verdict::Pass => {}
        }
    }
    let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    unsafe { CallWindowProcW(original, window, msg, wparam, lparam) }
}

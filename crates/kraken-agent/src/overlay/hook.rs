//! `IDXGISwapChain::Present` and `ResizeBuffers` detours. The vtable comes
//! from a throwaway device and swap chain; every swap chain shares it.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use parking_lot::Mutex;
use retour::RawDetour;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11CreateDeviceAndSwapChain, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter,
    IDXGISwapChain,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};
use windows::core::{HRESULT, Interface, w};

type PresentFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(*mut c_void, u32, u32, u32, DXGI_FORMAT, u32) -> HRESULT;

const PRESENT_SLOT: usize = 8;
const RESIZE_SLOT: usize = 13;

static PRESENT: OnceLock<PresentFn> = OnceLock::new();
static RESIZE: OnceLock<ResizeFn> = OnceLock::new();
static DETOURS: Mutex<Vec<Detour>> = Mutex::new(Vec::new());

struct Detour(RawDetour);

unsafe impl Send for Detour {}

unsafe extern "system" fn present(this: *mut c_void, sync: u32, flags: u32) -> HRESULT {
    if super::active() {
        let _ = catch_unwind(AssertUnwindSafe(|| super::render::frame(this)));
    }
    match PRESENT.get() {
        Some(original) => unsafe { original(this, sync, flags) },
        None => HRESULT(0),
    }
}

unsafe extern "system" fn resize(
    this: *mut c_void,
    count: u32,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    flags: u32,
) -> HRESULT {
    // The back buffer view must go before the buffers can be resized.
    let _ = catch_unwind(super::render::before_resize);
    match RESIZE.get() {
        Some(original) => unsafe { original(this, count, width, height, format, flags) },
        None => HRESULT(0),
    }
}

fn vtable() -> Result<(*const (), *const ()), String> {
    unsafe {
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            // A short-lived, invisible dummy window used only to grab the
            // swap-chain vtable; keep its title blank so it carries no name.
            w!(""),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            8,
            8,
            None,
            None,
            None,
            None,
        )
        .map_err(|e| format!("dummy window: {e}"))?;
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: 8,
                Height: 8,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            OutputWindow: window,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };
        let mut swap: Option<IDXGISwapChain> = None;
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let created = D3D11CreateDeviceAndSwapChain(
            None::<&IDXGIAdapter>,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&desc),
            Some(&mut swap),
            Some(&mut device),
            None,
            Some(&mut context),
        );
        let result = created
            .map_err(|e| format!("dummy swap chain: {e}"))
            .and_then(|()| swap.ok_or_else(|| "no swap chain".to_owned()))
            .map(|swap| {
                let table = *(swap.as_raw() as *const *const *const ());
                (*table.add(PRESENT_SLOT), *table.add(RESIZE_SLOT))
            });
        drop((context, device));
        let _ = DestroyWindow(window);
        result
    }
}

fn detour(target: *const (), replacement: *const ()) -> Result<*const (), String> {
    let raw = unsafe { RawDetour::new(target, replacement) }.map_err(|e| e.to_string())?;
    let trampoline = raw.trampoline() as *const ();
    unsafe { raw.enable() }.map_err(|e| e.to_string())?;
    DETOURS.lock().push(Detour(raw));
    Ok(trampoline)
}

pub fn install() -> Result<(), String> {
    let (present_at, resize_at) = vtable()?;
    let original = detour(resize_at, resize as *const ())?;
    let _ = RESIZE.set(unsafe { std::mem::transmute::<*const (), ResizeFn>(original) });
    let original = detour(present_at, present as *const ())?;
    let _ = PRESENT.set(unsafe { std::mem::transmute::<*const (), PresentFn>(original) });
    Ok(())
}

pub fn uninstall() {
    for detour in DETOURS.lock().drain(..) {
        if let Err(error) = unsafe { detour.0.disable() } {
            tracing::warn!(%error, "overlay detour disable failed");
        }
        // The trampolines may still be running on the render thread.
        std::mem::forget(detour);
    }
}

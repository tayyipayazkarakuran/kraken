#![allow(
    clippy::missing_safety_doc,
    clippy::missing_transmute_annotations,
    clippy::not_unsafe_ptr_arg_deref,
    reason = "thin FFI layer over pointers owned by the IL2CPP runtime"
)]

pub mod engine;
#[cfg(windows)]
pub mod game;
#[cfg(windows)]
pub mod il2cpp;

#[cfg(windows)]
mod boot;
#[cfg(windows)]
mod dump;
#[cfg(windows)]
mod hide;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod pump;

#[cfg(windows)]
pub use boot::unload;

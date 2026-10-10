mod config;
mod message;
pub mod names;
mod wire;

pub use config::*;
pub use message::*;
pub use wire::*;

pub const PROTO_VERSION: u32 = 8;

/// The two IPC pipe names, derived from the pid. Deliberately unbranded and
/// per-process (not a static string like some bots' fixed pipe), so a scan
/// for a known pipe name finds nothing to match on.
pub fn pipe_names(pid: u32) -> (String, String) {
    let tag = pid.wrapping_mul(0x9E37_79B1) ^ 0x5F35_6495;
    (
        format!(r"\\.\pipe\ipc-{tag:08x}-a"),
        format!(r"\\.\pipe\ipc-{tag:08x}-b"),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn toward(self, other: Self, step: f32) -> Self {
        let d = self.distance(other);
        if d <= step || d == 0.0 {
            return other;
        }
        let k = step / d;
        Self::new(self.x + (other.x - self.x) * k, self.y + (other.y - self.y) * k)
    }

    pub fn rounded(self) -> Self {
        Self::new(self.x.round(), self.y.round())
    }
}

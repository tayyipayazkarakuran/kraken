//! The in-game interface: a window-less egui context per game. Input comes
//! from the agent's window hook; the tessellated output goes back to the
//! agent, which draws it inside the game's own frame.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, TextureId, ViewportId, epaint, pos2,
    vec2,
};
use kraken_proto::{UiFrame, UiInput, UiMesh, UiRect, UiTexture, UiVertex};

use crate::theme;
use crate::ui::{self, State};

/// The largest panel, in points.
const PANEL_MAX: egui::Vec2 = egui::vec2(1100.0, 800.0);
const PANEL_MARGIN: f32 = 12.0;
/// Frames are re-sent at least this often (animations, live values).
const REFRESH: Duration = Duration::from_millis(100);

pub struct Remote {
    ctx: egui::Context,
    started: Instant,
    size: Option<(u32, u32)>,
    events: Vec<Event>,
    modifiers: Modifiers,
    focused: bool,
    pub panel_open: bool,
    last_hash: u64,
    last_sent: Option<Instant>,
    /// A full texture re-upload is needed (new context or reconnect).
    pending_redraw: bool,
}

impl Remote {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        Self {
            ctx,
            started: Instant::now(),
            size: None,
            events: Vec::new(),
            modifiers: Modifiers::default(),
            focused: true,
            panel_open: false,
            last_hash: 0,
            last_sent: None,
            pending_redraw: true,
        }
    }

    /// The agent restarted: its textures are gone, start over.
    pub fn reset(&mut self) {
        let open = self.panel_open;
        *self = Self::new();
        self.panel_open = open;
    }

    pub fn has_size(&self) -> bool {
        self.size.is_some_and(|(w, h)| w > 0 && h > 0)
    }

    fn scale(scale: f32) -> f32 {
        scale.clamp(0.6, 1.5)
    }

    /// Returns `true` when the input should trigger a frame right away.
    pub fn input(&mut self, input: UiInput, scale: f32) -> bool {
        let scale = Self::scale(scale);
        let point = |x: f32, y: f32| pos2(x / scale, y / scale);
        match input {
            UiInput::Resized { width, height } => {
                let changed = self.size != Some((width, height));
                self.size = Some((width, height));
                return changed;
            }
            UiInput::PointerMoved { x, y } => self.events.push(Event::PointerMoved(point(x, y))),
            UiInput::PointerLeft => self.events.push(Event::PointerGone),
            UiInput::Button {
                x,
                y,
                button,
                pressed,
            } => {
                let button = match button {
                    1 => PointerButton::Secondary,
                    2 => PointerButton::Middle,
                    _ => PointerButton::Primary,
                };
                self.events.push(Event::PointerButton {
                    pos: point(x, y),
                    button,
                    pressed,
                    modifiers: self.modifiers,
                });
            }
            UiInput::Wheel { x, y } => self.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(x, y),
                phase: egui::TouchPhase::Move,
                modifiers: self.modifiers,
            }),
            UiInput::Key {
                vk,
                pressed,
                modifiers,
            } => {
                self.modifiers = Modifiers {
                    alt: modifiers & 4 != 0,
                    ctrl: modifiers & 2 != 0,
                    shift: modifiers & 1 != 0,
                    mac_cmd: false,
                    command: modifiers & 2 != 0,
                };
                if let Some(key) = key_of(vk) {
                    self.events.push(Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: self.modifiers,
                    });
                    if pressed && self.modifiers.ctrl {
                        match key {
                            Key::C => self.events.push(Event::Copy),
                            Key::X => self.events.push(Event::Cut),
                            _ => {}
                        }
                    }
                }
            }
            UiInput::Text(text) => self.events.push(Event::Text(text)),
            UiInput::TogglePanel => self.panel_open = !self.panel_open,
            UiInput::Focus(focused) => {
                self.focused = focused;
                self.events.push(Event::WindowFocused(focused));
            }
        }
        true
    }

    /// Lays out the interface for `state`; `None` when nothing changed and
    /// no refresh is due.
    pub fn frame(&mut self, state: &mut State, force: bool) -> Option<UiFrame> {
        let (width, height) = self.size.filter(|(w, h)| *w > 0 && *h > 0)?;
        let scale = Self::scale(state.profile.overlay.scale);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(width as f32 / scale, height as f32 / scale));
        let mut raw = RawInput {
            screen_rect: Some(screen),
            time: Some(self.started.elapsed().as_secs_f64()),
            focused: self.focused,
            events: std::mem::take(&mut self.events),
            ..RawInput::default()
        };
        raw.viewports
            .entry(ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(scale);
        let had_events = !raw.events.is_empty();
        let show = state.profile.overlay.enabled;
        let mut panel_open = self.panel_open;
        let output = self.ctx.run_ui(raw, |root| {
            let ctx = root.ctx().clone();
            if !show {
                return;
            }
            ui::hud::markers(&ctx, state, scale);
            if state.profile.overlay.hud && ui::hud::card(&ctx, state, panel_open) {
                panel_open = !panel_open;
            }
            if panel_open {
                panel(&ctx, state, screen);
            }
        });
        self.panel_open = panel_open;
        let textures_changed =
            !output.textures_delta.set.is_empty() || !output.textures_delta.free.is_empty();
        let ppp = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, ppp);
        let meshes = meshes(&primitives, ppp);
        let capture = capture(&self.ctx, ppp);
        let keyboard = self.ctx.egui_wants_keyboard_input();
        let hash = digest(&meshes, &capture, keyboard);
        let due = self.last_sent.is_none_or(|t| t.elapsed() >= REFRESH);
        let changed = hash != self.last_hash;
        if !(force || had_events || textures_changed || self.pending_redraw || (changed && due)) {
            return None;
        }
        self.pending_redraw = false;
        self.last_hash = hash;
        self.last_sent = Some(Instant::now());
        Some(UiFrame {
            width,
            height,
            textures: output
                .textures_delta
                .set
                .iter()
                .flat_map(|(id, deltas)| deltas.iter().filter_map(|delta| texture(*id, delta)))
                .collect(),
            freed: output
                .textures_delta
                .free
                .iter()
                .map(|id| texture_key(*id))
                .collect(),
            meshes,
            capture,
            keyboard,
        })
    }
}

/// The control panel: the full Kraken interface in the top-left corner.
fn panel(ctx: &egui::Context, state: &mut State, screen: Rect) {
    let size = vec2(
        (screen.width() - 2.0 * PANEL_MARGIN).min(PANEL_MAX.x),
        (screen.height() - 2.0 * PANEL_MARGIN).min(PANEL_MAX.y),
    );
    if size.x < 200.0 || size.y < 150.0 {
        return;
    }
    egui::Area::new(egui::Id::new("kraken-panel"))
        .fixed_pos(pos2(PANEL_MARGIN, PANEL_MARGIN))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            let rect = Rect::from_min_size(ui.min_rect().min, size);
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(theme::RADIUS),
                theme::DEEP,
                egui::Stroke::new(1.0, theme::LINE),
                egui::StrokeKind::Outside,
            );
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.set_clip_rect(rect);
            ui::shell(ui, state);
        });
}

fn texture_key(id: TextureId) -> u64 {
    match id {
        TextureId::Managed(id) => id,
        TextureId::User(id) => id | (1 << 63),
    }
}

fn texture(id: TextureId, delta: &epaint::ImageDelta) -> Option<UiTexture> {
    let epaint::ImageData::Color(image) = &delta.image;
    let [width, height] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    Some(UiTexture {
        id: texture_key(id),
        origin: delta.pos.map(|[x, y]| [x as u32, y as u32]),
        width: width as u32,
        height: height as u32,
        rgba,
        linear: delta.options.magnification == egui::TextureFilter::Linear,
    })
}

fn meshes(primitives: &[epaint::ClippedPrimitive], ppp: f32) -> Vec<UiMesh> {
    primitives
        .iter()
        .filter_map(|clipped| {
            let epaint::Primitive::Mesh(mesh) = &clipped.primitive else {
                return None;
            };
            if mesh.indices.is_empty() {
                return None;
            }
            let clip = clipped.clip_rect;
            Some(UiMesh {
                texture: texture_key(mesh.texture_id),
                clip: UiRect {
                    min: [clip.min.x * ppp, clip.min.y * ppp],
                    max: [clip.max.x * ppp, clip.max.y * ppp],
                },
                vertices: mesh
                    .vertices
                    .iter()
                    .map(|v| UiVertex {
                        pos: [v.pos.x * ppp, v.pos.y * ppp],
                        uv: [v.uv.x, v.uv.y],
                        color: v.color.to_array(),
                    })
                    .collect(),
                indices: mesh.indices.clone(),
            })
        })
        .collect()
}

/// Every visible area except the background markers takes the mouse.
fn capture(ctx: &egui::Context, ppp: f32) -> Vec<UiRect> {
    ctx.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .into_iter()
            .filter(|layer| layer.order != egui::Order::Background)
            .filter_map(|layer| memory.area_rect(layer.id))
            .map(|rect| UiRect {
                min: [rect.min.x * ppp, rect.min.y * ppp],
                max: [rect.max.x * ppp, rect.max.y * ppp],
            })
            .collect()
    })
}

fn digest(meshes: &[UiMesh], capture: &[UiRect], keyboard: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    keyboard.hash(&mut hasher);
    for rect in capture {
        for v in rect.min.iter().chain(&rect.max) {
            v.to_bits().hash(&mut hasher);
        }
    }
    for mesh in meshes {
        mesh.texture.hash(&mut hasher);
        mesh.indices.hash(&mut hasher);
        for v in &mesh.vertices {
            v.pos[0].to_bits().hash(&mut hasher);
            v.pos[1].to_bits().hash(&mut hasher);
            v.color.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Windows virtual-key codes that matter for text fields and lists.
fn key_of(vk: u16) -> Option<Key> {
    Some(match vk {
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::ArrowLeft,
        0x26 => Key::ArrowUp,
        0x27 => Key::ArrowRight,
        0x28 => Key::ArrowDown,
        0x2D => Key::Insert,
        0x2E => Key::Delete,
        0x30..=0x39 => Key::from_name(&char::from(vk as u8).to_string())?,
        0x41..=0x5A => Key::from_name(&char::from(vk as u8).to_string())?,
        _ => return None,
    })
}

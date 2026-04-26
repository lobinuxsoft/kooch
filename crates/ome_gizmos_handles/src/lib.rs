//! Interactive editor handles — translate / rotate / scale gizmos.
//!
//! Stateful counterpart to [`ome_gizmos`]. While `ome_gizmos` is
//! immediate-mode (rebuild per frame from selection), handles carry
//! drag state across frames: the user clicks an axis, drags, releases.
//!
//! Architecture:
//!
//! - [`Handle`] trait: each interactive handle implements `draw` (visual),
//!   `pick` (ray-vs-handle hit test), and `drag` (per-frame world delta
//!   from input rays).
//! - [`HandleSet`] coordinator: owns a list of handles, runs the state
//!   machine `Idle → Hover → Drag`, returns the accumulated translation
//!   delta to apply to the selected entity each frame.
//! - [`TranslateHandle`]: built-in handle for one axis. The default
//!   [`HandleSet`] contains three (X, Y, Z).
//!
//! v1 scope: translate only, world-space, single-entity. Rotate and
//! scale are separate phases of #278; multi-entity drag, Local-space
//! mode, and undo integration are polish follow-ups.

mod plane;
mod rotate;
mod scale;
mod translate;

pub use plane::PlaneHandle;
pub use rotate::RotateHandle;
pub use scale::ScaleHandle;
pub use translate::TranslateHandle;

use glam::{Mat3, Quat, Vec3};
use ome_gizmos::Gizmos;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// World-space ray, used for picking and drag math.
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    /// Normalized direction.
    pub direction: Vec3,
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize_or_zero(),
        }
    }

    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}

/// Visual state of a single handle, passed to [`Handle::draw`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleState {
    Idle,
    Hover,
    Dragging,
}

/// Cardinal axis used by built-in translate / rotate / scale handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn vec(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }

    pub fn base_color(self) -> Vec3 {
        match self {
            Self::X => Vec3::new(1.0, 0.25, 0.25),
            Self::Y => Vec3::new(0.25, 1.0, 0.25),
            Self::Z => Vec3::new(0.35, 0.45, 1.0),
        }
    }
}

/// Per-frame ray delta passed to [`Handle::drag`] while a drag is
/// active.
///
/// - `start_ray` is the ray captured at the moment the user clicked
///   the handle. Stays fixed for the whole drag and lets snap math
///   anchor totals to a stable reference (so toggling a modifier
///   mid-drag works without rewinding more than one snap step).
/// - `last_ray` / `current_ray` are the previous and current frame's
///   rays — what the unsnapped per-frame delta is computed from.
/// - `modifiers` reflects the current keyboard state so handles can
///   gate snap math on Ctrl / Shift / Alt.
#[derive(Debug, Clone, Copy)]
pub struct DragInfo {
    pub start_ray: Ray,
    pub last_ray: Ray,
    pub current_ray: Ray,
    pub modifiers: DragModifiers,
    pub snap: SnapSettings,
}

/// Keyboard modifier state at this frame. Owned by the editor and
/// threaded through to handles so each handle can pick the appropriate
/// snap modifier (Ctrl for translate / rotate snap).
#[derive(Debug, Clone, Copy, Default)]
pub struct DragModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// User-tunable snap step sizes. Threaded through `DragInfo` to each
/// handle so the snap math reads the live values from the toolbar
/// instead of hard-coding compile-time constants.
#[derive(Debug, Clone, Copy)]
pub struct SnapSettings {
    /// Translate / plane snap step in world units. Applied per axis.
    pub translate: f32,
    /// Rotate snap step in degrees.
    pub rotate_deg: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            translate: 0.5,
            rotate_deg: 15.0,
        }
    }
}

/// Output of [`Handle::drag`] — what kind of edit the handle wants to
/// apply to the selected entity this frame. The editor matches on the
/// variant and updates `Transform` accordingly.
#[derive(Debug, Clone, Copy)]
pub enum TransformDelta {
    Translation(Vec3),
    Rotation(Quat),
    Scale(Vec3),
}

impl TransformDelta {
    /// Identity delta (no change). Used by drag returns when picking
    /// fails or modes don't match.
    pub fn none() -> Self {
        Self::Translation(Vec3::ZERO)
    }

    pub fn is_noop(self) -> bool {
        match self {
            Self::Translation(v) => v == Vec3::ZERO,
            Self::Rotation(q) => q.abs_diff_eq(Quat::IDENTITY, 1e-6),
            Self::Scale(v) => v == Vec3::ZERO,
        }
    }
}

/// Edit mode for a [`HandleSet`] — selects which subset of handles
/// renders / picks / drags this frame. Mirrors Maya / Unity / Unreal
/// conventions (W = translate, E = rotate, R = scale).
///
/// Note: the W/E/R hotkey labels are conventions enforced by the
/// editor's input layer; the enum itself is shortcut-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleMode {
    Translate,
    Rotate,
    Scale,
}

impl Default for HandleMode {
    fn default() -> Self {
        Self::Translate
    }
}

/// World-space frame in which a handle is placed.
///
/// - `origin` — world position of the entity (handles are drawn around it).
/// - `basis` — display orientation. `Mat3::IDENTITY` in World mode,
///   the entity's world rotation in Local mode. Drives visual cube /
///   arrow / torus orientation and the drag axis selection.
/// - `entity_world_rotation` — always the entity's actual world
///   rotation, regardless of mode. Needed for handles that have to
///   convert between world and local spaces (e.g. scale's
///   stretch-matrix conversion in World mode). Equal to `basis` in
///   Local mode; differs in World mode.
#[derive(Debug, Clone, Copy)]
pub struct HandleFrame {
    pub origin: Vec3,
    pub basis: Mat3,
    pub entity_world_rotation: Mat3,
}

impl Default for HandleFrame {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            basis: Mat3::IDENTITY,
            entity_world_rotation: Mat3::IDENTITY,
        }
    }
}

impl HandleFrame {
    /// Maps a local-space direction to world space using the frame's
    /// rotation. For built-in axis handles this turns `Axis::X` into
    /// either `(1,0,0)` (World mode) or the entity's local +X
    /// direction (Local mode).
    pub fn world_axis(&self, axis: Axis) -> Vec3 {
        (self.basis * axis.vec()).normalize_or(axis.vec())
    }
}

/// Interactive handle — produces a visual, accepts picks, applies drags.
pub trait Handle: Send + Sync + 'static {
    /// Which editor mode this handle is part of. `HandleSet` filters
    /// handles by their mode each frame so only the active set renders
    /// / picks / drags.
    fn mode(&self) -> HandleMode;

    /// Draws the handle into the gizmo batch. `frame` carries the
    /// world-space origin and basis; handles align their geometry to
    /// the basis so the Local/World inspector toggle is honored.
    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState);

    /// Returns the distance along `ray` if the handle is hit, `None`
    /// otherwise. Smallest distance wins when multiple handles are hit.
    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32>;

    /// Returns the world-space transform delta for one drag frame.
    /// Variant depends on the handle kind: translate handles return
    /// `Translation`, rotate handles return `Rotation`, scale handles
    /// return `Scale`.
    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> TransformDelta;
}

// ---------------------------------------------------------------------------
// HandleSet — state machine + coordinator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum SetState {
    Idle,
    Hover(usize),
    /// Drag in progress. `start_ray` stays fixed for the whole drag;
    /// `last_ray` is the previous frame's ray.
    Drag {
        idx: usize,
        start_ray: Ray,
        last_ray: Ray,
    },
}

/// Coordinator for a group of handles. Owns the `Idle → Hover → Drag`
/// state machine, dispatches input to the right handle, and forwards
/// the resulting translation delta back to the editor.
///
/// The default `HandleSet` contains three [`TranslateHandle`]s (X, Y, Z).
pub struct HandleSet {
    handles: Vec<Box<dyn Handle>>,
    state: SetState,
    frame: HandleFrame,
    mode: HandleMode,
}

impl Default for HandleSet {
    fn default() -> Self {
        Self {
            handles: vec![
                Box::new(TranslateHandle::new(Axis::X)),
                Box::new(TranslateHandle::new(Axis::Y)),
                Box::new(TranslateHandle::new(Axis::Z)),
                Box::new(PlaneHandle::new(Axis::X, Axis::Y)),
                Box::new(PlaneHandle::new(Axis::X, Axis::Z)),
                Box::new(PlaneHandle::new(Axis::Y, Axis::Z)),
                Box::new(RotateHandle::new(Axis::X)),
                Box::new(RotateHandle::new(Axis::Y)),
                Box::new(RotateHandle::new(Axis::Z)),
                Box::new(ScaleHandle::axis(Axis::X)),
                Box::new(ScaleHandle::axis(Axis::Y)),
                Box::new(ScaleHandle::axis(Axis::Z)),
                Box::new(ScaleHandle::center()),
            ],
            state: SetState::Idle,
            frame: HandleFrame::default(),
            mode: HandleMode::Translate,
        }
    }
}

impl HandleSet {
    /// Empty handle set. Use [`Self::default`] for the standard
    /// translate gizmo (3 axis arrows).
    pub fn new() -> Self {
        Self {
            handles: Vec::new(),
            state: SetState::Idle,
            frame: HandleFrame::default(),
            mode: HandleMode::Translate,
        }
    }

    pub fn add(&mut self, handle: Box<dyn Handle>) {
        self.handles.push(handle);
    }

    /// Updates the world-space origin where handles render. Called every
    /// frame from the entity's `GlobalTransform`.
    pub fn set_origin(&mut self, origin: Vec3) {
        self.frame.origin = origin;
    }

    /// Updates the rotation basis applied to local axes. `Mat3::IDENTITY`
    /// for World-space mode, the entity's world rotation for Local mode.
    pub fn set_basis(&mut self, basis: Mat3) {
        self.frame.basis = basis;
    }

    /// Sets the entity's actual world rotation, used by handles that
    /// need world→local space conversion regardless of display mode
    /// (notably `ScaleHandle` in World mode).
    pub fn set_entity_rotation(&mut self, rotation: Mat3) {
        self.frame.entity_world_rotation = rotation;
    }

    /// Switches the active edit mode. Filters which handles render /
    /// pick / drag this frame. Resets transient hover/drag state when
    /// the mode actually changes so leftover state from another mode
    /// doesn't bleed into the new one.
    pub fn set_mode(&mut self, mode: HandleMode) {
        if self.mode != mode {
            self.mode = mode;
            self.state = SetState::Idle;
        }
    }

    pub fn mode(&self) -> HandleMode {
        self.mode
    }

    /// Returns `true` if a handle is currently hovered or being dragged.
    /// The editor uses this to suppress camera input.
    pub fn is_active(&self) -> bool {
        !matches!(self.state, SetState::Idle)
    }

    /// Returns `true` if a drag is in progress.
    pub fn is_dragging(&self) -> bool {
        matches!(self.state, SetState::Drag { .. })
    }

    /// Processes one frame of input. Returns the [`TransformDelta`] to
    /// apply to the selected entity (`TransformDelta::none()` if no
    /// drag is active or the active handle's mode is filtered out).
    pub fn update(
        &mut self,
        ray: Option<Ray>,
        lmb_pressed: bool,
        lmb_held: bool,
        modifiers: DragModifiers,
        snap: SnapSettings,
    ) -> TransformDelta {
        // No cursor over the viewport → drop hover, end drag if any.
        let Some(ray) = ray else {
            self.state = SetState::Idle;
            return TransformDelta::none();
        };

        match self.state {
            SetState::Idle | SetState::Hover(_) => {
                let hit = self.pick_closest(ray);
                self.state = match (hit, lmb_pressed) {
                    (Some(idx), true) => SetState::Drag {
                        idx,
                        start_ray: ray,
                        last_ray: ray,
                    },
                    (Some(idx), false) => SetState::Hover(idx),
                    (None, _) => SetState::Idle,
                };
                TransformDelta::none()
            }
            SetState::Drag {
                idx,
                start_ray,
                last_ray,
            } => {
                if !lmb_held {
                    // Released — return to idle (re-pick on next frame).
                    self.state = SetState::Idle;
                    return TransformDelta::none();
                }
                let drag = DragInfo {
                    start_ray,
                    last_ray,
                    current_ray: ray,
                    modifiers,
                    snap,
                };
                let delta = self.handles[idx].drag(drag, self.frame);
                // Update the last_ray for the next drag tick;
                // start_ray stays fixed for the whole drag.
                self.state = SetState::Drag {
                    idx,
                    start_ray,
                    last_ray: ray,
                };
                delta
            }
        }
    }

    /// Renders the active-mode handles into the gizmo batch with their
    /// current state. Inactive-mode handles are skipped entirely.
    /// **While a drag is active**, sibling handles in the same mode are
    /// also hidden so the user only sees the one they're dragging —
    /// matches Unity / Maya / Blender behavior and reduces visual
    /// clutter while the manipulation is in progress.
    pub fn draw(&self, gizmos: &mut Gizmos<'_>) {
        let dragging_idx = match self.state {
            SetState::Drag { idx, .. } => Some(idx),
            _ => None,
        };
        for (i, h) in self.handles.iter().enumerate() {
            if h.mode() != self.mode {
                continue;
            }
            if let Some(active) = dragging_idx
                && active != i
            {
                continue;
            }
            let state = match self.state {
                SetState::Hover(idx) if idx == i => HandleState::Hover,
                SetState::Drag { idx, .. } if idx == i => HandleState::Dragging,
                _ => HandleState::Idle,
            };
            h.draw(gizmos, self.frame, state);
        }
    }

    fn pick_closest(&self, ray: Ray) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, h) in self.handles.iter().enumerate() {
            if h.mode() != self.mode {
                continue;
            }
            if let Some(t) = h.pick(ray, self.frame) {
                match best {
                    Some((_, best_t)) if best_t <= t => {}
                    _ => best = Some((i, t)),
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

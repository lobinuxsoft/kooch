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
mod translate;

pub use plane::PlaneHandle;
pub use translate::TranslateHandle;

use glam::{Mat3, Vec3};
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
/// active. `last_ray` is from the previous frame, `current_ray` from
/// this frame.
#[derive(Debug, Clone, Copy)]
pub struct DragInfo {
    pub last_ray: Ray,
    pub current_ray: Ray,
}

/// World-space frame in which a handle is placed: origin + orthonormal
/// basis. The editor sets `basis = Mat3::IDENTITY` for World-space mode
/// and `basis = entity_world_rotation` for Local-space mode.
#[derive(Debug, Clone, Copy)]
pub struct HandleFrame {
    pub origin: Vec3,
    pub basis: Mat3,
}

impl Default for HandleFrame {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            basis: Mat3::IDENTITY,
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
    /// Draws the handle into the gizmo batch. `frame` carries the
    /// world-space origin and basis; handles align their geometry to
    /// the basis so the Local/World inspector toggle is honored.
    fn draw(&self, gizmos: &mut Gizmos<'_>, frame: HandleFrame, state: HandleState);

    /// Returns the distance along `ray` if the handle is hit, `None`
    /// otherwise. Smallest distance wins when multiple handles are hit.
    fn pick(&self, ray: Ray, frame: HandleFrame) -> Option<f32>;

    /// Returns the world-space translation delta for one drag frame.
    /// Called repeatedly while the user drags this handle.
    fn drag(&self, drag: DragInfo, frame: HandleFrame) -> Vec3;
}

// ---------------------------------------------------------------------------
// HandleSet — state machine + coordinator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum SetState {
    Idle,
    Hover(usize),
    Drag(usize, Ray),
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
            ],
            state: SetState::Idle,
            frame: HandleFrame::default(),
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

    /// Returns `true` if a handle is currently hovered or being dragged.
    /// The editor uses this to suppress camera input.
    pub fn is_active(&self) -> bool {
        !matches!(self.state, SetState::Idle)
    }

    /// Returns `true` if a drag is in progress.
    pub fn is_dragging(&self) -> bool {
        matches!(self.state, SetState::Drag(..))
    }

    /// Processes one frame of input. Returns the translation delta to
    /// apply to the selected entity (`Vec3::ZERO` if no drag is active).
    pub fn update(
        &mut self,
        ray: Option<Ray>,
        lmb_pressed: bool,
        lmb_held: bool,
    ) -> Vec3 {
        // No cursor over the viewport → drop hover, end drag if any.
        let Some(ray) = ray else {
            self.state = SetState::Idle;
            return Vec3::ZERO;
        };

        match self.state {
            SetState::Idle | SetState::Hover(_) => {
                let hit = self.pick_closest(ray);
                self.state = match (hit, lmb_pressed) {
                    (Some(idx), true) => SetState::Drag(idx, ray),
                    (Some(idx), false) => SetState::Hover(idx),
                    (None, _) => SetState::Idle,
                };
                Vec3::ZERO
            }
            SetState::Drag(idx, last_ray) => {
                if !lmb_held {
                    // Released — return to idle (re-pick on next frame).
                    self.state = SetState::Idle;
                    return Vec3::ZERO;
                }
                let drag = DragInfo {
                    last_ray,
                    current_ray: ray,
                };
                let delta = self.handles[idx].drag(drag, self.frame);
                // Update the last_ray for the next drag tick.
                self.state = SetState::Drag(idx, ray);
                delta
            }
        }
    }

    /// Renders all handles into the gizmo batch with current state.
    pub fn draw(&self, gizmos: &mut Gizmos<'_>) {
        for (i, h) in self.handles.iter().enumerate() {
            let state = match self.state {
                SetState::Hover(idx) if idx == i => HandleState::Hover,
                SetState::Drag(idx, _) if idx == i => HandleState::Dragging,
                _ => HandleState::Idle,
            };
            h.draw(gizmos, self.frame, state);
        }
    }

    fn pick_closest(&self, ray: Ray) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, h) in self.handles.iter().enumerate() {
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

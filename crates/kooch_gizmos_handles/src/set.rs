//! [`HandleSet`] — the `Idle → Hover → Drag` state machine that owns
//! the handles, dispatches input, and forwards the active handle's
//! [`TransformDelta`] back to the editor.

use glam::{Mat3, Vec3};
use kooch_gizmos::Gizmos;

use crate::{
    Axis, DragInfo, DragModifiers, Handle, HandleFrame, HandleMode, HandleState, PlaneHandle, Ray,
    RotateHandle, ScaleHandle, SnapSettings, TransformDelta, TranslateHandle,
};

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

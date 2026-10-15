//! [`HandleSet`] — the `Idle → Hover → Drag` state machine that owns
//! the handles, dispatches input, and forwards the active handle's
//! [`TransformDelta`] back to the editor.

use glam::{Mat3, Vec3};
use kooch_gizmos::Gizmos;

use crate::{
    Axis, AxisMask, DragInfo, DragModifiers, Handle, HandleFrame, HandleMode, HandleState,
    PlaneHandle, Ray, RotateHandle, ScaleHandle, SnapSettings, TransformDelta, TranslateHandle,
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

/// The modes in the order a selection falls back through them: moving a thing is the one gesture
/// nearly everything accepts.
const MODES: [HandleMode; 3] = [HandleMode::Translate, HandleMode::Rotate, HandleMode::Scale];

/// Coordinator for a group of handles: runs `Idle → Hover → Drag`, dispatches input to the right
/// handle and returns its transform delta.
pub struct HandleSet {
    handles: Vec<Box<dyn Handle>>,
    state: SetState,
    frame: HandleFrame,
    mode: HandleMode,
    /// What the current selection offers, per mode: Translate, Rotate, Scale (#1469). All three
    /// axes everywhere until a consumer narrows it, which is what an entity wants.
    allowed: [AxisMask; 3],
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
            allowed: [AxisMask::ALL; 3],
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
            allowed: [AxisMask::ALL; 3],
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

    /// Updates the world size one reference unit of handle covers, so the set holds its size on
    /// screen (#1433). Called every frame from the camera and the origin above.
    pub fn set_scale(&mut self, scale: f32) {
        // 🔴 `!(scale > 0.0)`, not `scale <= 0.0`: a NaN passes the `<=` and would collapse every
        // handle to a point that cannot be picked.
        self.frame.scale = match !(scale > 0.0) || !scale.is_finite() {
            true => 1.0,
            false => scale,
        };
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

    /// Declares what the selection offers, per mode (#1469). Call every frame, before `update`:
    /// the answer depends on what is selected, and a stale mask edits by a rule the author's last
    /// click retired.
    ///
    /// Drag state is dropped when the offer changes, so a drag cannot survive into a mode that no
    /// longer accepts it.
    pub fn set_allowed(&mut self, translate: AxisMask, rotate: AxisMask, scale: AxisMask) {
        let wanted = [translate, rotate, scale];
        if self.allowed == wanted {
            return;
        }
        self.allowed = wanted;
        self.state = SetState::Idle;
        // 🔴 The mode the author left behind may be one the new selection does not offer, and
        // staying in it is the very bug this mask exists to remove — a gizmo that draws nothing.
        if !self.offers(self.mode) {
            self.mode = MODES
                .into_iter()
                .find(|mode| self.offers(*mode))
                .unwrap_or(HandleMode::Translate);
        }
    }

    /// Whether `mode` means anything to the current selection. The toolbar reads this to grey a
    /// button out, rather than offer a gizmo that would do nothing.
    pub fn offers(&self, mode: HandleMode) -> bool {
        !self.mask(mode).is_empty()
    }

    /// Switches the edit mode, resetting hover and drag state when it changes so none bleeds into
    /// the new mode.
    ///
    /// A mode the selection does not offer is refused: switching to a gizmo that draws nothing
    /// leaves the author with no gizmo and no reason why.
    pub fn set_mode(&mut self, mode: HandleMode) {
        if !self.offers(mode) {
            return;
        }
        if self.mode != mode {
            self.mode = mode;
            self.state = SetState::Idle;
        }
    }

    fn mask(&self, mode: HandleMode) -> AxisMask {
        match mode {
            HandleMode::Translate => self.allowed[0],
            HandleMode::Rotate => self.allowed[1],
            HandleMode::Scale => self.allowed[2],
        }
    }

    /// Whether handle `h` renders, picks and drags right now.
    fn offered(&self, h: &dyn Handle) -> bool {
        h.mode() == self.mode && self.mask(self.mode).covers(h.axes())
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
    /// Where the handles are, in world space.
    pub fn origin(&self) -> Vec3 {
        self.frame.origin
    }

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

    /// Renders the active mode's handles. While dragging, only the dragged handle shows, as in
    /// Unity, Maya and Blender.
    pub fn draw(&self, gizmos: &mut Gizmos<'_>) {
        let dragging_idx = match self.state {
            SetState::Drag { idx, .. } => Some(idx),
            _ => None,
        };
        for (i, h) in self.handles.iter().enumerate() {
            if !self.offered(h.as_ref()) {
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
            if !self.offered(h.as_ref()) {
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

#[cfg(test)]
mod tests;

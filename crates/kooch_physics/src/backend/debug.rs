//! Line segments describing what the solver actually holds.
//!
//! # Why this exists when colliders are already drawn
//!
//! The editor's `ColliderVisualizer` draws a collider from its ECS
//! components, folding scale the same way the physics does. That is the
//! right tool for "does this shape wrap my model", and it is the common
//! case.
//!
//! It cannot answer the other question. Drawing the components is the same
//! arithmetic done twice: if the sync layer never built the body, or built
//! it from a stale spec, or the solver moved it somewhere the ECS has not
//! heard about, the gizmo shows the shape that *should* exist and says
//! nothing about the one that does. **When those two disagree, the
//! disagreement is the bug**, and only one of them can report it.
//!
//! So this is deliberately not a second collider outline. It is the
//! solver's own account of itself: contacts, centres of mass, joint
//! anchors, broad-phase bounds, and which bodies it has stopped
//! simulating. None of that is derivable from the components at all.
//!
//! # Line segments, not draw calls
//!
//! The backend produces geometry and never renders. Rapier's
//! `DebugRenderPipeline` walks the world and hands over pairs of points —
//! already tessellated, so a sphere arrives as segments rather than a
//! centre and a radius — and something else decides what to do with them.
//! That keeps the physics crate free of any opinion about rendering, and
//! keeps rapier's types out of [`PhysicsBackend`](super::PhysicsBackend).

use glam::Vec3;

/// One segment of the debug overlay, in world space.
///
/// Colour is linear RGB, already resolved from whatever the backend used
/// internally. A consumer pushes these straight into a line renderer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Vec3,
}

/// Which parts of the solver to describe.
///
/// Separate switches rather than one flag because they answer different
/// questions and cost different amounts: contacts are cheap and usually
/// what you want, collider shapes are the expensive one and are mostly
/// redundant with the component gizmo.
///
/// # Default
///
/// Everything off. The overlay is a tool, and a tool that is on by default
/// is clutter — the walk is per-frame CPU work proportional to shape count
/// times tessellation, so an unused overlay should cost exactly nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DebugCategories {
    /// Collider outlines **as the solver holds them**, which is the point:
    /// compared against the component gizmo, a mismatch is a sync bug.
    /// Off by default because the component gizmo already covers the
    /// ordinary case, and this is the expensive category.
    pub collider_shapes: bool,
    /// Where bodies are actually touching. The question "is friction doing
    /// this" is unanswerable without it.
    pub contacts: bool,
    /// Joint anchors and the separation between them — a joint anchored to
    /// the wrong point looks identical to one that is not working.
    pub joints: bool,
    /// Broad-phase bounds. Mostly useful when something is not colliding
    /// at all and the question is whether the broad phase can even see it.
    pub collider_aabbs: bool,
    /// Each body's local axes, drawn at its **centre of mass** — the thing
    /// that made #618 impossible to diagnose by looking.
    pub body_axes: bool,
}

impl DebugCategories {
    /// Everything on. For a screenshot, or when you have no idea yet.
    pub fn all() -> Self {
        Self {
            collider_shapes: true,
            contacts: true,
            joints: true,
            collider_aabbs: true,
            body_axes: true,
        }
    }

    /// Whether anything at all is switched on.
    ///
    /// The caller checks this before asking, so a disabled overlay does not
    /// even reach the backend — "costs nothing when off" has to mean the
    /// walk never happens, not that it happens and returns nothing.
    pub fn any(&self) -> bool {
        self.collider_shapes
            || self.contacts
            || self.joints
            || self.collider_aabbs
            || self.body_axes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tool that is on by default is clutter, and this one costs frame
    /// time to produce.
    #[test]
    fn nothing_is_enabled_by_default() {
        assert!(!DebugCategories::default().any());
    }

    #[test]
    fn all_enables_every_category() {
        let all = DebugCategories::all();
        assert!(all.any());
        for enabled in [
            all.collider_shapes,
            all.contacts,
            all.joints,
            all.collider_aabbs,
            all.body_axes,
        ] {
            assert!(enabled, "a category is missing from `all`");
        }
    }

    /// `any` has to notice each switch on its own, or turning one on
    /// silently draws nothing.
    #[test]
    fn any_notices_each_category_alone() {
        let cases = [
            DebugCategories {
                collider_shapes: true,
                ..Default::default()
            },
            DebugCategories {
                contacts: true,
                ..Default::default()
            },
            DebugCategories {
                joints: true,
                ..Default::default()
            },
            DebugCategories {
                collider_aabbs: true,
                ..Default::default()
            },
            DebugCategories {
                body_axes: true,
                ..Default::default()
            },
        ];
        for case in cases {
            assert!(case.any(), "{case:?} reports nothing enabled");
        }
    }
}

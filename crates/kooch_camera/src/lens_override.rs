//! [`LensOverride`] — what a vcam asks of the lens while it is live: Cinemachine's `LensSettings`
//! (#1254).
//!
//! 🔴 **A vcam is not a camera.** It does not render: no culling mask, no clear colour, no overlay.
//! The lens belongs to [`PerspectiveCamera`], which already carries `fov`, `near` and `far`, and
//! every camera has one. A vcam is a director — it says where to stand and what to look at — and a
//! director does not HAVE a lens, it ASKS for one. Asking is what is optional, which is why this is
//! a component and not four fields with an "inherit" flag each.
//!
//! 🔴 **Inheritance is the absence of this component**, not a flag per field. Cinemachine's lens
//! always holds concrete values — `LensSettings.Default` is FOV 40, near 0.1, far 5000 — and
//! `FromCamera()` copies the camera's **once, at creation**. The only thing it inherits per frame
//! is the projection mode and the aspect. A flag per field would have to answer "what do I
//! interpolate against when one side of a blend inherits and the other does not", a question
//! Cinemachine never has to answer because both sides always hold numbers.
//!
//! So a vcam without this component is seen through the driven camera's lens, exactly as every
//! scene was before it existed, and one with it overrides. The same shape as a body or an aim being
//! a component rather than an enum beside one (#1397).

use kooch_ecs::Reflect;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::reflect::FieldRange;

/// What a vcam asks of the driven camera's lens while it is live.
///
/// Add it to a vcam that has to look wider or narrower than the others; leave it off and the
/// camera keeps the lens it was authored with. A handover interpolates whatever each side ends up
/// with, so a vcam that asks and one that does not still blend.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct LensOverride {
    /// Vertical field of view, in degrees. What "wider" and "narrower" mean.
    #[reflect(range = FOV_RANGE)]
    pub fov: f32,
    /// Nearest distance drawn.
    #[reflect(range = NEAR_RANGE)]
    pub near: f32,
    /// Furthest distance drawn.
    #[reflect(range = FAR_RANGE)]
    pub far: f32,
    /// Roll around the view axis, in degrees. Positive tilts the horizon clockwise.
    ///
    /// 🔴 Applied where the pose is written to the render camera, **not** mixed into the vcam's
    /// rotation. Aim owns that rotation and a second writer to it is the bug this rig keeps having
    /// (#1361). Being a scalar beside the pose also makes it interpolate linearly, which is what a
    /// roll should do — inside a slerp it would be at the mercy of the rest of the rotation.
    #[reflect(range = DUTCH_RANGE)]
    pub dutch: f32,
}

/// Wide enough for a fisheye, narrow enough for a scope, and never the degenerate ends `Lens`
/// clamps away anyway.
const FOV_RANGE: FieldRange = FieldRange {
    min: 1.0,
    max: 179.0,
    step: 0.5,
};

const NEAR_RANGE: FieldRange = FieldRange {
    min: 0.001,
    max: 10.0,
    step: 0.001,
};

const FAR_RANGE: FieldRange = FieldRange {
    min: 1.0,
    max: 100_000.0,
    step: 1.0,
};

/// A full turn either way: a roll past 180° is the same picture reached the long way, and a rig
/// that wants to spin the horizon is animating this rather than authoring it.
const DUTCH_RANGE: FieldRange = FieldRange {
    min: -180.0,
    max: 180.0,
    step: 1.0,
};

impl Default for LensOverride {
    /// The engine's own camera, so adding the component changes nothing until a field is moved.
    ///
    /// 🔴 Not Cinemachine's 40°/5000: those are Unity's defaults, and a component that changes the
    /// picture the moment it is added reads as a bug.
    fn default() -> Self {
        let camera = PerspectiveCamera::default();
        Self {
            fov: camera.fov,
            near: camera.near,
            far: camera.far,
            dutch: 0.0,
        }
    }
}

impl LensOverride {
    /// Every field moved towards `to` by `t`, for a handover.
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * t;
        Self {
            fov: mix(self.fov, to.fov),
            near: mix(self.near, to.near),
            far: mix(self.far, to.far),
            dutch: mix(self.dutch, to.dutch),
        }
    }
}

impl Component for LensOverride {}

/// The lens `vcam` ends up with: what it asks for, or the camera's own when it asks for nothing.
pub fn lens_of(registry: &ComponentRegistry, vcam: Entity, camera: LensOverride) -> LensOverride {
    registry
        .get_cpu::<LensOverride>()
        .and_then(|storage| storage.get(vcam))
        .copied()
        .unwrap_or(camera)
}

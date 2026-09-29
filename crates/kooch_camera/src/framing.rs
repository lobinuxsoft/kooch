//! [`CameraFraming`] — where on screen a vcam holds its target, and how far the target may wander
//! before the camera answers: Cinemachine's composer dead and soft zones (#1252).
//!
//! 🔴 Framing **turns** the camera and never moves it (#1361), as `CinemachineRotationComposer`
//! does: *"The composer does not change the camera's position. It will only pan and tilt the camera
//! where it is."* The Aim stage has exactly one owner, and a `look_at` of `Composed` is how a vcam
//! says this is it.
//!
//! #1329 had it the other way round, on a premise that was false: that in a third-person rig the
//! rotation belongs to the player. It does not — the stick moves the **body**, both here and in
//! `OrbitalFollow`. The fight was this against `look_at`, two owners of the rotation, and moving
//! this one to the position only postponed it until a shoulder owned the position too (#1359).
//!
//! The error is an **angle**, measured off the orientation the camera already has, so there is no
//! slack to remember: a correction carried in a field is a second opinion about where the camera
//! points. Pan is about the vcam's own up, so the horizon stays level under gravity.

use std::collections::HashMap;

use crate::frame::CameraFrame;
use glam::{Vec2, Vec3};
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::FieldRange;

/// Frames the target of the vcam it sits on. Beside a [`VirtualCamera`]; replaces its position
/// damping, since the soft zone is the easing.
///
/// [`VirtualCamera`]: crate::VirtualCamera
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraFraming {
    /// Off aims at the target itself, as without this component. The rig follows it either way.
    pub enabled: bool,
    /// Where the target sits on screen: `0` is the centre, `±0.5` the edges, +Y up.
    #[reflect(range = SCREEN_RANGE)]
    pub screen: Vec2,
    /// Width and height, as fractions of the screen, the target moves inside with the camera not
    /// turning at all.
    #[reflect(range = ZONE_RANGE)]
    pub dead_zone: Vec2,
    /// Width and height over which the camera comes up to speed: none of the correction on the dead
    /// zone's edge, all of it here. Never smaller than the dead zone.
    ///
    /// 🔴 A band, not a wall. Without it the camera goes from not turning to turning at full rate
    /// between two frames, and that step is what reads as a shake; a wall that shoved the camera
    /// back measured three times worse than the rig on its own.
    #[reflect(range = ZONE_RANGE)]
    pub soft_zone: Vec2,
    /// Seconds to close the gap to the dead zone's edge, leaving a hundredth of it behind — the
    /// same easing the rig's damping uses. Zero is rigid.
    ///
    /// 🔴 Not a duration. A tween that restarts whenever its goal moves spends every frame at the
    /// fastest part of its curve and steps whenever the target starts or stops (#1336).
    #[reflect(range = TIME_RANGE, alias = "soft_time")]
    pub soft_time: f32,
}

/// The enabled framing on `vcam`, if it has one.
///
/// 🔴 The only place that question is asked, and the aim is the only thing that asks it. Two
/// lookups would be two answers waiting to disagree.
pub fn of(
    registry: &kooch_ecs::component::ComponentRegistry,
    vcam: Entity,
) -> Option<CameraFraming> {
    registry
        .get_cpu::<CameraFraming>()?
        .get(vcam)
        .copied()
        .filter(|framing| framing.enabled)
}

/// The composed aim: turns the camera so the target lands where it is held on screen. Reached from
/// [`aim_stage`](crate::virtual_camera::aim_stage) when the vcam asks for it, never registered on
/// its own — the Aim stage has one owner.
///
/// Answers whether it ran, so the caller knows not to damp a rotation that is already eased.
pub fn composed(step: &mut crate::rig::RigStep) -> bool {
    let Some(framing) = of(step.registry, step.entity) else {
        return false;
    };
    let mut state = step
        .carried
        .tracked
        .of(step.entity)
        .unwrap_or(Framed::at(step.frame.target));
    framing.compose(
        &mut state,
        &mut step.frame,
        step.up,
        step.reference,
        step.dt,
    );
    step.memory.tracked.set(step.entity, state);
    true
}

/// How far a target or a camera may drift and still count as standing still, in metres.
const STILL: f32 = 1e-5;

const SCREEN_RANGE: FieldRange = FieldRange {
    min: -0.5,
    max: 0.5,
    step: 0.01,
};

const ZONE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 2.0,
    step: 0.01,
};

const TIME_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 3.0,
    step: 0.01,
};

impl Default for CameraFraming {
    fn default() -> Self {
        Self {
            enabled: true,
            screen: Vec2::ZERO,
            dead_zone: Vec2::new(0.1, 0.1),
            soft_zone: Vec2::new(0.6, 0.6),
            soft_time: 0.5,
        }
    }
}

impl Component for CameraFraming {}

/// How much of the world a view shows at one metre: half its height and half its width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    pub half_height: f32,
    pub half_width: f32,
}

impl Lens {
    /// From a vertical field of view in degrees and a width over height.
    pub fn new(fov: f32, aspect: f32) -> Self {
        let half_height = (fov.clamp(1.0, 179.0).to_radians() * 0.5).tan();
        Self {
            half_height,
            half_width: half_height * aspect.max(0.01),
        }
    }

    /// The angle, in degrees, a point sits off the view axis when it is `fraction` of the screen
    /// from its centre, per axis.
    ///
    /// 🔴 Exact, not the field of view times the fraction. Cinemachine takes the linear reading
    /// (`(rect.yMin - 0.5) * fov`), and on a 90° screen it puts a `screen` of `0.25` at `0.207` —
    /// a field whose own documentation says `±0.5` is the edge cannot be a quarter of the way and
    /// land somewhere else.
    pub fn angle(&self, fraction: Vec2) -> Vec2 {
        Vec2::new(
            (fraction.x * 2.0 * self.half_width).atan().to_degrees(),
            (fraction.y * 2.0 * self.half_height).atan().to_degrees(),
        )
    }

    /// The screen's size in metres at `depth`.
    /// Public because a gizmo needs it: putting a point on screen is exactly this, and a second
    /// copy of the arithmetic in the editor is a second place for it to drift.
    pub fn span(&self, depth: f32) -> Vec2 {
        Vec2::new(self.half_width, self.half_height) * 2.0 * depth.max(0.01)
    }
}

impl CameraFraming {
    /// Pans and tilts so the target lands where it is held on screen, copied from
    /// `CinemachineRotationComposer::RotateToScreenBounds`: the angular error, clamped to the dead
    /// zone, eased once.
    ///
    /// Read from where the camera **is**, walls included — the body and the deoccluder have both had
    /// their say by now, and aiming from where the body wanted it would point past a camera that got
    /// pushed.
    pub fn compose(
        &self,
        state: &mut Framed,
        frame: &mut CameraFrame,
        up: Vec3,
        reference: Vec3,
        dt: f32,
    ) {
        // 🔴 Level before measuring, as Cinemachine rebuilds its basis with
        // `Quaternion.LookRotation(dir, ReferenceUp)` before every correction. This turns the
        // orientation the camera already has, so a roll it picks up is kept for ever — and under an
        // `up` that moves, a pan about the new one applied to a basis built for the old is not a
        // pure yaw, so one is picked up every frame (#1363). A rebuilt basis has none by
        // construction, which is why `look_at` never had this.
        frame.rotation =
            crate::virtual_camera::look_at(Vec3::ZERO, frame.rotation * Vec3::NEG_Z, up, reference);
        let target = frame.target;
        let at = seen_at(frame.rotation, target - frame.position, up);
        // `frame.screen` already carries the lead, so one thing decides where the character sits
        // (#1330). Halves, because a zone is measured from its centre out.
        let at = at - frame.lens.angle(self.screen + frame.screen);
        let dead = frame.lens.angle(self.dead_zone * 0.5);
        let soft = frame.lens.angle(self.soft_zone.max(self.dead_zone) * 0.5);

        // Nothing inside the dead zone, the excess outside it.
        let owed = past(at, dead);
        // 🔴 Ramped while the target moves, whole once it stops. Not to keep a promise — the
        // easing is exponential and promises nothing — but because a correction that ramps to
        // nothing at the dead zone's edge approaches it and never arrives: the target parks in the
        // soft band, a third of the way out, for ever. Measured at 0.134 against the edge's 0.1.
        let share = match state.last.abs_diff_eq(target, STILL) {
            true => Vec2::ONE,
            false => Vec2::new(ramp(at.x, dead.x, soft.x), ramp(at.y, dead.y, soft.y)),
        };
        // One quantity, eased once: the residual angle. The vcam's own rotation damping does not run
        // on top of this — two eases in series on one quantity is #1329, on the other axis.
        let alpha = crate::virtual_camera::settled(dt, self.soft_time);
        frame.rotation = turned(frame.rotation, owed * share * alpha, up);
        state.last = target;
    }
}

/// Where `direction` sits off the view axis, in degrees, read in the **screen's** own axes: `+x`
/// right, `+y` up. Cinemachine's `GetCameraRotationToTarget`, with the axes named rather than swapped
/// into a `Vector2`.
fn seen_at(rotation: glam::Quat, direction: Vec3, up: Vec3) -> Vec2 {
    if direction.length_squared() < 1e-12 {
        return Vec2::ZERO;
    }
    let direction = direction.normalize();
    let forward = rotation * Vec3::NEG_Z;
    let flat = direction - up * direction.dot(up);
    // Straight along `up`: no horizon direction to pan towards, so the tilt says all of it.
    let pan = match flat.length_squared() > 1e-12 {
        true => signed(
            crate::virtual_camera::flattened(forward, up),
            flat.normalize(),
            up,
        ),
        false => 0.0,
    };
    let panned = glam::Quat::from_axis_angle(up, pan.to_radians()) * forward;
    Vec2::new(-pan, signed(panned, direction, panned.cross(up)))
}

/// Turns the camera towards a target sitting `at` degrees off the view axis, in the screen's axes.
/// Pan about `up` first, then tilt about the camera's own right, so the horizon never rolls —
/// Cinemachine's `ApplyCameraRotation`.
///
/// 🔴 The pan is negated here and nowhere else. A rotation about `up` carries the view towards
/// `-right`, so a target on the right is reached by a **negative** pan while a target above is
/// reached by a positive tilt. One axis disagrees with the screen, and this is the one line that
/// knows it.
fn turned(rotation: glam::Quat, at: Vec2, up: Vec3) -> glam::Quat {
    let panned = glam::Quat::from_axis_angle(up, (-at.x).to_radians()) * rotation;
    panned * glam::Quat::from_rotation_x(at.y.to_radians())
}

/// The angle from `from` to `to` about `axis`, in degrees, signed by which way round it goes.
fn signed(from: Vec3, to: Vec3, axis: Vec3) -> f32 {
    from.cross(to)
        .dot(axis.normalize())
        .atan2(from.dot(to))
        .to_degrees()
}

/// How much of the correction applies at `at`: none on the dead zone's edge, all of it on the soft
/// one, and all of it beyond. Both zones as **half**-widths. A band of zero width is a step, which is
/// what a soft zone the same size as the dead one asks for.
fn ramp(at: f32, dead: f32, soft: f32) -> f32 {
    let band = (soft - dead).max(0.0);
    match band > 0.0 {
        true => ((at.abs() - dead.max(0.0)) / band).clamp(0.0, 1.0),
        false => 1.0,
    }
}

/// How far past a zone of `half` an error sits, per axis, in whatever unit both are given in.
fn past(at: Vec2, half: Vec2) -> Vec2 {
    let beyond = |at: f32, half: f32| at.signum() * (at.abs() - half.max(0.0)).max(0.0);
    Vec2::new(beyond(at.x, half.x), beyond(at.y, half.y))
}

/// One vcam's framing in flight: where the target was last step, which is the whole of it — the
/// correction itself is read off the camera's own orientation.
#[derive(Debug, Clone, Copy)]
pub struct Framed {
    last: Vec3,
}

impl Framed {
    /// Framed on a target that has not moved yet.
    pub fn at(target: Vec3) -> Self {
        Self { last: target }
    }
}

/// Every framed vcam's tracked point, carried between steps. Rebuilt from the vcams seen each step,
/// so a despawned one leaves nothing behind.
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    points: HashMap<Entity, Framed>,
}

impl Tracked {
    /// This vcam's framing, or `None` on its first framed step.
    pub fn of(&self, entity: Entity) -> Option<Framed> {
        self.points.get(&entity).copied()
    }

    pub fn set(&mut self, entity: Entity, framed: Framed) {
        self.points.insert(entity, framed);
    }
}

#[cfg(test)]
mod tests;

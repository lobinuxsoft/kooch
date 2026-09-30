//! [`VirtualCamera`] — camera behaviour as data a designer authors, with modes and names from
//! phantom-camera (MIT, #671).

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition};

/// No follow logic; the pose is whatever else wrote it.
pub const FOLLOW_NONE: u32 = 0;
/// Sits exactly on the target.
pub const FOLLOW_GLUED: u32 = 1;
/// The target's position plus a fixed offset.
pub const FOLLOW_SIMPLE: u32 = 2;
/// A spring arm on the target, rotatable around it — Cinemachine's `OrbitalFollow`.
pub const FOLLOW_ORBITAL: u32 = 3;
/// A [`PositionComposer`](crate::PositionComposer) moves the camera so the target lands where it
/// belongs on screen — Cinemachine's `PositionComposer` (#1369).
pub const FOLLOW_POSITION_COMPOSER: u32 = 4;
/// The same arm, over a shoulder: a pivot chain from the target through the shoulder and the hand —
/// Cinemachine's `ThirdPersonFollow` (#1380). A shooter's body.
pub const FOLLOW_SHOULDER: u32 = 5;

/// No rotation logic.
pub const LOOK_AT_NONE: u32 = 0;
/// Copies the target's rotation.
pub const LOOK_AT_MIMIC: u32 = 1;
/// Points straight at the target.
pub const LOOK_AT_SIMPLE: u32 = 2;
/// Looks along the spring arm: the orbit's own direction, so a shoulder offset stays off centre
/// instead of being turned back into the middle.
pub const LOOK_AT_ARM: u32 = 3;
/// A [`RotationComposer`](crate::RotationComposer) owns the rotation: it pans and tilts to hold the target
/// where it belongs on screen, inside a dead zone. Cinemachine's `RotationComposer` (#1361).
pub const LOOK_AT_COMPOSED: u32 = 4;

/// An inactive vcam computes nothing.
pub const INACTIVE_NEVER: u32 = 0;
/// An inactive vcam keeps updating.
pub const INACTIVE_ALWAYS: u32 = 1;

/// Up is world +Y, whatever the target is doing.
pub const UP_WORLD: u32 = 0;
/// Up is away from the gravity acting where the target is.
pub const UP_GRAVITY: u32 = 1;
/// Up is the target's own up axis.
pub const UP_TARGET: u32 = 2;

/// Labels for the `up_mode` dropdown.
pub static UP_MODE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "World (+Y)",
        value: UP_WORLD as i64,
    },
    FieldChoice {
        label: "Align to gravity",
        value: UP_GRAVITY as i64,
    },
    FieldChoice {
        label: "Align to target",
        value: UP_TARGET as i64,
    },
];

/// Labels for the `follow` dropdown.
pub static FOLLOW_MODE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "None",
        value: FOLLOW_NONE as i64,
    },
    FieldChoice {
        label: "Hard Lock To Target",
        value: FOLLOW_GLUED as i64,
    },
    FieldChoice {
        label: "Follow",
        value: FOLLOW_SIMPLE as i64,
    },
    FieldChoice {
        label: "Orbital Follow",
        value: FOLLOW_ORBITAL as i64,
    },
    FieldChoice {
        label: "Position Composer",
        value: FOLLOW_POSITION_COMPOSER as i64,
    },
    FieldChoice {
        label: "Third Person Follow",
        value: FOLLOW_SHOULDER as i64,
    },
];

/// Labels for the `look_at` dropdown.
pub static LOOK_AT_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "None",
        value: LOOK_AT_NONE as i64,
    },
    FieldChoice {
        label: "Rotate With Follow Target",
        value: LOOK_AT_MIMIC as i64,
    },
    FieldChoice {
        label: "Hard Look At",
        value: LOOK_AT_SIMPLE as i64,
    },
    FieldChoice {
        label: "Pan Tilt",
        value: LOOK_AT_ARM as i64,
    },
    FieldChoice {
        label: "Rotation Composer",
        value: LOOK_AT_COMPOSED as i64,
    },
];

/// Labels for the `inactive_update` dropdown.
pub static INACTIVE_UPDATE_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "Never (cheaper)",
        value: INACTIVE_NEVER as i64,
    },
    FieldChoice {
        label: "Always",
        value: INACTIVE_ALWAYS as i64,
    },
];

/// `offset` only means something to the modes that add one.
pub static OFFSET_WHEN: FieldCondition = FieldCondition {
    field: "follow",
    values: &[FOLLOW_SIMPLE as i64],
};

/// Both shoulders and everywhere between them.
pub static SIDE_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

/// The spring arm's parameters, shared by both bodies built on one.
pub static ARM_WHEN: FieldCondition = FieldCondition {
    field: "follow",
    values: &[FOLLOW_ORBITAL as i64, FOLLOW_SHOULDER as i64],
};

/// The shoulder's own, which only one body reads — so a field never shows where it does nothing.
pub static SHOULDER_WHEN: FieldCondition = FieldCondition {
    field: "follow",
    values: &[FOLLOW_SHOULDER as i64],
};

/// How far a target must move, per axis in world units, before the camera writes a new pose. A
/// floor, not a knob: damping is asymptotic and would otherwise write forever.
pub const SETTLE_EPSILON: f32 = 1e-4;

/// Camera behaviour on a **virtual camera** — its own entity with a framing and a
/// [`Transform`](kooch_ecs::transform::Transform), copied onto the rendering camera by the Host.
/// Defaults to a third-person look-at with no target, so a fresh one moves nothing.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct VirtualCamera {
    /// Which vcam wins when several are live; highest drives the camera. Ties go to the lower
    /// entity index, stable where storage order is not.
    pub priority: i32,
    /// A vcam that is switched off is not a candidate.
    pub enabled: bool,
    /// Where the camera sits. One of the `FOLLOW_*` constants.
    #[reflect(choices = FOLLOW_MODE_CHOICES)]
    pub follow: u32,
    /// Which [`CameraTarget`](crate::CameraTarget) group this framing follows — their weighted
    /// centre when several, inert while none.
    /// A group number, not an entity reference: a query has no identity to lose on reload (#712).
    pub group: u32,
    /// Added to the target's position in `Simple`.
    #[reflect(shown_when = OFFSET_WHEN)]
    pub offset: Vec3,
    /// Spring arm length — how far back from the target the camera sits.
    #[reflect(alias = "distance", shown_when = ARM_WHEN)]
    pub camera_distance: f32,
    /// Rotation around the target's up axis, in degrees.
    #[reflect(shown_when = ARM_WHEN)]
    pub yaw: f32,
    /// Rotation above the horizon, in degrees. Positive looks down.
    #[reflect(shown_when = ARM_WHEN)]
    pub pitch: f32,
    /// Where the arm's pivot sits, offset from the target in the arm's own basis: `x` beside the
    /// view, `y` along `up`, `z` the way the camera looks. An over-the-shoulder view (#1359).
    ///
    /// 🔴 The arm's basis, not the target's. Cinemachine's `ThirdPersonFollow` offsets in the
    /// target's, which it can because its rotation IS the target's; here the orbit owns the yaw, so
    /// a shoulder in the character's frame would swing around it as the player looks about.
    #[reflect(alias = "shoulder", shown_when = SHOULDER_WHEN)]
    pub shoulder_offset: Vec3,
    /// How far the arm's pivot sits above the shoulder — Cinemachine's `VerticalArmLength`.
    ///
    /// 🔴 Not the same axis as `shoulder.y`, which #1359 assumed and was wrong about (#1365). The
    /// shoulder sits in the **levelled** basis and this along the view's own up, which pitches: look
    /// down and the pivot swings forward and down while the shoulder stays. It is what decides how
    /// the target's place on screen moves as the view turns vertically.
    #[reflect(alias = "arm_rise", shown_when = SHOULDER_WHEN)]
    pub vertical_arm_length: f32,
    /// Which shoulder the camera is on: `0` the left, `1` the right, halfway between them centred.
    /// Cinemachine's `CameraSide`, and what a swap animates without touching the authored offset.
    #[reflect(alias = "side", range = SIDE_RANGE, shown_when = SHOULDER_WHEN)]
    pub camera_side: f32,
    /// Where the camera looks. One of the `LOOK_AT_*` constants.
    #[reflect(choices = LOOK_AT_CHOICES)]
    pub look_at: u32,
    /// Seconds the camera takes to reach its pose once the target stops, per world axis — exactly,
    /// a tween that restarts while the target keeps moving. Zero is rigid.
    #[reflect(alias = "damping_value, damping_time")]
    pub damping: Vec3,
    /// The blend a scene wrote before blends belonged to the brain. Ignored, and said in the log
    /// once so the number is not lost silently: a blend is between two vcams, and this is one of
    /// them (#1339).
    #[reflect(hidden, alias = "blend_time")]
    pub blend_duration: f32,
    /// The same, as above.
    #[reflect(hidden)]
    pub blend_curve: u32,
    /// The same, as above.
    #[reflect(hidden)]
    pub blend_ease: u32,
    /// Which way is up for this vcam: one of the `UP_*` constants.
    /// `Gravity` asks the field, since a rolling body's rotation is no up; without `kooch_gravity`
    /// it behaves as `World`.
    #[reflect(choices = UP_MODE_CHOICES)]
    pub up_mode: u32,
    /// Whether a vcam on a camera that is not rendering still computes.
    /// One of the `INACTIVE_*` constants.
    #[reflect(choices = INACTIVE_UPDATE_CHOICES)]
    pub inactive_update: u32,
    /// Seconds to turn into a new orientation, exactly, so a changing up does not snap the horizon.
    /// Rotation only; zero is rigid.
    #[reflect(alias = "rotation_damping_value, rotation_damping_time")]
    pub rotation_damping: f32,
}

/// One axis of exponential easing. `time <= 0` is rigid.
pub(crate) fn eased(current: f32, desired: f32, time: f32, dt: f32) -> f32 {
    current + (desired - current) * settled(dt, time)
}

/// What is left of a gap after `time` seconds; the rest is too small to see, as in Cinemachine.
const RESIDUAL: f32 = 0.01;

/// The fraction of a gap to close this step so that after `time` seconds only [`RESIDUAL`] is
/// left, at any frame rate. `time <= 0` closes it at once.
pub(crate) fn settled(dt: f32, time: f32) -> f32 {
    if time <= 0.0 || dt <= 0.0 {
        return 1.0;
    }
    1.0 - RESIDUAL.powf(dt / time)
}

/// Says once, per vcam, that a blend written on it is not read any more, and what it said.
///
/// 🔴 Not migrated, because there is nowhere unambiguous to put it: several vcams each carry a
/// number and the brain needs one. Silently dropping it would change how a scene cuts with nothing
/// on screen to explain it, so the number is logged and the author moves it (#1339).
pub fn report_moved_blends(resources: &mut kooch_core::resource::Resources) {
    let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<VirtualCamera>() else {
        return;
    };
    for (&entity, vcam) in storage.iter_mut() {
        if vcam.blend_duration < 0.0 {
            continue;
        }
        tracing::info!(
            target: "kooch_camera",
            entity = entity.index(),
            blend_duration = vcam.blend_duration,
            "a virtual camera's blend is the brain's now; set it on the camera that carries the \
             CameraBrain",
        );
        // Negative marks it said: a blend is never negative, so nothing an author can type is lost.
        vcam.blend_duration = -1.0;
    }
}

impl Default for VirtualCamera {
    fn default() -> Self {
        Self {
            priority: 0,
            enabled: true,
            follow: FOLLOW_ORBITAL,
            group: 0,
            offset: Vec3::new(0.0, 2.0, 6.0),
            camera_distance: 6.0,
            yaw: 0.0,
            pitch: 20.0,
            // Centred: an authored scene that never heard of a shoulder is
            // placed exactly where it was.
            shoulder_offset: Vec3::ZERO,
            vertical_arm_length: 0.0,
            // The authored `shoulder.x` as written: `2 × 1 − 1` is one.
            camera_side: 1.0,
            look_at: LOOK_AT_SIMPLE,
            damping: Vec3::splat(0.5),
            up_mode: UP_WORLD,
            // Long enough to read as a transition, short enough not to
            // feel like the game took the camera away.
            blend_duration: 0.5,
            blend_curve: crate::blend::CURVE_SINE,
            blend_ease: crate::blend::EASE_IN_OUT,
            inactive_update: INACTIVE_NEVER,
            rotation_damping: 0.5,
        }
    }
}

impl Component for VirtualCamera {}

impl VirtualCamera {
    /// Whether this vcam has anything to do as far as it can tell alone; whether its group has
    /// members is checked where the pose is planned.
    pub fn is_inert(&self) -> bool {
        !self.enabled || (self.follow == FOLLOW_NONE && self.look_at == LOOK_AT_NONE)
    }

    /// Where this vcam wants to stand, before any damping: a pure function of the point it follows,
    /// where it is now and a resolved `up`.
    ///
    /// `current` is what `Follow::None` keeps, which is what lets a vcam look without moving.
    pub fn wanted(&self, target: Vec3, current: Vec3, up: Vec3, reference: Vec3) -> Vec3 {
        let up = normalised_up(up);
        match self.follow {
            FOLLOW_GLUED => target,
            FOLLOW_SIMPLE => target + self.offset,
            FOLLOW_ORBITAL | FOLLOW_SHOULDER => target + self.arm(up, reference),
            _ => current,
        }
    }

    /// Where this vcam wants to look from `eye`, before any damping.
    ///
    /// `current` is what `LookAt::None` keeps, so a rig can move without turning.
    pub fn aimed(
        &self,
        eye: Vec3,
        target: Vec3,
        target_rot: glam::Quat,
        current: glam::Quat,
        up: Vec3,
        reference: Vec3,
    ) -> glam::Quat {
        let up = normalised_up(up);
        match self.look_at {
            LOOK_AT_MIMIC => target_rot,
            LOOK_AT_SIMPLE => look_at(eye, target, up, reference),
            // A direction, not the pivot: the eye is eased and the aim is not, so a rigid view
            // along the arm cannot inherit the body's lag.
            LOOK_AT_ARM => {
                let along = self.along(self.back(up, reference), up);
                look_at(eye, eye - along, up, reference)
            }
            _ => current,
        }
    }

    /// The spring arm's offset: its pivot, plus its reach from there. Fixed length; shortening
    /// against obstacles needs #562.
    ///
    /// Two segments, as Cinemachine's `ThirdPersonFollow`: **yaw swings the whole basis, pitch turns
    /// only the reach.** That is what keeps a shoulder beside the head instead of rolling it under
    /// the character when the player looks down.
    fn arm(&self, up: Vec3, reference: Vec3) -> Vec3 {
        let (_, _, hand) = self.rig_positions(Vec3::ZERO, up, reference);
        hand + self.along(self.back(up, reference), up) * self.camera_distance.max(0.0)
    }

    /// The arm's three pivots around `target`: its root, the shoulder, and the hand the camera
    /// reaches back from. Cinemachine's `GetRigPositions`, and public for the same reason — a gizmo,
    /// the Inspector and a test all want these points and must not each derive them.
    ///
    /// The camera itself is [`wanted`](Self::wanted); this is the chain that leads to it.
    pub fn rig_positions(&self, target: Vec3, up: Vec3, reference: Vec3) -> (Vec3, Vec3, Vec3) {
        // 🔴 Only the body that shows the fields reads them. Authored under one body and left behind
        // by a switch to another, they would otherwise still move the camera from a hidden row.
        if self.follow != FOLLOW_SHOULDER {
            return (target, target, target);
        }
        let up = normalised_up(up);
        let back = self.back(up, reference);
        let shoulder = target + self.shouldered(back, up);
        (
            target,
            shoulder,
            shoulder + self.risen(back, up) * self.vertical_arm_length,
        )
    }

    /// The view's own up, pitched with it — Cinemachine's `targetRot * Vector3.up`, and the axis the
    /// arm rises along. Perpendicular to `along`, so the two segments never fold onto each other.
    fn risen(&self, back: Vec3, up: Vec3) -> Vec3 {
        let (sin_pitch, cos_pitch) = self.pitched();
        up * cos_pitch - back * sin_pitch
    }

    /// The clamped pitch, as a sine and a cosine. Clamped just shy of the poles: a fully vertical arm
    /// leaves no horizontal basis, and the camera flips.
    fn pitched(&self) -> (f32, f32) {
        self.pitch.to_radians().clamp(-1.5533, 1.5533).sin_cos()
    }

    /// The yaw's direction on `up`'s horizon — the plane perpendicular to it — pointing away from
    /// what the camera looks at.
    fn back(&self, up: Vec3, reference: Vec3) -> Vec3 {
        glam::Quat::from_axis_angle(up, self.yaw.to_radians()) * flattened(reference, up)
    }

    /// The unit direction from the pivot out to the camera: `back`, raised off the horizon by pitch.
    /// Independent of `distance`, so an aim along it survives a zero-length arm.
    fn along(&self, back: Vec3, up: Vec3) -> Vec3 {
        let (sin_pitch, cos_pitch) = self.pitched();
        back * cos_pitch + up * sin_pitch
    }

    /// The pivot's offset from the target, read in the arm's basis. Built off `back` alone, so
    /// pitching the view does not roll the shoulder.
    fn shouldered(&self, back: Vec3, up: Vec3) -> Vec3 {
        let forward = -back;
        // `forward × up`, the same hand `look_at` builds its basis with.
        let right = forward.cross(up);
        // `Lerp(-x, x, side)`, written as the multiplier it is.
        let beside = self.shoulder_offset.x * (self.camera_side.clamp(0.0, 1.0) * 2.0 - 1.0);
        right * beside + up * self.shoulder_offset.y + forward * self.shoulder_offset.z
    }

    /// Eases `current` towards `desired`, per axis, leaving a hundredth of the gap after
    /// `damping_value` seconds.
    ///
    /// 🔴 Exponential, not a tween with a duration. A tween that restarts whenever its goal moves
    /// spends every frame at the fastest part of its curve, and steps whenever the target starts or
    /// stops — the "saltos raros" (#1336). This has no state to restart: the same gap, the same
    /// fraction, whatever the goal is doing.
    pub fn damped(&self, current: Vec3, desired: Vec3, dt: f32) -> Vec3 {
        Vec3::new(
            eased(current.x, desired.x, self.damping.x, dt),
            eased(current.y, desired.y, self.damping.y, dt),
            eased(current.z, desired.z, self.damping.z, dt),
        )
    }

    /// Tweens an orientation the same way, along the shorter arc.
    pub fn damped_rotation(&self, current: glam::Quat, desired: glam::Quat, dt: f32) -> glam::Quat {
        // Components are not axes, and the long way round is 359° of roll.
        let desired = match current.dot(desired) < 0.0 {
            true => -desired,
            false => desired,
        };
        current
            .slerp(desired, settled(dt, self.rotation_damping))
            .normalize()
    }
}

/// A usable up: world up when handed zero, as `gravity_at` gives where no field reaches, instead of
/// `NaN` downstream.
fn normalised_up(up: Vec3) -> Vec3 {
    if up.length_squared() < 1e-12 {
        Vec3::Y
    } else {
        up.normalize()
    }
}

/// A first yaw origin for a vcam with none. First frame only: by the hairy ball theorem no
/// reference derived from `up` alone is continuous, so [`transported`] carries it after.
///
/// Public for the same reason [`up_for`](crate::plugin::up_for) is: what a gizmo draws before the
/// rig has run is the answer the rig itself would start from (#1387).
pub fn seed_reference(up: Vec3) -> Vec3 {
    let axis = if up.dot(Vec3::Z).abs() > 0.999 {
        Vec3::X
    } else {
        Vec3::Z
    };
    (axis - up * axis.dot(up)).normalize()
}

/// Carries a yaw origin to a new up along the shortest arc, so it has no pole to cross — what
/// `seed_reference` cannot do.
pub fn transported(reference: Vec3, from_up: Vec3, to_up: Vec3) -> Vec3 {
    let (from_up, to_up) = (normalised_up(from_up), normalised_up(to_up));
    let axis = from_up.cross(to_up);
    // Unchanged, or exactly reversed — which has no shortest arc, so the reference is re-flattened
    // rather than spun half a turn.
    let turn = match axis.length_squared() > 1e-12 {
        true => glam::Quat::from_axis_angle(axis.normalize(), from_up.angle_between(to_up)),
        false => glam::Quat::IDENTITY,
    };
    flattened(turn * reference, to_up)
}

/// A unit vector on `up`'s horizon plane, nearest `reference`: transport drifts off the plane in
/// `f32`, and a tilted reference builds a basis that is not square.
pub(crate) fn flattened(reference: Vec3, up: Vec3) -> Vec3 {
    let flat = reference - up * reference.dot(up);
    // Parallel to `up`, so it names no direction on the horizon at all.
    // Only reachable from a caller that handed in a reference for some
    // other up entirely.
    match flat.length_squared() > 1e-12 {
        true => flat.normalize(),
        false => seed_reference(up),
    }
}

/// A rotation looking from `eye` at `target` with `up` up; identity when they coincide, instead of
/// a `NaN` pose.
pub(crate) fn look_at(eye: Vec3, target: Vec3, up: Vec3, reference: Vec3) -> glam::Quat {
    let forward = target - eye;
    if forward.length_squared() < 1e-12 {
        return glam::Quat::IDENTITY;
    }
    let forward = forward.normalize();
    // Looking along `up` leaves no right vector; the carried reference is already on the horizon
    // plane and stands in.
    let up = match forward.dot(up).abs() > 0.999 {
        true => flattened(reference, up),
        false => up,
    };
    // `forward × up`, not `up × forward`: the other order is a reflection, and `Quat::from_mat3`
    // returns a finite, plausible, wrong quaternion for it.
    let right = forward.cross(up).normalize();
    let up = right.cross(forward);
    // The camera looks down -Z, so the basis' forward column is negated.
    glam::Quat::from_mat3(&glam::Mat3::from_cols(right, up, -forward))
}

/// The Body stage: where the camera stands.
///
/// 🔴 A framed rig is **not** damped twice. The frame's ease IS the body's smoothing — two eases in
/// series on one position is what made every earlier version of this rig fight itself (#1329).
pub fn body_stage(step: &mut crate::rig::RigStep) {
    // 🔴 The composer eases the offset itself, so it is the whole of the body: running the vcam's
    // damping over it is two eases in series on one quantity, which is #1329.
    if step.vcam.follow == FOLLOW_POSITION_COMPOSER {
        crate::position_composer::body(step);
        return;
    }
    let wanted = step.vcam.wanted(
        step.frame.target,
        step.frame.position,
        step.up,
        step.reference,
    );
    step.frame
        .place(step.vcam.damped(step.frame.previous, wanted, step.dt));
}

/// The Aim stage: where the camera looks. One owner — in a third-person rig it is the player's, and
/// the only easing on it is this one (#1329).
///
/// 🔴 Aimed from where the **body** left the camera, not from where a frame or a wall left it. A
/// frame says "hold the target off centre" by moving the camera while the aim stays the body's, so
/// aiming from the moved position would cancel the frame it just asked for. A wall pulls in along
/// the same line, so for it the two are the same direction anyway.
pub fn aim_stage(step: &mut crate::rig::RigStep) {
    // 🔴 The composer eases the residual angle itself, so it is the whole of the aim: running the
    // vcam's rotation damping over it is two eases in series on one quantity, which is #1329 on the
    // other axis.
    if step.vcam.look_at == LOOK_AT_COMPOSED {
        crate::framing::composed(step);
        return;
    }
    let aimed = step.vcam.aimed(
        step.frame.position,
        step.frame.target,
        step.target.rotation,
        step.frame.rotation,
        step.up,
        step.reference,
    );
    step.frame.rotation = step
        .vcam
        .damped_rotation(step.frame.rotation, aimed, step.dt);
}

#[cfg(test)]
mod tests;

/// Moves a vcam that authored a shoulder on the orbital body onto the one that reads it, once.
///
/// 🔴 The shoulder lived on `Orbital Follow` until #1380 split the two, the way Cinemachine has
/// always had them. Left alone, the fields would stop showing and stop being read on the same load —
/// an offset tuned for an hour, gone with nothing said.
pub fn migrate_shoulder_body(resources: &mut kooch_core::resource::Resources) {
    let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<VirtualCamera>() else {
        return;
    };
    for (&entity, vcam) in storage.iter_mut() {
        let authored = vcam.shoulder_offset != Vec3::ZERO || vcam.vertical_arm_length != 0.0;
        if vcam.follow != FOLLOW_ORBITAL || !authored {
            continue;
        }
        vcam.follow = FOLLOW_SHOULDER;
        tracing::info!(
            target: "kooch_camera",
            entity = entity.index(),
            "a vcam with a shoulder moved from Orbital Follow to Third Person Follow",
        );
    }
}

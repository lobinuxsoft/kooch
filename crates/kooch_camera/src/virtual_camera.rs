//! [`VirtualCamera`] — camera behaviour as data a designer authors, with modes and names from
//! phantom-camera (MIT, #671).

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{FieldChoice, FieldCondition};
use kooch_ecs::tween::Chase;

/// No follow logic; the pose is whatever else wrote it.
pub const FOLLOW_NONE: u32 = 0;
/// Sits exactly on the target.
pub const FOLLOW_GLUED: u32 = 1;
/// The target's position plus a fixed offset.
pub const FOLLOW_SIMPLE: u32 = 2;
/// A spring arm on the target, rotatable around it. Third person.
pub const FOLLOW_THIRD_PERSON: u32 = 3;

/// No rotation logic.
pub const LOOK_AT_NONE: u32 = 0;
/// Copies the target's rotation.
pub const LOOK_AT_MIMIC: u32 = 1;
/// Points straight at the target.
pub const LOOK_AT_SIMPLE: u32 = 2;

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
        label: "Glued",
        value: FOLLOW_GLUED as i64,
    },
    FieldChoice {
        label: "Simple (offset)",
        value: FOLLOW_SIMPLE as i64,
    },
    FieldChoice {
        label: "Third person (spring arm)",
        value: FOLLOW_THIRD_PERSON as i64,
    },
];

/// Labels for the `look_at` dropdown.
pub static LOOK_AT_CHOICES: &[FieldChoice] = &[
    FieldChoice {
        label: "None",
        value: LOOK_AT_NONE as i64,
    },
    FieldChoice {
        label: "Mimic target rotation",
        value: LOOK_AT_MIMIC as i64,
    },
    FieldChoice {
        label: "Look at target",
        value: LOOK_AT_SIMPLE as i64,
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

/// The spring arm's parameters.
pub static THIRD_PERSON_WHEN: FieldCondition = FieldCondition {
    field: "follow",
    values: &[FOLLOW_THIRD_PERSON as i64],
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
    #[reflect(shown_when = THIRD_PERSON_WHEN)]
    pub distance: f32,
    /// Rotation around the target's up axis, in degrees.
    #[reflect(shown_when = THIRD_PERSON_WHEN)]
    pub yaw: f32,
    /// Rotation above the horizon, in degrees. Positive looks down.
    #[reflect(shown_when = THIRD_PERSON_WHEN)]
    pub pitch: f32,
    /// Where the camera looks. One of the `LOOK_AT_*` constants.
    #[reflect(choices = LOOK_AT_CHOICES)]
    pub look_at: u32,
    /// Whether the camera eases towards its pose instead of snapping.
    pub damping: bool,
    /// Seconds the camera takes to reach its pose once the target stops, per world axis — exactly,
    /// a tween that restarts while the target keeps moving. Zero is rigid.
    #[reflect(shown_when = DAMPING_WHEN, alias = "damping_value, damping_time")]
    pub damping_duration: Vec3,
    /// Seconds the handover **to** this vcam lasts, exactly; zero cuts. The incoming vcam owns it
    /// because how you arrive matters, not what came before.
    #[reflect(alias = "blend_time")]
    pub blend_duration: f32,
    /// Shape of the blend, one of the `CURVE_*` constants. Shown even at zero duration:
    /// `shown_when` cannot enumerate every float above zero.
    #[reflect(choices = crate::blend::BLEND_CURVE_CHOICES)]
    pub blend_curve: u32,
    /// Which end of the blend is slow. One of the `EASE_*` constants.
    #[reflect(choices = crate::blend::BLEND_EASE_CHOICES)]
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
    #[reflect(shown_when = DAMPING_WHEN, alias = "rotation_damping_value, rotation_damping_time")]
    pub rotation_damping_duration: f32,
}

/// The damping values only matter when damping is on.
pub static DAMPING_WHEN: FieldCondition = FieldCondition {
    field: "damping",
    values: &[1],
};

impl Default for VirtualCamera {
    fn default() -> Self {
        Self {
            priority: 0,
            enabled: true,
            follow: FOLLOW_THIRD_PERSON,
            group: 0,
            offset: Vec3::new(0.0, 2.0, 6.0),
            distance: 6.0,
            yaw: 0.0,
            pitch: 20.0,
            look_at: LOOK_AT_SIMPLE,
            damping: true,
            damping_duration: Vec3::splat(0.5),
            up_mode: UP_WORLD,
            // Long enough to read as a transition, short enough not to
            // feel like the game took the camera away.
            blend_duration: 0.5,
            blend_curve: crate::blend::CURVE_SINE,
            blend_ease: crate::blend::EASE_IN_OUT,
            inactive_update: INACTIVE_NEVER,
            rotation_damping_duration: 0.5,
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

    /// The pose this vcam wants before damping, as a pure function of the target's pose, its
    /// current pose and a resolved `up`.
    /// `current_rot` lets `LookAt::None` leave the rotation alone.
    pub fn desired(
        &self,
        target_pos: Vec3,
        target_rot: glam::Quat,
        current_pos: Vec3,
        current_rot: glam::Quat,
        up: Vec3,
    ) -> (Vec3, glam::Quat) {
        let up = normalised_up(up);
        self.desired_with(
            target_pos,
            target_rot,
            current_pos,
            current_rot,
            up,
            seed_reference(up),
        )
    }

    /// The same, given the yaw origin to measure from — carried per vcam by the Host, since it
    /// cannot come from `up` alone (see `seed_reference`).
    pub fn desired_with(
        &self,
        target_pos: Vec3,
        target_rot: glam::Quat,
        current_pos: Vec3,
        current_rot: glam::Quat,
        up: Vec3,
        reference: Vec3,
    ) -> (Vec3, glam::Quat) {
        let up = normalised_up(up);
        let position = match self.follow {
            FOLLOW_GLUED => target_pos,
            FOLLOW_SIMPLE => target_pos + self.offset,
            FOLLOW_THIRD_PERSON => target_pos + self.arm(up, reference),
            // `None` keeps the camera wherever it is, which is what lets
            // a vcam do look-at only — a turret that tracks without moving.
            _ => current_pos,
        };

        let rotation = match self.look_at {
            LOOK_AT_MIMIC => target_rot,
            LOOK_AT_SIMPLE => look_at(position, target_pos, up, reference),
            _ => current_rot,
        };

        (position, rotation)
    }

    /// The spring arm's offset: around `up` by yaw, raised off the horizon by pitch — the horizon
    /// being the plane perpendicular to `up`. Fixed length; shortening against obstacles needs
    /// #562.
    fn arm(&self, up: Vec3, reference: Vec3) -> Vec3 {
        // Clamped just shy of the poles: a fully vertical arm leaves no
        // horizontal basis, and the camera flips.
        let pitch = self.pitch.to_radians().clamp(-1.5533, 1.5533);
        let (sin_pitch, cos_pitch) = pitch.sin_cos();

        let forward = flattened(reference, up);
        let swung = glam::Quat::from_axis_angle(up, self.yaw.to_radians()) * forward;

        (swung * cos_pitch + up * sin_pitch) * self.distance.max(0.0)
    }

    /// Tweens `current` towards `desired`, per axis, arriving `damping_duration` after it stops moving.
    pub fn damped(&self, damping: &mut Damping, current: Vec3, desired: Vec3, dt: f32) -> Vec3 {
        if !self.damping {
            damping.position = [
                Chase::at(desired.x),
                Chase::at(desired.y),
                Chase::at(desired.z),
            ];
            return desired;
        }
        let [x, y, z] = &mut damping.position;
        let time = self.damping_duration;
        Vec3::new(
            x.step(current.x, desired.x, dt, time.x),
            y.step(current.y, desired.y, dt, time.y),
            z.step(current.z, desired.z, dt, time.z),
        )
    }

    /// Tweens an orientation the same way, along the shorter arc.
    pub fn damped_rotation(
        &self,
        damping: &mut Damping,
        current: glam::Quat,
        desired: glam::Quat,
        dt: f32,
    ) -> glam::Quat {
        let time = match self.damping {
            true => self.rotation_damping_duration,
            false => 0.0,
        };
        damping.rotation.step(current, desired, dt, time)
    }
}

/// A vcam's damping in flight: one tween per world axis and one for the orientation. Carried by the
/// Host between steps, since a tween is a clock.
#[derive(Debug, Clone, Copy)]
pub struct Damping {
    position: [Chase<f32>; 3],
    rotation: Chase<glam::Quat>,
}

impl Damping {
    /// At rest on a pose.
    pub fn at(position: Vec3, rotation: glam::Quat) -> Self {
        Self {
            position: [
                Chase::at(position.x),
                Chase::at(position.y),
                Chase::at(position.z),
            ],
            rotation: Chase::at(rotation),
        }
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
pub(crate) fn seed_reference(up: Vec3) -> Vec3 {
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
fn flattened(reference: Vec3, up: Vec3) -> Vec3 {
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

#[cfg(test)]
mod tests;

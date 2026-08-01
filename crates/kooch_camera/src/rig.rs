//! [`CameraRig`] — camera behaviour as data a designer authors.
//!
//! Without this, every game writes its own follow code and every game
//! gets a different, worse version of the same thing. The modes and the
//! names come from [phantom-camera](https://github.com/ramokz/phantom-camera)
//! (MIT), which is the open equivalent of Unity's Cinemachine; porting its
//! vocabulary rather than inventing one means the concepts are already
//! proven and already familiar (#671).

use glam::Vec3;
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;
use kooch_ecs::reflect::{EntityRef, FieldChoice, FieldCondition};

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

/// An inactive rig computes nothing.
pub const INACTIVE_NEVER: u32 = 0;
/// An inactive rig keeps updating.
pub const INACTIVE_ALWAYS: u32 = 1;

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

/// How much a target has to move before the camera bothers, per axis, in
/// world units. Below this the pose is left alone.
///
/// Not a knob: a floor. Damping is asymptotic, so a camera chasing a
/// target it has effectively caught would keep writing microscopically
/// different poses forever, and every write is a dirty transform to
/// propagate and mirror.
pub const SETTLE_EPSILON: f32 = 1e-4;

/// Camera behaviour, on a **virtual camera** of its own.
///
/// # Why a virtual camera rather than the camera itself
///
/// This component does not go on the entity that renders. It goes on an
/// entity that only holds a rig and a [`Transform`], and a Host system
/// elects one of them and copies its pose onto the camera that does
/// render. That is phantom-camera's model, and Cinemachine's before it.
///
/// An earlier version put the rig straight on the camera, reasoning that
/// with no blending a second election would be dead weight. It bought
/// nothing and cost two things:
///
/// - **Three framings needed three cameras.** A vcam is not a camera, so
///   a scene can hold as many as it wants and still render through one.
/// - **The split would have to happen later**, as a migration of scenes
///   people had already saved — and a scene stores components by type
///   name, so that migration is the expensive kind.
///
/// The two priorities answer different questions and neither replaces
/// the other:
///
/// | Priority | Elects |
/// |---|---|
/// | [`PerspectiveCamera::priority`](kooch_ecs::perspective_camera::PerspectiveCamera) | which camera **renders** — the editor's own sits at 1000 |
/// | [`CameraRig::priority`] | which rig **drives** that camera |
///
/// Blending (#671 phase 3) is then interpolating between two vcam poses,
/// which needs no further change to this shape.
///
/// # Default
///
/// A freshly added rig does nothing: `follow` and `look_at` are `None`
/// and `target` is empty. A component that started moving the camera the
/// instant it was added would fight whoever placed the camera.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct CameraRig {
    /// Which rig wins when several are live. Highest drives the camera.
    ///
    /// Ties break on the lower entity index — arbitrary, but *stable*.
    /// Upstream lets the last one evaluated win, which is fine in a
    /// scene tree with a defined order and is not fine here: component
    /// storage has no iteration order to rely on, so a tie would hand
    /// the camera to a different rig from frame to frame.
    pub priority: i32,
    /// A rig that is switched off is not a candidate at all.
    ///
    /// phantom-camera uses node visibility for this. A vcam draws
    /// nothing, so it borrows a field it has no other use for; here it
    /// is worth its own name.
    pub enabled: bool,
    /// Where the camera sits. One of the `FOLLOW_*` constants.
    #[reflect(choices = FOLLOW_MODE_CHOICES)]
    pub follow: u32,
    /// What to follow and look at. Inert until this is set.
    pub target: Option<EntityRef>,
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
    /// Seconds to close most of the gap, per world axis.
    ///
    /// A time constant rather than a blend factor, so the feel does not
    /// change with the frame rate — the mistake this kind of code is
    /// usually shipped with. Zero on an axis is rigid on that axis.
    #[reflect(shown_when = DAMPING_WHEN)]
    pub damping_value: Vec3,
    /// Whether a rig on a camera that is not rendering still computes.
    /// One of the `INACTIVE_*` constants.
    #[reflect(choices = INACTIVE_UPDATE_CHOICES)]
    pub inactive_update: u32,
}

/// The damping values only matter when damping is on.
pub static DAMPING_WHEN: FieldCondition = FieldCondition {
    field: "damping",
    values: &[1],
};

impl Default for CameraRig {
    fn default() -> Self {
        Self {
            priority: 0,
            enabled: true,
            follow: FOLLOW_NONE,
            target: None,
            offset: Vec3::new(0.0, 2.0, 6.0),
            distance: 6.0,
            yaw: 0.0,
            pitch: 20.0,
            look_at: LOOK_AT_NONE,
            damping: true,
            damping_value: Vec3::splat(0.15),
            inactive_update: INACTIVE_NEVER,
        }
    }
}

impl Component for CameraRig {}

impl CameraRig {
    /// Whether this rig has anything to do.
    pub fn is_inert(&self) -> bool {
        !self.enabled
            || self.target.is_none()
            || (self.follow == FOLLOW_NONE && self.look_at == LOOK_AT_NONE)
    }

    /// The pose this rig wants, given where its target is.
    ///
    /// Pure, so the geometry is testable without a world: `target_pos`
    /// and `target_rot` are the target's, `current_*` is where the vcam
    /// is now, and the return is where it would like to be before
    /// damping.
    ///
    /// `current_rot` exists because `LookAt::None` has to mean *leave the
    /// rotation alone*, and a function that never saw the current
    /// rotation cannot return it. It used to return the **target's**
    /// rotation instead, so a follow-only rig silently mimicked whatever
    /// it was following.
    pub fn desired(
        &self,
        target_pos: Vec3,
        target_rot: glam::Quat,
        current_pos: Vec3,
        current_rot: glam::Quat,
    ) -> (Vec3, glam::Quat) {
        let position = match self.follow {
            FOLLOW_GLUED => target_pos,
            FOLLOW_SIMPLE => target_pos + self.offset,
            FOLLOW_THIRD_PERSON => target_pos + self.arm(),
            // `None` keeps the camera wherever it is, which is what lets
            // a rig do look-at only — a turret that tracks without moving.
            _ => current_pos,
        };

        let rotation = match self.look_at {
            LOOK_AT_MIMIC => target_rot,
            LOOK_AT_SIMPLE => look_at(position, target_pos),
            _ => current_rot,
        };

        (position, rotation)
    }

    /// The spring arm's offset from the target: back and up, by yaw and
    /// pitch.
    ///
    /// A fixed length. Shortening it against obstacles needs scene
    /// queries (#562) and is deliberately not here.
    fn arm(&self) -> Vec3 {
        let yaw = self.yaw.to_radians();
        // Clamped just shy of the poles: straight down makes the look-at
        // basis degenerate, and the camera flips.
        let pitch = self.pitch.to_radians().clamp(-1.5533, 1.5533);
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let (sin_pitch, cos_pitch) = pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, cos_yaw * cos_pitch) * self.distance.max(0.0)
    }

    /// Eases `current` towards `desired` over `dt`.
    ///
    /// Exponential, per axis, on a time constant: `1 - e^(-dt/tau)`. The
    /// naive `lerp(current, desired, k)` moves further per second the
    /// faster the frame rate, so the same numbers feel different on two
    /// machines. This does not.
    pub fn damped(&self, current: Vec3, desired: Vec3, dt: f32) -> Vec3 {
        if !self.damping {
            return desired;
        }
        Vec3::new(
            ease(current.x, desired.x, self.damping_value.x, dt),
            ease(current.y, desired.y, self.damping_value.y, dt),
            ease(current.z, desired.z, self.damping_value.z, dt),
        )
    }
}

/// One axis of exponential easing. `tau <= 0` is rigid.
fn ease(current: f32, desired: f32, tau: f32, dt: f32) -> f32 {
    if tau <= 0.0 || dt <= 0.0 {
        return desired;
    }
    let alpha = 1.0 - (-dt / tau).exp();
    current + (desired - current) * alpha
}

/// A rotation looking from `eye` towards `target`, up being +Y.
///
/// Falls back to identity when the two coincide: a camera glued to what
/// it looks at has no direction to face, and normalising a zero vector
/// would hand the renderer a `NaN` pose.
fn look_at(eye: Vec3, target: Vec3) -> glam::Quat {
    let forward = target - eye;
    if forward.length_squared() < 1e-12 {
        return glam::Quat::IDENTITY;
    }
    let forward = forward.normalize();
    // Degenerate when looking straight up or down; nudge the reference up
    // axis rather than producing a zero-length right vector.
    let up = if forward.y.abs() > 0.999 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    // `forward × up`, not `up × forward`. The other order yields a basis
    // with determinant -1 — a reflection, not a rotation — and
    // `Quat::from_mat3` assumes it was handed a rotation, so it returns a
    // quaternion that is finite, plausible, and wrong. Every existing
    // test asserted `is_finite()`, which a reflection passes.
    let right = forward.cross(up).normalize();
    let up = right.cross(forward);
    // The camera looks down -Z, so the basis' forward column is negated.
    glam::Quat::from_mat3(&glam::Mat3::from_cols(right, up, -forward))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(follow: u32) -> CameraRig {
        CameraRig {
            follow,
            target: Some(EntityRef::Live(kooch_ecs::entity::Entity::new(0, 0))),
            damping: false,
            ..Default::default()
        }
    }

    #[test]
    fn a_rig_without_a_target_is_inert() {
        let mut r = CameraRig {
            follow: FOLLOW_SIMPLE,
            ..Default::default()
        };
        assert!(r.is_inert(), "no target means nothing to follow");
        r.target = Some(EntityRef::Live(kooch_ecs::entity::Entity::new(0, 0)));
        assert!(!r.is_inert());
    }

    /// A freshly added component must not move the camera, or dropping a
    /// rig on a placed camera would teleport it.
    #[test]
    fn the_default_does_nothing() {
        assert!(CameraRig::default().is_inert());
    }

    #[test]
    fn simple_follow_is_the_target_plus_the_offset() {
        let mut r = rig(FOLLOW_SIMPLE);
        r.offset = Vec3::new(0.0, 3.0, 10.0);
        let (pos, _) = r.desired(
            Vec3::new(5.0, 0.0, 0.0),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
        );
        assert_eq!(pos, Vec3::new(5.0, 3.0, 10.0));
    }

    #[test]
    fn the_spring_arm_keeps_its_length_at_every_yaw() {
        let mut r = rig(FOLLOW_THIRD_PERSON);
        r.distance = 7.0;
        for yaw in [0.0, 37.0, 90.0, 180.0, -145.0] {
            r.yaw = yaw;
            let (pos, _) = r.desired(
                Vec3::ZERO,
                glam::Quat::IDENTITY,
                Vec3::ZERO,
                glam::Quat::IDENTITY,
            );
            assert!(
                (pos.length() - 7.0).abs() < 1e-4,
                "yaw {yaw} gave length {}",
                pos.length(),
            );
        }
    }

    /// Straight down is where a look-at basis degenerates and the image
    /// rolls over. The clamp is what stops it.
    #[test]
    fn pitch_is_clamped_short_of_the_pole() {
        let mut r = rig(FOLLOW_THIRD_PERSON);
        r.pitch = 90.0;
        let (pos, _) = r.desired(
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
        );
        assert!(
            Vec3::new(pos.x, 0.0, pos.z).length() > 1e-3,
            "a fully vertical arm leaves no horizontal basis: {pos:?}",
        );
    }

    /// Follow `None` with a look-at is a turret: it tracks and stays put.
    #[test]
    fn follow_none_leaves_the_position_alone() {
        let mut r = rig(FOLLOW_NONE);
        r.look_at = LOOK_AT_SIMPLE;
        let here = Vec3::new(1.0, 2.0, 3.0);
        let (pos, _) = r.desired(
            Vec3::new(9.0, 0.0, 0.0),
            glam::Quat::IDENTITY,
            here,
            glam::Quat::IDENTITY,
        );
        assert_eq!(pos, here);
        assert!(!r.is_inert(), "look-at alone is still work to do");
    }

    /// The property that matters: the same elapsed time gives the same
    /// result whatever the step size. A `lerp` with a constant factor
    /// fails this, and that is the bug this replaces.
    #[test]
    fn damping_is_frame_rate_independent() {
        let mut r = CameraRig {
            damping: true,
            damping_value: Vec3::splat(0.25),
            ..Default::default()
        };
        r.follow = FOLLOW_SIMPLE;

        let desired = Vec3::new(10.0, 0.0, 0.0);
        let coarse = r.damped(Vec3::ZERO, desired, 1.0 / 30.0);

        let mut fine = Vec3::ZERO;
        for _ in 0..2 {
            fine = r.damped(fine, desired, 1.0 / 60.0);
        }

        assert!(
            (coarse.x - fine.x).abs() < 1e-3,
            "one 30 Hz step gave {} and two 60 Hz steps gave {}",
            coarse.x,
            fine.x,
        );
    }

    #[test]
    fn damping_off_snaps_exactly() {
        let r = CameraRig {
            damping: false,
            ..Default::default()
        };
        let desired = Vec3::new(3.0, 4.0, 5.0);
        assert_eq!(r.damped(Vec3::ZERO, desired, 1.0 / 60.0), desired);
    }

    /// Zero on an axis is rigid on that axis, while its neighbours ease.
    #[test]
    fn a_zero_time_constant_is_rigid_on_that_axis_only() {
        let r = CameraRig {
            damping: true,
            damping_value: Vec3::new(0.0, 0.2, 0.2),
            ..Default::default()
        };
        let got = r.damped(Vec3::ZERO, Vec3::splat(10.0), 1.0 / 60.0);
        assert_eq!(got.x, 10.0, "x should be rigid");
        assert!(got.y < 10.0 && got.y > 0.0, "y should be easing: {}", got.y);
    }

    /// The test this file did not have, and the reason a mirrored basis
    /// shipped: every rotation assertion here checked `is_finite()`, and
    /// a reflection is perfectly finite. What a look-at owes you is that
    /// the camera's forward axis actually points at the thing.
    #[test]
    fn look_at_points_the_camera_at_the_target() {
        let mut r = rig(FOLLOW_NONE);
        r.look_at = LOOK_AT_SIMPLE;

        for (eye, target) in [
            (Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0)),
            (Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0)),
            (Vec3::new(3.0, 4.0, 5.0), Vec3::new(-2.0, 1.0, 8.0)),
            (Vec3::new(-7.0, 2.0, 0.0), Vec3::ZERO),
        ] {
            let (_, rot) = r.desired(target, glam::Quat::IDENTITY, eye, glam::Quat::IDENTITY);
            // A camera looks down its own -Z.
            let forward = rot * -Vec3::Z;
            let want = (target - eye).normalize();
            assert!(
                forward.dot(want) > 0.9999,
                "from {eye:?} to {target:?}: facing {forward:?}, wanted {want:?}",
            );
        }
    }

    /// A mirrored basis also flips the horizon. Checking `up` catches a
    /// roll of 180° that a forward-only assertion would let through.
    #[test]
    fn look_at_keeps_the_horizon_upright() {
        let mut r = rig(FOLLOW_NONE);
        r.look_at = LOOK_AT_SIMPLE;
        let (_, rot) = r.desired(
            Vec3::new(0.0, 0.0, -10.0),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
        );
        assert!(
            (rot * Vec3::Y).dot(Vec3::Y) > 0.0,
            "the camera is upside down: up is {:?}",
            rot * Vec3::Y,
        );
    }

    /// Looking straight at something along -Z is the identity rotation.
    /// It was a reflection, which `is_finite()` happily accepted.
    #[test]
    fn the_canonical_look_at_is_the_identity() {
        let mut r = rig(FOLLOW_NONE);
        r.look_at = LOOK_AT_SIMPLE;
        let (_, rot) = r.desired(
            Vec3::new(0.0, 0.0, -1.0),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
        );
        assert!(
            rot.abs_diff_eq(glam::Quat::IDENTITY, 1e-5),
            "expected identity, got {rot:?}",
        );
    }

    /// `None` means leave it alone. It used to return the *target's*
    /// rotation, so a follow-only rig silently mimicked its target —
    /// which is what `Mimic` is for.
    #[test]
    fn look_at_none_keeps_the_cameras_own_rotation() {
        let mut r = rig(FOLLOW_SIMPLE);
        r.look_at = LOOK_AT_NONE;
        let mine = glam::Quat::from_rotation_y(0.7);
        let targets = glam::Quat::from_rotation_x(1.3);
        let (_, rot) = r.desired(Vec3::new(4.0, 0.0, 0.0), targets, Vec3::ZERO, mine);
        assert!(
            rot.abs_diff_eq(mine, 1e-6),
            "expected {mine:?}, got {rot:?}"
        );
    }

    #[test]
    fn a_disabled_rig_is_inert() {
        let mut r = rig(FOLLOW_SIMPLE);
        assert!(!r.is_inert());
        r.enabled = false;
        assert!(r.is_inert(), "a switched-off rig must not be a candidate");
    }

    #[test]
    fn looking_at_where_you_already_are_is_not_a_nan() {
        let mut r = rig(FOLLOW_GLUED);
        r.look_at = LOOK_AT_SIMPLE;
        let (_, rot) = r.desired(
            Vec3::splat(2.0),
            glam::Quat::IDENTITY,
            Vec3::ZERO,
            glam::Quat::IDENTITY,
        );
        assert!(rot.is_finite(), "a degenerate look-at produced {rot:?}");
    }

    /// A camera looking dead down is the other degenerate case, and it
    /// has to stay finite rather than roll.
    #[test]
    fn looking_straight_down_stays_finite() {
        let mut r = rig(FOLLOW_NONE);
        r.look_at = LOOK_AT_SIMPLE;
        let (_, rot) = r.desired(
            Vec3::ZERO,
            glam::Quat::IDENTITY,
            Vec3::new(0.0, 10.0, 0.0),
            glam::Quat::IDENTITY,
        );
        assert!(rot.is_finite(), "straight down produced {rot:?}");
    }
}

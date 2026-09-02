//! [`VirtualCamera`] — camera behaviour as data a designer authors.
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
use kooch_ecs::reflect::{FieldChoice, FieldCondition};

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
/// entity that only holds a framing and a [`Transform`], and a Host system
/// elects one of them and copies its pose onto the camera that does
/// render. That is phantom-camera's model, and Cinemachine's before it.
///
/// An earlier version put this component straight on the camera, reasoning that
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
/// | [`VirtualCamera::priority`] | which vcam **drives** that camera |
///
/// Blending (#671 phase 3) is then interpolating between two vcam poses,
/// which needs no further change to this shape.
///
/// # Default
///
/// A third-person vcam looking at its target — everything set except
/// the target, which is empty, so a fresh one still moves nothing.
/// [`VirtualCamera::is_inert`] is what guarantees that, not the modes.
///
/// The modes used to default to `None` as well, on the grounds that a
/// component moving the camera the instant it was added would fight
/// whoever placed it. That was true when this lived *on* the camera.
/// A vcam is its own entity now, with nobody to fight, and three fields
/// to set before anything happens is three chances to give up. Upstream
/// starts at `NONE` too, but it is a node added to a scene tree, where
/// nothing is expected to happen on its own; a menu entry called
/// "Virtual Camera" promises a camera, not an empty component.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Camera")]
pub struct VirtualCamera {
    /// Which vcam wins when several are live. Highest drives the camera.
    ///
    /// Ties break on the lower entity index — arbitrary, but *stable*.
    /// Upstream lets the last one evaluated win, which is fine in a
    /// scene tree with a defined order and is not fine here: component
    /// storage has no iteration order to rely on, so a tie would hand
    /// the camera to a different vcam from frame to frame.
    pub priority: i32,
    /// A vcam that is switched off is not a candidate at all.
    ///
    /// phantom-camera uses node visibility for this. A vcam draws
    /// nothing, so it borrows a field it has no other use for; here it
    /// is worth its own name.
    pub enabled: bool,
    /// Where the camera sits. One of the `FOLLOW_*` constants.
    #[reflect(choices = FOLLOW_MODE_CHOICES)]
    pub follow: u32,
    /// Which [`CameraTarget`](crate::CameraTarget) group this framing
    /// follows.
    ///
    /// Entities carrying that tag are the subject; several of them are a
    /// group and the camera follows their weighted centre. Inert while
    /// nothing carries it.
    ///
    /// # Why a group number and not a reference to the entity
    ///
    /// A reference has to survive being written to a file, and the one
    /// this replaced did not: it named an identity the loader reassigned
    /// every session, so a camera authored to follow the player followed
    /// nothing after a reload (#712). A query has no identity to lose.
    ///
    /// And it is not only a repair. Several entities carrying the tag
    /// *are* a group, so framing one subject and framing four are the
    /// same code — where a reference needs a second mechanism for the
    /// second case, as Cinemachine and phantom-camera both do.
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
    /// Seconds to close most of the gap, per world axis.
    ///
    /// A time constant rather than a blend factor, so the feel does not
    /// change with the frame rate — the mistake this kind of code is
    /// usually shipped with. Zero on an axis is rigid on that axis.
    #[reflect(shown_when = DAMPING_WHEN)]
    pub damping_value: Vec3,
    /// Seconds to take when the camera hands over **to** this vcam.
    ///
    /// # Why the incoming vcam owns the transition
    ///
    /// Because the interesting question is "how do we arrive here", not
    /// "how do we leave there": a cutscene framing wants to be eased
    /// into, a hit-reaction framing wants to be cut to, and neither
    /// depends on which framing was on screen before. Upstream draws the
    /// same line — its tween resource is documented as defining "how
    /// transitioning *to* that instance will look".
    ///
    /// Zero cuts. Priority ties are settled before this is consulted, so
    /// a blend never starts against a winner that is about to change.
    pub blend_duration: f32,
    /// Shape of the blend. One of the `CURVE_*` constants.
    ///
    /// Shown even at `blend_duration == 0`, where it does nothing:
    /// `shown_when` matches a field against a list of integers, and
    /// "any duration above zero" is not something a float can enumerate.
    /// Hiding it with a condition that happens to hold would be relying
    /// on an accident.
    #[reflect(choices = crate::blend::BLEND_CURVE_CHOICES)]
    pub blend_curve: u32,
    /// Which end of the blend is slow. One of the `EASE_*` constants.
    #[reflect(choices = crate::blend::BLEND_EASE_CHOICES)]
    pub blend_ease: u32,
    /// Which way is up for this vcam. One of the `UP_*` constants.
    ///
    /// # Why the vcam owns this and not the camera
    ///
    /// A camera has no opinion about up; a *framing* does. Two vcams on
    /// the same camera can want different answers — a gameplay vcam
    /// aligned to the planet you are standing on, a cutscene vcam fixed
    /// to the world.
    ///
    /// # Why not just read the target's rotation
    ///
    /// Because a rolling body does not have one worth reading. A
    /// character controller aligns itself to gravity, so its up is the
    /// answer; a ball rolling by friction spins freely, and following
    /// its up would put the horizon on a spit. `Gravity` asks the field
    /// directly, which is the only source that is right in both cases.
    ///
    /// Without `kooch_gravity` compiled in, `Gravity` behaves as
    /// `World` — the field cannot be consulted if there is no field.
    #[reflect(choices = UP_MODE_CHOICES)]
    pub up_mode: u32,
    /// Whether a vcam on a camera that is not rendering still computes.
    /// One of the `INACTIVE_*` constants.
    #[reflect(choices = INACTIVE_UPDATE_CHOICES)]
    pub inactive_update: u32,
    /// Seconds for the camera to settle into a new orientation.
    ///
    /// Only rotation. Position damping is per axis because a follow
    /// camera is often looser horizontally than vertically; an
    /// orientation has no axes to treat differently, and slerping
    /// towards the new basis is the whole behaviour.
    ///
    /// This is what stops an up that changes — crossing between two
    /// gravity fields — from snapping the horizon over in one frame.
    /// Zero is rigid.
    #[reflect(shown_when = DAMPING_WHEN)]
    pub rotation_damping_value: f32,
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
            damping_value: Vec3::splat(0.15),
            up_mode: UP_WORLD,
            // Long enough to read as a transition, short enough not to
            // feel like the game took the camera away.
            blend_duration: 0.5,
            blend_curve: crate::blend::CURVE_SINE,
            blend_ease: crate::blend::EASE_IN_OUT,
            inactive_update: INACTIVE_NEVER,
            rotation_damping_value: 0.12,
        }
    }
}

impl Component for VirtualCamera {}

impl VirtualCamera {
    /// Whether this vcam has anything to do, as far as it can tell alone.
    ///
    /// It cannot tell whether anything carries its
    /// [`CameraTarget`](crate::CameraTarget) group — that is a query over
    /// the world, and the answer changes as entities spawn and despawn.
    /// A vcam whose group is empty is skipped where the pose is planned,
    /// not here.
    pub fn is_inert(&self) -> bool {
        !self.enabled || (self.follow == FOLLOW_NONE && self.look_at == LOOK_AT_NONE)
    }

    /// The pose this vcam wants, given where its target is.
    ///
    /// Pure, so the geometry is testable without a world: `target_pos`
    /// and `target_rot` are the target's, `current_*` is where the vcam
    /// is now, and the return is where it would like to be before
    /// damping.
    ///
    /// `current_rot` exists because `LookAt::None` has to mean *leave the
    /// rotation alone*, and a function that never saw the current
    /// rotation cannot return it. It used to return the **target's**
    /// rotation instead, so a follow-only vcam silently mimicked whatever
    /// it was following.
    /// `up` is which way this vcam considers up, already resolved from
    /// [`VirtualCamera::up_mode`] by the caller — the vcam itself cannot ask
    /// a gravity field anything.
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

    /// The same, given the yaw origin to measure from.
    ///
    /// The overload exists because that origin cannot be derived from
    /// `up` alone without a discontinuity — see [`seed_reference`]. The
    /// Host carries one per vcam and transports it, and only the first
    /// frame has nothing to carry.
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

    /// The spring arm's offset from the target: around `up` by yaw, then
    /// raised off the horizon by pitch.
    ///
    /// A fixed length. Shortening it against obstacles needs scene
    /// queries (#562) and is deliberately not here.
    ///
    /// The horizon is the plane perpendicular to `up`, which is what
    /// makes an arbitrary up work at all: with `up = +Y` this reduces
    /// exactly to the old fixed-axis formula, and with any other up the
    /// same yaw and pitch mean the same thing relative to the ground the
    /// target is standing on.
    fn arm(&self, up: Vec3, reference: Vec3) -> Vec3 {
        // Clamped just shy of the poles: a fully vertical arm leaves no
        // horizontal basis, and the camera flips.
        let pitch = self.pitch.to_radians().clamp(-1.5533, 1.5533);
        let (sin_pitch, cos_pitch) = pitch.sin_cos();

        let forward = flattened(reference, up);
        let swung = glam::Quat::from_axis_angle(up, self.yaw.to_radians()) * forward;

        (swung * cos_pitch + up * sin_pitch) * self.distance.max(0.0)
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

    /// Eases an orientation towards `desired`, on the same time constant
    /// idea as [`VirtualCamera::damped`].
    ///
    /// `slerp` rather than easing each component: quaternion components
    /// are not axes and interpolating them separately does not produce a
    /// rotation on the way. The shorter arc is picked explicitly, or a
    /// basis flip would take the long way round — 359° of horizon roll
    /// instead of 1°.
    pub fn damped_rotation(&self, current: glam::Quat, desired: glam::Quat, dt: f32) -> glam::Quat {
        let tau = self.rotation_damping_value;
        if !self.damping || tau <= 0.0 || dt <= 0.0 {
            return desired;
        }
        let desired = if current.dot(desired) < 0.0 {
            -desired
        } else {
            desired
        };
        let alpha = 1.0 - (-dt / tau).exp();
        current.slerp(desired, alpha).normalize()
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

/// A usable up, whatever was handed in.
///
/// A zero vector is what you get from `gravity_at` in a spot no field
/// reaches, and normalising it would poison every basis downstream with
/// `NaN`. World up is the honest fallback: it is what the camera would
/// have used anyway.
fn normalised_up(up: Vec3) -> Vec3 {
    if up.length_squared() < 1e-12 {
        Vec3::Y
    } else {
        up.normalize()
    }
}

/// A first yaw origin for a vcam that has no previous one.
///
/// # Why this cannot be the per-frame answer
///
/// Any direction perpendicular to `up` will do — yaw is measured from
/// it — but derived from `up` alone it cannot vary continuously. This
/// projects a world axis onto the horizon plane, and that projection
/// collapses where the axis lines up with `up`; the fallback axis is
/// ninety degrees away, so a target rolling over that point swings the
/// camera ninety degrees in, and ninety degrees back out. A hundred and
/// eighty degrees of yaw nobody asked for, at one spot on every planet.
///
/// There is no choice of axis that avoids it. A vector field of
/// references over the sphere would have to be continuous and nowhere
/// zero, and the hairy ball theorem says no such field exists. So the
/// reference is *carried* instead of derived — see [`transported`] —
/// and this only ever answers the first frame.
pub(crate) fn seed_reference(up: Vec3) -> Vec3 {
    let axis = if up.dot(Vec3::Z).abs() > 0.999 {
        Vec3::X
    } else {
        Vec3::Z
    };
    (axis - up * axis.dot(up)).normalize()
}

/// Carries a yaw origin from one up to another, along the shortest arc.
///
/// The reference is only ever nudged by the same rotation that moved
/// `up`, so it has no preferred direction to snap to and no pole to
/// cross. This is what [`seed_reference`] cannot do.
pub fn transported(reference: Vec3, from_up: Vec3, to_up: Vec3) -> Vec3 {
    let (from_up, to_up) = (normalised_up(from_up), normalised_up(to_up));
    let axis = from_up.cross(to_up);
    // Unchanged, or exactly reversed. Reversed has no shortest arc —
    // every perpendicular axis is equally short — so the reference is
    // kept and re-flattened rather than spun through an arbitrary half
    // turn, which is the flip this whole function exists to avoid.
    let turn = match axis.length_squared() > 1e-12 {
        true => glam::Quat::from_axis_angle(axis.normalize(), from_up.angle_between(to_up)),
        false => glam::Quat::IDENTITY,
    };
    flattened(turn * reference, to_up)
}

/// A unit vector on the horizon plane of `up`, nearest to `reference`.
///
/// Transport keeps a carried reference perpendicular to `up` in exact
/// arithmetic; in `f32` it drifts, and a basis built from a reference
/// that has tilted a degree off the plane is a basis that is no longer
/// square.
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

/// A rotation looking from `eye` towards `target`, keeping `up` up.
///
/// Falls back to identity when the two coincide: a camera glued to what
/// it looks at has no direction to face, and normalising a zero vector
/// would hand the renderer a `NaN` pose.
fn look_at(eye: Vec3, target: Vec3, up: Vec3, reference: Vec3) -> glam::Quat {
    let forward = target - eye;
    if forward.length_squared() < 1e-12 {
        return glam::Quat::IDENTITY;
    }
    let forward = forward.normalize();
    // Degenerate when looking along the up axis itself; nudge the
    // reference rather than producing a zero-length right vector.
    // Looking along the up axis itself leaves no right vector. The
    // carried reference is already on the horizon plane and already
    // continuous, so it stands in without reintroducing a fixed axis.
    let up = match forward.dot(up).abs() > 0.999 {
        true => flattened(reference, up),
        false => up,
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
mod tests;

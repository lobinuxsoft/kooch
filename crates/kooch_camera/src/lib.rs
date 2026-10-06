//! Camera behaviour as data: a [`VirtualCamera`] holds a framing and a pose, and the Host in
//! [`plugin`] copies the elected one onto the rendering camera.
//! Design ported from phantom-camera (MIT), Cinemachine's open equivalent (#671).

pub mod aims;
pub mod blend;
pub mod bodies;
pub mod brain;
pub mod extensions;
pub mod frame;
pub mod framing;
pub mod impulse;
pub mod lens_override;
pub mod lookahead;
pub mod occlusion;
pub mod orbit;
pub mod orbital_follow;
pub mod plugin;
pub mod position_composer;
pub mod rig;
pub mod target;
pub mod third_person_aim;
pub mod third_person_follow;
pub mod virtual_camera;
pub mod when;

pub use aims::{HardLookAt, PanTilt, RotateWithFollowTarget};
pub use blend::{
    BLEND_CURVE_CHOICES, BLEND_EASE_CHOICES, CURVE_CUBIC, CURVE_EXPO, CURVE_LINEAR, CURVE_QUAD,
    CURVE_SINE, EASE_IN, EASE_IN_OUT, EASE_OUT,
};
pub use bodies::{Follow, HardLockToTarget};
pub use brain::CameraBrain;
pub use extensions::{CameraOffset, CameraRecomposer};
pub use framing::RotationComposer;
pub use impulse::{ImpulseListener, ImpulseSource, Impulses};
pub use lens_override::LensOverride;
pub use lookahead::CameraLookahead;
pub use occlusion::Deoccluder;
pub use orbit::{CameraOrbit, orbit_cameras};
pub use orbital_follow::OrbitalFollow;
pub use plugin::{
    CameraBlend, CameraComponentsPlugin, CameraPlugin, CameraState, CameraStates, brain_transposes,
    up_for, update_camera_states,
};
pub use position_composer::PositionComposer;
pub use rig::{CameraRig, RigMemory, RigStage, RigStep};
pub use target::{CameraTarget, GroupPose, weighted_centre};
pub use third_person_aim::{ThirdPersonAim, resolve_aims};
pub use third_person_follow::ThirdPersonFollow;
// 🔴 The `FOLLOW_*` and `LOOK_AT_*` constants are **not** here any more (#1397). A mode is a
// component, so naming one from code means adding it; those values are what a migration reads out of
// a file written before that, and nothing else should reach for them.
pub use virtual_camera::{
    INACTIVE_ALWAYS, INACTIVE_NEVER, ORBIT_SPHERE, ORBIT_THREE_RING, UP_GRAVITY, UP_TARGET,
    UP_WORLD, VirtualCamera, seed_reference,
};
pub use when::{CameraWhen, step_camera_whens};

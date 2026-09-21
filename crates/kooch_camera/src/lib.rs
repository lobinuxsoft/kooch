//! Camera behaviour as data: a [`VirtualCamera`] holds a framing and a pose, and the Host in
//! [`plugin`] copies the elected one onto the rendering camera.
//! Design ported from phantom-camera (MIT), Cinemachine's open equivalent (#671).

pub mod blend;
pub mod brain;
pub mod framing;
pub mod lookahead;
pub mod occlusion;
pub mod plugin;
pub mod target;
pub mod virtual_camera;

pub use blend::{
    BLEND_CURVE_CHOICES, BLEND_EASE_CHOICES, CURVE_CUBIC, CURVE_EXPO, CURVE_LINEAR, CURVE_QUAD,
    CURVE_SINE, EASE_IN, EASE_IN_OUT, EASE_OUT,
};
pub use brain::CameraBrain;
pub use framing::CameraFraming;
pub use lookahead::CameraLookahead;
pub use occlusion::CameraCollision;
pub use plugin::{CameraBlend, CameraComponentsPlugin, CameraPlugin, drive_virtual_cameras};
pub use target::{CameraTarget, weighted_centre};
pub use virtual_camera::{
    FOLLOW_GLUED, FOLLOW_NONE, FOLLOW_SIMPLE, FOLLOW_THIRD_PERSON, INACTIVE_ALWAYS, INACTIVE_NEVER,
    LOOK_AT_MIMIC, LOOK_AT_NONE, LOOK_AT_SIMPLE, UP_GRAVITY, UP_TARGET, UP_WORLD, VirtualCamera,
};

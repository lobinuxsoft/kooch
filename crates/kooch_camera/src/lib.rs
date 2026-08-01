//! Camera behaviour as authorable data, rather than code every game
//! rewrites.
//!
//! A camera in this engine is an entity with
//! [`PerspectiveCamera`](kooch_ecs::perspective_camera::PerspectiveCamera)
//! and a [`Transform`](kooch_ecs::transform::Transform), and until this
//! crate nothing moved it. Every project therefore wrote its own follow
//! code, and every project got a different, worse version of the same
//! thing.
//!
//! A [`VirtualCamera`] is an entity that is *not* a camera: it holds a
//! framing — a `follow` mode, a `look_at` mode, a target — and a
//! [`Transform`](kooch_ecs::transform::Transform) it writes itself. The
//! Host in [`plugin`] elects one and copies its pose onto the camera
//! that renders. A scene can hold as many framings as it likes and still
//! render through one camera.
//!
//! # Where the design comes from
//!
//! [phantom-camera](https://github.com/ramokz/phantom-camera) (MIT) is
//! the open equivalent of Unity's Cinemachine, and its design is ported
//! here rather than reinvented: the Host that owns the real camera, the
//! follow/look-at split, the mode names, per-axis damping, and
//! `inactive_update`. Studying a proven design beats guessing at one
//! (#671).
//!
//! The *name* is Cinemachine's, though. phantom-camera calls these
//! `PhantomCamera` because the addon's brand is its node name, which is
//! no use to an engine; and "rig" — what this component was called
//! first — already means something else in Cinemachine, where a
//! `FreeLook` is built out of three of them.
//!
//! # What is deliberately not here yet
//!
//! - **Blending between virtual cameras.** The shape is ready for it:
//!   each vcam holds its own pose, so a blend is an interpolation
//!   between two of them (#671 phase 3).
//! - **`Group`, `Path` and `Framed` follow.** Next, and they need no new
//!   machinery.
//! - **A spring arm that shortens against obstacles.** Needs scene
//!   queries (#562).
//! - **Noise and shake.** Impulse-driven, once there is an impulse.

pub mod blend;
pub mod plugin;
pub mod virtual_camera;

pub use blend::{
    BLEND_CURVE_CHOICES, BLEND_EASE_CHOICES, CURVE_CUBIC, CURVE_EXPO, CURVE_LINEAR, CURVE_QUAD,
    CURVE_SINE, EASE_IN, EASE_IN_OUT, EASE_OUT,
};
pub use plugin::{CameraBlend, CameraComponentsPlugin, CameraPlugin, drive_virtual_cameras};
pub use virtual_camera::{
    FOLLOW_GLUED, FOLLOW_NONE, FOLLOW_SIMPLE, FOLLOW_THIRD_PERSON, INACTIVE_ALWAYS, INACTIVE_NEVER,
    LOOK_AT_MIMIC, LOOK_AT_NONE, LOOK_AT_SIMPLE, UP_GRAVITY, UP_TARGET, UP_WORLD, VirtualCamera,
};

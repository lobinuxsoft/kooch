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
//! [`CameraRig`] is one component: pick a `follow` mode, pick a `look_at`
//! mode, name a target, and the camera does the rest. Nothing is written
//! in a system by the game.
//!
//! # Where the design comes from
//!
//! [phantom-camera](https://github.com/ramokz/phantom-camera) (MIT) is
//! the open equivalent of Unity's Cinemachine, and its vocabulary is
//! ported here rather than reinvented: the follow/look-at split, the mode
//! names, per-axis damping, and `inactive_update`. Studying a proven
//! design beats guessing at one (#671).
//!
//! # What is deliberately not here yet
//!
//! - **Blending between rigs.** That is what forces virtual cameras to be
//!   separate entities from the rendering camera; see [`CameraRig`] for
//!   why one rig per camera is right until then.
//! - **`Group`, `Path` and `Framed` follow.** Next, and they need no new
//!   machinery.
//! - **A spring arm that shortens against obstacles.** Needs scene
//!   queries (#562).
//! - **Noise and shake.** Impulse-driven, once there is an impulse.

pub mod plugin;
pub mod rig;

pub use plugin::{CameraComponentsPlugin, CameraPlugin, drive_camera_rigs};
pub use rig::{
    CameraRig, FOLLOW_GLUED, FOLLOW_NONE, FOLLOW_SIMPLE, FOLLOW_THIRD_PERSON, INACTIVE_ALWAYS,
    INACTIVE_NEVER, LOOK_AT_MIMIC, LOOK_AT_NONE, LOOK_AT_SIMPLE,
};

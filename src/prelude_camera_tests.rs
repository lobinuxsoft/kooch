//! 🔴 #1384: the facade is the only surface a project sees, and it is the one the routine
//! `cargo check --workspace` cannot see — `camera` is not a default feature, so the `pub use` that
//! names these constants is behind a `#[cfg]` nothing in the default build turns on. #1380 renamed
//! one of them, every crate compiled, and roll-a-ball would not build.
//!
//! Run with the features a game actually uses:
//!   cargo test -p kooch --features camera,physics,gravity,character,audio,blockmesh

use crate::prelude::*;

/// 🔴 Every mode is a **component** now (#1397), so "reachable" means the type is nameable from the
/// prelude — a project that cannot name `OrbitalFollow` cannot give a camera a body at all. Naming
/// them is the test: a type that stopped existing does not compile.
#[test]
fn every_camera_mode_is_reachable() {
    let bodies: Vec<&str> = vec![
        std::any::type_name::<HardLockToTarget>(),
        std::any::type_name::<Follow>(),
        std::any::type_name::<OrbitalFollow>(),
        std::any::type_name::<ThirdPersonFollow>(),
        std::any::type_name::<PositionComposer>(),
    ];
    let aims: Vec<&str> = vec![
        std::any::type_name::<HardLookAt>(),
        std::any::type_name::<PanTilt>(),
        std::any::type_name::<RotateWithFollowTarget>(),
        std::any::type_name::<RotationComposer>(),
    ];
    assert_eq!(bodies.len(), 5);
    assert_eq!(aims.len(), 4);

    // The one dropdown left is a field of the body that reads it, and it is still a list.
    let surfaces = [ORBIT_SPHERE, ORBIT_THREE_RING];
    assert_eq!(
        surfaces.len(),
        kooch_camera::virtual_camera::ORBIT_STYLE_CHOICES.len(),
    );
    let ups = [UP_WORLD, UP_GRAVITY, UP_TARGET];
    assert_eq!(
        ups.len(),
        kooch_camera::virtual_camera::UP_MODE_CHOICES.len(),
    );
}

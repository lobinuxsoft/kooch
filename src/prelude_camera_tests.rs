//! 🔴 #1384: the facade is the only surface a project sees, and it is the one the routine
//! `cargo check --workspace` cannot see — `camera` is not a default feature, so the `pub use` that
//! names these constants is behind a `#[cfg]` nothing in the default build turns on. #1380 renamed
//! one of them, every crate compiled, and roll-a-ball would not build.
//!
//! Run with the features a game actually uses:
//!   cargo test -p kooch --features camera,physics,gravity,character,audio,blockmesh

use crate::prelude::*;

/// Every mode the Inspector offers is reachable by name from the prelude. Naming them is the test:
/// a constant that stopped existing does not compile, which is the failure this file is for.
#[test]
fn every_camera_mode_is_reachable() {
    let bodies = [
        FOLLOW_NONE,
        FOLLOW_GLUED,
        FOLLOW_SIMPLE,
        FOLLOW_ORBITAL,
        FOLLOW_POSITION_COMPOSER,
        FOLLOW_SHOULDER,
    ];
    let aims = [
        LOOK_AT_NONE,
        LOOK_AT_MIMIC,
        LOOK_AT_SIMPLE,
        LOOK_AT_ARM,
        LOOK_AT_COMPOSED,
    ];
    let ups = [UP_WORLD, UP_GRAVITY, UP_TARGET];

    // And each list is as long as the dropdown that offers it: a mode added to the engine and not
    // exported is one a project cannot ask for from code.
    assert_eq!(
        bodies.len(),
        kooch_camera::virtual_camera::FOLLOW_MODE_CHOICES.len(),
    );
    assert_eq!(
        aims.len(),
        kooch_camera::virtual_camera::LOOK_AT_CHOICES.len(),
    );
    assert_eq!(
        ups.len(),
        kooch_camera::virtual_camera::UP_MODE_CHOICES.len(),
    );
}

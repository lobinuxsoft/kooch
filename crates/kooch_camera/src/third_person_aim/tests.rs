use super::*;

/// The camera stands 0.6 m to the right of a character at the origin, both looking down −Z.
fn aim() -> ThirdPersonAim {
    ThirdPersonAim {
        aim_distance: 100.0,
        ..Default::default()
    }
}

/// Without a solver there is nothing to hit, so the point sits at the full reach — and a game
/// without physics still gets a reticle that means something.
#[test]
fn nothing_to_hit_reaches_full() {
    let resources = Resources::new();
    let eye = Vec3::new(0.6, 0.0, 3.0);
    let at = aim().resolved(&resources, eye, Vec3::ZERO, Vec3::NEG_Z);
    // The ray starts level with the character, 3 m along, and reaches the rest.
    assert!((at.z + 97.0).abs() < 1e-3, "{at:?}");
    assert!((at.x - 0.6).abs() < 1e-3, "it left the view axis: {at:?}");
}

/// 🔴 The character's own back never answers the ray. It starts level with them, so a camera behind
/// a character aims past them rather than at them.
#[test]
fn the_character_is_not_what_it_aims_at() {
    let resources = Resources::new();
    let eye = Vec3::Z * 3.0;
    let at = aim().resolved(&resources, eye, Vec3::ZERO, Vec3::NEG_Z);
    assert!(at.z < -1.0, "it stopped at or before the character: {at:?}");
}

/// A camera in front of its target — a look-back rig — has nothing to skip, and the reach is whole.
#[test]
fn a_target_behind_skips_nothing() {
    let resources = Resources::new();
    let eye = Vec3::NEG_Z * 3.0;
    let at = aim().resolved(&resources, eye, Vec3::ZERO, Vec3::NEG_Z);
    assert!((at.z + 103.0).abs() < 1e-3, "{at:?}");
}

/// 🔴 Two points, and this is why: the camera stands beside the character, so the line the character
/// shoots along is not the view axis. Without a solver the second ray hits nothing and the answer is
/// the camera's point — the shape is what is asserted here, the difference needs geometry.
#[test]
fn what_the_character_reaches_is_its_own_question() {
    let resources = Resources::new();
    let looking_at = Vec3::new(0.6, 0.0, -97.0);
    let reached = aim().reached(&resources, Vec3::ZERO, looking_at);
    assert_eq!(reached, looking_at);
}

/// A target standing exactly on the point has no direction to cast along, and a normalised zero is
/// a `NaN` reticle.
#[test]
fn a_coincident_target_is_not_nan() {
    let resources = Resources::new();
    let reached = aim().reached(&resources, Vec3::ZERO, Vec3::ZERO);
    assert!(reached.is_finite(), "{reached:?}");
}

/// The reach never runs backwards: a character further than `aim_distance` would otherwise leave a
/// negative ray, which a solver reads as no ray at all.
#[test]
fn a_distant_character_still_aims_forward() {
    let resources = Resources::new();
    let far = ThirdPersonAim {
        aim_distance: 5.0,
        ..aim()
    };
    let at = far.resolved(&resources, Vec3::Z * 50.0, Vec3::ZERO, Vec3::NEG_Z);
    assert!(at.z < 0.0, "the ray folded back on itself: {at:?}");
}

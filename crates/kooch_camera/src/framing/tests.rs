use super::*;

const DT: f32 = 1.0 / 60.0;

/// 90° over a square screen: at 1 m depth the screen is 2 m by 2 m, so fractions read as metres/2.
fn lens() -> Lens {
    Lens::new(90.0, 1.0)
}

fn framing() -> CameraFraming {
    CameraFraming {
        dead_zone: Vec2::splat(0.2),
        soft_zone: Vec2::splat(0.6),
        ..Default::default()
    }
}

/// Steps the tracked point once, the camera looking down -Z from 1 m away.
fn step(framing: &CameraFraming, tracked: Vec3, target: Vec3) -> Vec3 {
    framing.follow(tracked, target, Quat::IDENTITY, 1.0, lens(), DT)
}

#[test]
fn inside_the_dead_zone_holds() {
    // 0.1 of the screen is 0.2 m here; the dead zone reaches 0.1 either side.
    let target = Vec3::new(0.19, -0.19, 0.0);
    assert_eq!(step(&framing(), Vec3::ZERO, target), Vec3::ZERO);
}

#[test]
fn the_soft_zone_eases_back() {
    let target = Vec3::new(0.4, 0.0, 0.0);
    let moved = step(&framing(), Vec3::ZERO, target).x;
    assert!(moved > 0.0 && moved < 0.2, "eased, not snapped: {moved}");
}

#[test]
fn the_soft_edge_is_hard() {
    // 0.5 of the screen right is 0.2 past the soft edge (0.3): all of it at once, plus some easing.
    let target = Vec3::new(1.0, 0.0, 0.0);
    let tracked = step(&framing(), Vec3::ZERO, target);
    let left = (target.x - tracked.x) / 2.0;
    assert!(
        left <= 0.3 + 1e-5,
        "target is back inside the soft zone: {left}"
    );
}

#[test]
fn a_rigid_soft_zone_reaches_the_dead_edge() {
    let rigid = CameraFraming {
        soft_time: 0.0,
        ..framing()
    };
    let tracked = step(&rigid, Vec3::ZERO, Vec3::new(0.4, 0.0, 0.0));
    assert!(
        (tracked.x - 0.2).abs() < 1e-5,
        "target on the dead edge: {tracked}"
    );
}

#[test]
fn depth_follows_at_once() {
    let tracked = step(&framing(), Vec3::ZERO, Vec3::new(0.0, 0.0, -3.0));
    assert_eq!(tracked, Vec3::new(0.0, 0.0, -3.0));
}

/// A soft zone authored smaller than the dead zone is the dead zone: no gap to ease across.
#[test]
fn a_small_soft_zone_clamps() {
    let odd = CameraFraming {
        dead_zone: Vec2::splat(0.4),
        soft_zone: Vec2::splat(0.1),
        ..framing()
    };
    let tracked = step(&odd, Vec3::ZERO, Vec3::new(0.6, 0.0, 0.0));
    assert!((tracked.x - 0.2).abs() < 1e-5, "{tracked}");
}

#[test]
fn the_aim_offsets_the_screen() {
    let right = CameraFraming {
        screen: Vec2::new(0.25, 0.0),
        ..framing()
    };
    let aim = right.aim(Vec3::ZERO, Quat::IDENTITY, 1.0, lens());
    // Looking left of the point puts the point right of centre.
    assert!((aim.x + 0.5).abs() < 1e-5, "{aim}");
}

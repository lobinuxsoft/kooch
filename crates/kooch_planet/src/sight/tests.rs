use glam::DVec3;

use super::Cone;

/// A camera looking down -Z sees ahead of it, not behind it, and always what it stands in.
#[test]
fn a_cone_sees_ahead_only() {
    let cone = Cone::new(-DVec3::Z, 60f64.to_radians(), 16.0 / 9.0);
    assert!(cone.holds(DVec3::new(0.0, 0.0, -100.0), 1.0));
    assert!(!cone.holds(DVec3::new(0.0, 0.0, 100.0), 1.0));
    assert!(!cone.holds(DVec3::new(100.0, 0.0, 1.0), 1.0));
    assert!(cone.holds(DVec3::new(0.0, 0.0, 5.0), 10.0));
}

use super::*;
use glam::DVec3;

#[test]
fn default_is_zero() {
    let origin = ActiveOrigin::default();
    assert_eq!(origin, ActiveOrigin::ZERO);
    assert_eq!(origin.coord(), UniverseCoord::ZERO);
}

#[test]
fn set_updates_coord() {
    let mut origin = ActiveOrigin::default();
    let new_coord = UniverseCoord::from_dvec3(DVec3::new(5000.0, 0.0, 0.0));
    origin.set(new_coord);
    assert_eq!(origin.coord(), new_coord);
}

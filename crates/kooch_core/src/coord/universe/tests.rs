use super::*;

const EPS: f64 = 1e-9;

#[test]
fn zero_round_trip() {
    let z = UniverseCoord::ZERO;
    assert_eq!(z.to_dvec3(), DVec3::ZERO);
    assert_eq!(UniverseCoord::from_dvec3(DVec3::ZERO), UniverseCoord::ZERO);
}

#[test]
fn round_trip_arbitrary_position() {
    let world = DVec3::new(1234.5, -678.9, 0.001);
    let uc = UniverseCoord::from_dvec3(world);
    assert!((uc.to_dvec3() - world).length() < EPS);
}

#[test]
fn sector_zero_covers_centred_range() {
    // Center of sector 0.
    assert_eq!(UniverseCoord::from_dvec3(DVec3::ZERO).sector, IVec3::ZERO);
    // Just inside the upper edge — still sector 0 (upper exclusive).
    let near_edge = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF - 1.0, 0.0, 0.0));
    assert_eq!(near_edge.sector, IVec3::ZERO);
    // At the lower edge — sector 0 (lower inclusive).
    let lower_edge = UniverseCoord::from_dvec3(DVec3::new(-SECTOR_HALF, 0.0, 0.0));
    assert_eq!(lower_edge.sector, IVec3::ZERO);
}

#[test]
fn crossing_upper_boundary_increments_sector() {
    // SECTOR_HALF = 512. (513, 0, 0) lands in sector (1, 0, 0)
    // because the upper edge is exclusive.
    let uc = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF + 1.0, 0.0, 0.0));
    assert_eq!(uc.sector, IVec3::new(1, 0, 0));
    // 513 - 1024 = -511.
    assert!((uc.offset.x - (-SECTOR_HALF + 1.0)).abs() < EPS);
}

#[test]
fn negative_world_lands_in_negative_sector() {
    let world = DVec3::new(-1500.0, -200.0, 50.0);
    let uc = UniverseCoord::from_dvec3(world);
    // -1500 + 512 = -988; floor(-988 / 1024) = -1.
    assert_eq!(uc.sector.x, -1);
    // y = -200 stays in sector 0 (within half).
    assert_eq!(uc.sector.y, 0);
    // z = 50 stays in sector 0.
    assert_eq!(uc.sector.z, 0);
    assert!((uc.to_dvec3() - world).length() < EPS);
}

#[test]
fn translate_across_sector_boundary() {
    let start = UniverseCoord::from_dvec3(DVec3::new(SECTOR_HALF - 1.0, 0.0, 0.0));
    assert_eq!(start.sector, IVec3::ZERO);
    let after = start.translated(DVec3::new(2.0, 0.0, 0.0));
    assert_eq!(after.sector, IVec3::new(1, 0, 0));
    assert!((after.offset.x - (-SECTOR_HALF + 1.0)).abs() < EPS);
}

#[test]
fn translate_back_returns_to_origin_sector() {
    let start = UniverseCoord::ZERO;
    let mid = start.translated(DVec3::new(SECTOR_SIZE_METERS * 5.0, 0.0, 0.0));
    assert_eq!(mid.sector, IVec3::new(5, 0, 0));
    let back = mid.translated(DVec3::new(-SECTOR_SIZE_METERS * 5.0, 0.0, 0.0));
    assert_eq!(back.sector, IVec3::ZERO);
    assert!(back.offset.length() < EPS);
}

#[test]
fn delta_to_preserves_precision_far_from_origin() {
    // Two coords 1 million meters from origin, 10 m apart.
    let a = UniverseCoord::from_dvec3(DVec3::new(1_000_000.0, 0.0, 0.0));
    let b = UniverseCoord::from_dvec3(DVec3::new(1_000_010.0, 0.0, 0.0));
    let delta = a.delta_to(&b);
    assert!((delta.x - 10.0).abs() < EPS);
    // f32 cast loses no precision because the delta is small.
    let delta_f32 = delta.as_vec3();
    assert!((delta_f32.x - 10.0).abs() < 1e-5);
}

#[test]
fn normalised_idempotent() {
    // Build a non-canonical coord (offset outside the half range).
    let weird = UniverseCoord::new(IVec3::ZERO, DVec3::new(SECTOR_SIZE_METERS * 3.5, 0.0, 0.0));
    let canon = weird.normalised();
    // Canonical sector for world = 3584 m: 3584 + 512 = 4096; / 1024 = 4.0; floor = 4.
    assert_eq!(canon.sector.x, 4);
    // Offset = 3584 - 4096 = -512.
    assert!((canon.offset.x - (-SECTOR_HALF)).abs() < EPS);
    // Calling normalised again is a no-op.
    assert_eq!(canon, canon.normalised());
}

#[test]
fn default_is_zero() {
    assert_eq!(UniverseCoord::default(), UniverseCoord::ZERO);
}

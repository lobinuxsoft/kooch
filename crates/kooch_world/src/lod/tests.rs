use super::*;

#[test]
fn default_is_conservative_single_ring() {
    // Editor-default: 1 LOD-0 ring × 256 m.
    let cfg = LodRingConfig::default();
    assert_eq!(cfg.lod_count(), 1);
    assert_eq!(cfg.radius_for(0), 256.0);
}

#[test]
fn aspirational_config_has_four_rings() {
    let cfg = LodRingConfig::aspirational_planet_scale();
    assert_eq!(cfg.lod_count(), 4);
    assert_eq!(cfg.radius_for(0), 512.0);
    assert_eq!(cfg.radius_for(1), 2000.0);
    assert_eq!(cfg.radius_for(2), 8000.0);
    assert_eq!(cfg.radius_for(3), 32000.0);
}

#[test]
fn radius_for_out_of_range_is_zero() {
    let cfg = LodRingConfig::default();
    assert_eq!(cfg.radius_for(99), 0.0);
}

#[test]
fn max_radius_returns_largest() {
    let cfg = LodRingConfig::aspirational_planet_scale();
    assert_eq!(cfg.max_radius(), 32000.0);
}

#[test]
fn max_radius_empty_config_is_zero() {
    let cfg = LodRingConfig { rings: vec![] };
    assert_eq!(cfg.max_radius(), 0.0);
}

#[test]
fn aspirational_radii_strictly_increase() {
    let cfg = LodRingConfig::aspirational_planet_scale();
    let mut prev = 0.0;
    for r in &cfg.rings {
        assert!(r.radius_meters > prev, "ring {:?} not strictly larger", r);
        prev = r.radius_meters;
    }
}

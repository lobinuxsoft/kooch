use super::*;

use kooch_lighting::ClusterSettings;

/// A camera looking down -Z from the origin, with the engine's own
/// projection so the census is exercised against the reversed infinite
/// frustum rather than a friendlier one.
fn camera(viewport: Vec2) -> CensusCamera {
    CensusCamera {
        world_from_view: Mat4::IDENTITY,
        clip_from_view: crate::projection::perspective_infinite_rh_reverse_z(
            60f32.to_radians(),
            viewport.x / viewport.y,
            0.1,
        ),
        viewport,
    }
}

fn grid(viewport: Vec2) -> ClusterGrid {
    ClusterGrid::new(&ClusterSettings::default(), viewport)
}

#[test]
fn a_chain_ends_at_one_page() {
    let config = PageConfig::default();
    assert_eq!(config.side(0), 128);
    assert_eq!(config.side(config.levels() - 1), 1);
}

#[test]
fn levels_cover_every_page() {
    let config = PageConfig::default();
    let counted: u32 = (0..config.levels()).map(|l| config.side(l).pow(2)).sum();
    assert_eq!(counted, config.per_face());
    assert_eq!(config.level_base(0), 0);
    assert_eq!(config.level_base(1), config.side(0).pow(2));
}

#[test]
fn a_page_marks_once() {
    let mut out = PageCensus::new(PageConfig::default(), ClipmapConfig::default(), 2);
    assert!(out.mark(0, 0, 0, 3, 4));
    assert!(!out.mark(0, 0, 0, 3, 4));
    assert!(out.mark(1, 0, 0, 3, 4));
    assert!(out.mark(0, 1, 0, 3, 4));
    assert!(out.mark(0, 0, 1, 3, 4));
    assert_eq!(out.resident(), 4);
}

#[test]
fn a_distant_light_is_coarse() {
    let config = PageConfig::default();
    // Same screen-pixel footprint, one metre away against a hundred.
    let near = level_for(config, 1.0, 0.01);
    let far = level_for(config, 100.0, 0.01);
    assert!(near > far, "near {near} should be coarser than far {far}");
}

#[test]
fn an_unlit_frame_residents_nothing() {
    let viewport = Vec2::new(1280.0, 720.0);
    let out = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[],
    );
    assert_eq!(out.resident(), 0);
    assert_eq!(out.bytes(), 0);
}

#[test]
fn a_light_out_of_reach_is_skipped() {
    let viewport = Vec2::new(1280.0, 720.0);
    // Behind the camera, and a range that cannot cross the origin.
    let light = CensusLight::point(Vec3::new(0.0, 0.0, 500.0), 1.0);
    let out = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[light],
    );
    assert_eq!(out.pairs(), 0);
    assert_eq!(out.resident(), 0);
}

#[test]
fn residency_follows_the_screen() {
    let viewport = Vec2::new(1280.0, 720.0);
    let light = CensusLight::point(Vec3::new(0.0, 0.0, -10.0), 12.0);
    let out = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[light],
    );
    assert!(out.pairs() > 0, "the light reaches cells in front of it");
    assert!(out.resident() > 0, "and those cells need pages");
    // The whole point of the structure: what is resident is a fraction
    // of what the light could address.
    let addressable = PageConfig::default().per_face() * CUBE_FACES as u32;
    assert!(
        out.resident() < addressable,
        "resident {} of {addressable} addressable",
        out.resident()
    );
}

#[test]
fn a_smaller_page_residents_more() {
    let viewport = Vec2::new(1280.0, 720.0);
    let light = CensusLight::point(Vec3::new(0.0, 0.0, -10.0), 12.0);
    let coarse = census(
        PageConfig {
            page: 256,
            virtual_size: 16384,
        },
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[light],
    );
    let fine = census(
        PageConfig {
            page: 64,
            virtual_size: 16384,
        },
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[light],
    );
    assert!(
        fine.resident() > coarse.resident(),
        "fine {} vs coarse {}",
        fine.resident(),
        coarse.resident()
    );
}

#[test]
fn a_spot_pays_one_face() {
    let viewport = Vec2::new(1280.0, 720.0);
    let at = Vec3::new(0.0, 0.0, -10.0);
    let point = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[CensusLight::point(at, 12.0)],
    );
    let spot = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &camera(viewport),
        &[CensusLight::spot(at, 12.0)],
    );
    assert!(
        spot.resident() < point.resident(),
        "spot {} vs point {}",
        spot.resident(),
        point.resident()
    );
}

#[test]
fn a_sun_residents_a_fraction() {
    let viewport = Vec2::new(1280.0, 720.0);
    let config = PageConfig::default();
    let clipmap = ClipmapConfig::default();
    let out = census(
        config,
        clipmap,
        &grid(viewport),
        &camera(viewport),
        &[CensusLight::sun(Vec3::new(-0.3, -1.0, -0.2))],
    );
    assert!(out.resident() > 0, "the sun reaches every cell");
    let addressable = clipmap.levels * config.side(0).pow(2);
    assert!(
        out.resident() < addressable,
        "resident {} of {addressable} addressable",
        out.resident()
    );
}

#[test]
fn a_coarse_level_holds_the_far_cells() {
    // Containment, not density, is what decides out there: no level
    // finer than the cell's own reach can hold it, whatever the screen
    // asked for.
    let clipmap = ClipmapConfig::default();
    assert_eq!(level_above(0.5), 0);
    assert_eq!(level_above(2.0), 1);
    assert_eq!(level_above(5.0), 3);
    assert_eq!(clipmap.extent(0), clipmap.base);
    assert_eq!(clipmap.extent(3), clipmap.base * 8.0);
}

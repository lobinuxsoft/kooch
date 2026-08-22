use super::*;

use super::pool::PoolConfig;

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

/// The frame a test censuses: this camera, these lights, no surface
/// filter — the walk over the frustum's whole volume.
fn frame<'a>(viewport: Vec2, lights: &'a [CensusLight]) -> CensusFrame<'a> {
    CensusFrame {
        camera: camera(viewport),
        lights,
        surfaces: &[],
    }
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
    assert_eq!(counted, config.face_pages());
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
        &frame(viewport, &[]),
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
        &frame(viewport, &[light]),
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
        &frame(viewport, &[light]),
    );
    assert!(out.pairs() > 0, "the light reaches cells in front of it");
    assert!(out.resident() > 0, "and those cells need pages");
    // The whole point of the structure: what is resident is a fraction
    // of what the light could address.
    let addressable = PageConfig::default().face_pages() * CUBE_FACES as u32;
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
        &frame(viewport, &[light]),
    );
    let fine = census(
        PageConfig {
            page: 64,
            virtual_size: 16384,
        },
        ClipmapConfig::default(),
        &grid(viewport),
        &frame(viewport, &[light]),
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
        &frame(viewport, &[CensusLight::point(at, 12.0)]),
    );
    let spot = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &frame(viewport, &[CensusLight::spot(at, 12.0)]),
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
        &frame(viewport, &[CensusLight::sun(Vec3::new(-0.3, -1.0, -0.2))]),
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

#[test]
fn boxes_that_touch_overlap() {
    let a = WorldBox::new(Vec3::ZERO, Vec3::ONE);
    assert!(a.overlaps(&WorldBox::new(Vec3::splat(0.5), Vec3::splat(2.0))));
    assert!(
        a.overlaps(&WorldBox::new(Vec3::ONE, Vec3::splat(2.0))),
        "touching"
    );
    assert!(!a.overlaps(&WorldBox::new(Vec3::splat(1.01), Vec3::splat(2.0))));
    // Built from either corner, and it is the same box.
    assert_eq!(WorldBox::new(Vec3::ONE, Vec3::ZERO), a);
}

#[test]
fn a_surface_narrows_the_walk() {
    let viewport = Vec2::new(1280.0, 720.0);
    let lights = [CensusLight::point(Vec3::new(0.0, 0.0, -10.0), 12.0)];
    let whole = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &frame(viewport, &lights),
    );
    // One small box in front of the camera, where the light is.
    let surfaces = [WorldBox::new(Vec3::splat(-1.0), Vec3::new(1.0, 1.0, -9.0))];
    let narrowed = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &CensusFrame {
            camera: camera(viewport),
            lights: &lights,
            surfaces: &surfaces,
        },
    );
    assert!(
        narrowed.cells() < whole.cells(),
        "cells {} vs {}",
        narrowed.cells(),
        whole.cells()
    );
    assert!(
        narrowed.resident() < whole.resident(),
        "resident {} vs {}",
        narrowed.resident(),
        whole.resident()
    );
}

#[test]
fn a_surface_nothing_reaches_residents_nothing() {
    let viewport = Vec2::new(1280.0, 720.0);
    let lights = [CensusLight::point(Vec3::new(0.0, 0.0, -10.0), 12.0)];
    // Behind the camera, so no cell of the grid overlaps it.
    let surfaces = [WorldBox::new(Vec3::splat(400.0), Vec3::splat(401.0))];
    let out = census(
        PageConfig::default(),
        ClipmapConfig::default(),
        &grid(viewport),
        &CensusFrame {
            camera: camera(viewport),
            lights: &lights,
            surfaces: &surfaces,
        },
    );
    assert_eq!(out.cells(), 0);
    assert_eq!(out.resident(), 0);
}

#[test]
fn the_table_stays_half_empty() {
    // The load factor is the whole reason a probe is cheap: at 0.5 the
    // expected count is under two. A table sized to the pool rather than
    // to twice it would spend most inserts walking.
    for pages in [64u32, 1000, 4096, 6144] {
        let config = PoolConfig { pages, views: 1 };
        let entries = config.entries();
        assert!(entries.is_power_of_two(), "the mask has to be an `and`");
        assert!(
            entries >= pages * 2,
            "{entries} entries for {pages} pages is past half full"
        );
    }
}

#[test]
fn the_table_is_kilobytes_not_megabytes() {
    // The number that killed the flat answer. 101 lights and a sun
    // address 28 409 856 pages; a `u32` each is 108 MiB, 42 % of the
    // pool it would index. Sized to residency instead, Epic's own
    // 4096-page pool costs this.
    let config = PoolConfig {
        pages: POOL_PAGES,
        views: 1,
    };
    assert!(
        config.table_bytes() < 128 * 1024,
        "the table is {} bytes",
        config.table_bytes()
    );
}

#[test]
fn the_atlas_is_square_enough() {
    // A strip would pass `max_texture_dimension_2d` the moment a page is
    // wider than a handful of texels.
    let page = PageConfig::default();
    for pages in [64u32, 1000, 4096, 8192] {
        let config = PoolConfig { pages, views: 1 };
        let side = config.per_row() * page.page;
        assert!(side <= 16384, "{pages} pages want a {side}-texel atlas");
        assert!(
            config.per_row().pow(2) >= pages,
            "{} across does not hold {pages}",
            config.per_row()
        );
    }
}

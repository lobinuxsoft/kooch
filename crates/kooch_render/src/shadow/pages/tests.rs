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
    //
    // 128 KiB with three words an entry — slot, age, listing. The third
    // word cost 32 KiB and bought the route back from a page key to its
    // place in the compacted list; the comparison that matters is
    // against the 108 MiB, not against the previous kilobyte count.
    let config = PoolConfig {
        pages: POOL_PAGES,
        views: 1,
    };
    let flat = 28_409_856u64 * 4;
    assert!(
        config.table_bytes() < 256 * 1024,
        "the table is {} bytes",
        config.table_bytes()
    );
    assert!(
        config.table_bytes() * 800 < flat,
        "the table is {} bytes against the flat answer's {flat}; it has          stopped being three orders of magnitude cheaper",
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

/// The cascades stop being rasterised when the pages replace them.
///
/// # 🔴 The flag existed and nobody asked it
///
/// `FrameShadows::cascades_enabled` already reached the shading, and
/// `inti_shadow` already returned from the page branch without touching
/// a cascade layer. What no code path consulted was whether to DRAW
/// them: four culls and four depth passes ran every frame filling
/// layers with no reader, for as long as the feature has existed.
///
/// A source check rather than a GPU one. The claim is "this call is
/// gated", and answering it on a device would mean standing up a whole
/// frame to observe an absence — which is the shape of assertion that
/// passes for the wrong reason.
#[test]
fn the_cascades_are_not_drawn_for_a_paged_sun() {
    let pass = include_str!("../pass.rs");
    let gate = pass
        .find("if prepared.draw_cascades {")
        .expect("`ShadowPass::record` no longer gates on the flag");
    let render = pass[gate..]
        .find("self.rasterizer.render(")
        .expect("the cascade raster left `record`; this test now guards nothing");
    // The spot and point draws follow and must NOT be inside the gate:
    // they share this atlas and have no page raster to replace them.
    let closes = pass[gate..]
        .find("\n        }\n")
        .expect("the gate never closes");
    assert!(
        render < closes,
        "the cascade raster is outside the gate it was supposed to be inside"
    );
    for other in ["render_points(", "render_spots("] {
        let at = pass[gate..]
            .find(other)
            .unwrap_or_else(|| panic!("`{other}` is gone from `record`"));
        assert!(
            at > closes,
            "`{other}` was pulled inside the cascade gate; the local lights \
             have no page raster yet and would stop casting"
        );
    }

    // 🔴 And the gate is its OWN flag. `cascades_enabled` means the
    // sun's data in the frame uniform is valid, and
    // `IntiFrame::with_optional_shadows` turns `shadows_enabled` on
    // when it is — a flag `inti_shadow` checks BEFORE it branches to
    // the pages. Folding the raster's decision into it turned the whole
    // sun off: fully lit everywhere, cascades and pages alike, which is
    // exactly what shipped.
    let frame = include_str!("../../meshlet/render_stage/frame/shadows.rs");
    assert!(
        frame.contains("let draw_cascades = cascades_enabled && !settings.virtual_pages;"),
        "the raster's gate stopped consulting `virtual_pages`"
    );
    assert!(
        frame.contains("let cascades_enabled = sun.is_some();"),
        "`cascades_enabled` is deciding something other than whether there is a sun; \
         `shadows_enabled` rides on it and turning it off turns the PAGES off too"
    );
}

/// The flag the raster's gate must not borrow.
///
/// 🔴 What shipped: gating the cascade draw on `cascades_enabled` and
/// then computing that flag from `virtual_pages`. `with_optional_shadows`
/// only calls `with_shadows` — the one place `shadows_enabled` is set —
/// when it is true, and `inti_shadow` returns fully lit on
/// `shadows_enabled` BEFORE it reaches the branch that picks pages over
/// cascades. So turning the cascade DRAW off turned every shadow in the
/// scene off, which is a whole-frame regression with no error anywhere.
#[test]
fn the_sampling_switch_is_not_the_drawing_switch() {
    let reader = include_str!("../../../../kooch_lighting/shaders/inti_pbr.wgsl");
    let branch = reader
        .find("inti_pages.sun.w > 0.5")
        .expect("the page branch is gone from `inti_shadow`");
    let guard = reader
        .find("inti.shadows_enabled == 0u")
        .expect("the master switch is gone from `inti_shadow`");
    assert!(
        guard < branch,
        "the page branch now runs before the `shadows_enabled` check; this test          guards an ordering that no longer exists"
    );

    // The one place that flag is set, and the condition it rides on.
    let frame = include_str!("../../../../kooch_lighting/src/frame.rs");
    assert!(
        frame.contains("let frame = if s.cascades_enabled {"),
        "`with_optional_shadows` no longer gates `with_shadows` on `cascades_enabled`;          the coupling this test exists for has moved"
    );
}

/// A lamp's pages are READ, and read without a cube slot.
///
/// # 🔴 The half that made the other three invisible
///
/// The expansion tested lamp pages, the depth pass drew them, the pool
/// claimed them — and `inti_point_shadow` sampled the cube atlas anyway.
/// A pass that costs and shows nothing, with 7937 meshlet/page pairs a
/// frame to prove it was running.
///
/// The second claim matters as much as the first: the cube path returns
/// fully lit for any lamp past `MAX_POINT_SHADOWS`, which is 32 against
/// a scene of a hundred. Gating the PAGE path on the same slot would
/// carry that ceiling straight into the technique built to remove it.
///
/// A source check because the alternative is a GPU rig with a hundred
/// lamps to observe the hundredth one — and what is being asserted is a
/// branch, not a pixel.
#[test]
fn a_lamp_reads_its_pages_without_a_cube_slot() {
    let shading = include_str!("../../../../kooch_lighting/shaders/inti_pbr.wgsl");

    for kind in ["INTI_KIND_POINT", "INTI_KIND_SPOT"] {
        // `else if`, so this is the SHADOW branch and not the cone
        // falloff that tests the same discriminant earlier in the file.
        let at = shading
            .find(&format!("}} else if (light.kind == {kind}) {{"))
            .unwrap_or_else(|| panic!("the {kind} shadow branch is gone from the shading"));
        let branch = &shading[at..(at + 1400).min(shading.len())];
        assert!(
            branch.contains("inti_local_page_shadow("),
            "{kind} never reaches the page reader; its pages are drawn and never sampled"
        );
        // The page call must come BEFORE the slot test, or it inherits
        // the cube budget it exists to replace.
        let page = branch
            .find("inti_local_page_shadow(")
            .expect("checked above");
        let slot = branch
            .find("light.shadow_slot != INTI_NO_SHADOW_SLOT")
            .unwrap_or(usize::MAX);
        assert!(
            page < slot,
            "{kind} gates its page read on a cube slot; a lamp past the 32-cube budget \
             would stay fully lit with its own pages sitting drawn in the pool"
        );
    }

    // 🔴 And it biases like a LAMP. `INTI_POINT_DEPTH_BIAS` is four
    // times `INTI_DEPTH_BIAS`, and the doc beside those constants says
    // why and what borrowing the sun's looks like: a stair-stepped
    // square printed on an empty floor under a lamp, the floor
    // shadowing itself. That doc exists because it already happened
    // once to the cube reader; the page reader shipped repeating it.
    let reader = shading
        .find("fn inti_local_page_shadow(")
        .expect("the page reader is gone");
    let end = shading[reader..]
        .find("\n}\n")
        .map(|e| reader + e)
        .unwrap_or(shading.len());
    // Comments stripped: this one NAMES the cascade constants to explain
    // why it does not use them, and a scan that reads prose finds what
    // the prose is warning about.
    let body: String = shading[reader..end]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("INTI_POINT_DEPTH_BIAS") && body.contains("INTI_POINT_NORMAL_BIAS"),
        "the page reader biases with the sun's constants; a cube face is 90 degrees and \
         its texels are coarse, so a quarter of the depth push it needs prints the floor \
         on itself"
    );
    assert!(
        !body.contains("* INTI_DEPTH_BIAS") && !body.contains("* INTI_NORMAL_BIAS"),
        "the page reader still reaches for a cascade's bias somewhere"
    );

    // 🔴 And the reader agrees with the writer on depth, ALONG THE MAJOR
    // AXIS. Two claims in one:
    //
    // The `w` is what makes a lamp's page a projection instead of a
    // mapping. Dividing at the vertex and handing the rasteriser `w = 1`
    // fills the triangle by straight lines between three
    // separately-divided corners — correct at the corners, wrong
    // everywhere else, and worst on the two big triangles a floor is
    // made of. It reads as every shadow leaning the same way.
    //
    // And the depth has to be `PAGE_NEAR / major`, not `/ length`. Only
    // the first is projective: `depth * w` is then the constant
    // PAGE_NEAR, so the rasteriser's own divide reconstructs it exactly
    // at every fragment. The radial form is off by the ratio between
    // them — 1 straight ahead of the lamp, 1.73 at a face's corner.
    let depth = include_str!("../../../shaders/page_depth.wgsl");
    assert!(
        depth.contains("page_clip_w("),
        "the lamp draw stopped handing the rasteriser a w; its triangles are being \
         filled by linear interpolation between separately-divided corners"
    );
    assert!(
        depth.contains("PAGE_NEAR / max(face.z, PAGE_NEAR)"),
        "the depth pass stopped storing along the major axis"
    );
    assert!(
        shading.contains("PAGE_NEAR / max(major, PAGE_NEAR)"),
        "the reader stopped reconstructing depth the way the raster writes it"
    );
    assert!(
        !depth.contains("length(offset)") && !body.contains("PAGE_NEAR / max(length"),
        "a radial distance is back on one side of the comparison"
    );
}

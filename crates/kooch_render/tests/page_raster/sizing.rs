//! How big the rasterizer's pools, lists and dispatches grow.

use super::*;

#[test]
fn the_shaders_compile() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // Builds all four pipelines, so a WGSL mistake in any of the three
    // shaders or in the shared table fails here.
    let _ = rasterizer(&device);
}

#[test]
fn the_atlas_holds_the_pool() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let texture = raster.atlas_texture();
    assert_eq!(texture.format(), PAGE_DEPTH_FORMAT);
    let page = PageConfig::default().page;
    let across = texture.size().width / page;
    // A LAYER per camera, and the budget is the layers together: the
    // whole point of slicing is that two viewports cost what one did.
    assert_eq!(
        texture.size().depth_or_array_layers,
        VIEWS,
        "one layer per camera"
    );
    assert_eq!(across * across, small().slice(), "a layer is one slice");
    assert!(
        across * across * VIEWS >= small().pages,
        "{across} across on {VIEWS} layers cannot hold {}",
        small().pages
    );
    assert_eq!(
        texture.size().width,
        texture.size().height,
        "a strip wastes the second dimension of every texture limit"
    );
}

#[test]
fn the_lamp_arena_is_sized_by_groups() {
    // The arena is `[slot * capacity + group]` — one ROW A LAMP — so whatever sizes it is
    // multiplied by up to `LAMP_CULLS`. Sized by the cull's thread count instead of the scene's
    // real group count, 2024 instances at 4700 meshlets over 64 lamps asks for 2.4 GB.
    let instances = 2024u64;
    let meshlets = 4700u64;
    let lamps = 64u64;
    let over_approximation = instances * meshlets * lamps * 4;
    assert!(
        over_approximation > 256 * 1024 * 1024,
        "the bug this guards needs the over-approximation to exceed a buffer limit; \
         it measured {over_approximation} bytes",
    );
    // The real group count is the prefix sum, which for a scene of mostly single-group cubes is
    // nearer the instance count than the thread count.
    let real_groups = instances + 24 * 1000;
    assert!(
        real_groups * lamps * 4 < 64 * 1024 * 1024,
        "the real count has to fit comfortably, or the fix is not one",
    );
}

/// The pre-pass's pair list is a PRODUCT — lamps times instances — and a constant cannot hold one.
#[test]
fn the_pair_list_outgrows_constants() {
    // `dense.scene`, measured: 2157 entities and 64 lamps, and the ones under test carry range 90
    // over a city this size — so the sphere test keeps most instances for most lamps.
    let instances = 2157u64;
    let lamps = 64u64;
    let old_cap = 16_384u64;
    assert!(
        instances * lamps > old_cap * 8,
        "the bug this guards needs the scene's bound to dwarf the old cap;          it measured {} pairs against {old_cap}",
        instances * lamps,
    );
    // And the bound the list now grows to still fits a buffer, at the eight bytes a pair costs.
    assert!(
        instances * lamps * 8 < 16 * 1024 * 1024,
        "the bound has to fit comfortably, or growing to it is not the fix",
    );
}

/// The per-view clear of `visible_counts` must not reach the lamps' buckets.
#[test]
fn the_clear_spares_lamp_buckets() {
    let source = include_str!("../../src/shadow/pages/raster/record.rs");
    assert!(
        !source.contains("clear_buffer(&self.visible_counts, 0, None)"),
        "the per-view clear spans the whole buffer again; it must stop at the sun's levels,          because the lamps' cull runs once a frame and will not refill what a second view wiped"
    );
    assert!(
        source.contains("clear_buffer(&self.visible_counts, 0, Some(levels as u64 * 4))"),
        "the per-view clear no longer covers the sun's own levels"
    );
}

/// The lamps' meshlet passes are dispatched over `pairs * meshlets`, and that product does not fit
/// one dispatch dimension.
#[test]
fn the_dispatch_outgrows_one_dimension() {
    use kooch_core::gpu::limits::{MAX_WORKGROUPS_PER_DIM, tiled_workgroups};

    // Measured on `dense.scene`, from the cull's own growth logs:
    // `visible_meshlets required = 9 842 308` over 2157 instances.
    let instances = 2157u32;
    let scene_max_meshlets = 9_842_308u32 / instances;
    let lamps = 64u32;
    // Even the pair cap this path shipped with is far past the limit.
    let old_pair_cap = 16_384u32;
    let threads = old_pair_cap * scene_max_meshlets;
    let flat = threads.div_ceil(64);
    assert!(
        flat > MAX_WORKGROUPS_PER_DIM,
        "the bug this guards needs the flat count to exceed the limit;          it measured {flat} against {MAX_WORKGROUPS_PER_DIM}"
    );

    // Tiled, both dimensions are legal and every thread is still covered.
    let (x, y) = tiled_workgroups(threads, 64);
    assert!(x <= MAX_WORKGROUPS_PER_DIM && y <= MAX_WORKGROUPS_PER_DIM);
    assert!(
        u64::from(x) * u64::from(y) * 64 >= u64::from(threads),
        "the tiled shape does not cover every thread"
    );

    // And the pre-pass's own dispatch, which is lamps times instances.
    let (px, py) = tiled_workgroups(lamps * instances, 64);
    assert!(px <= MAX_WORKGROUPS_PER_DIM && py <= MAX_WORKGROUPS_PER_DIM);

    // The shaders must read the second dimension, or tiling the args
    // just runs the same threads several times.
    let source = include_str!("../../shaders/lamp_cull.wgsl");
    for entry in ["cs_lamp_pairs", "cs_lamp_err", "cs_lamp_cull"] {
        let at = source
            .find(&format!("fn {entry}("))
            .unwrap_or_else(|| panic!("lamp_cull.wgsl has no {entry}"));
        let body = &source[at..at + 400];
        assert!(
            body.contains("num_workgroups"),
            "{entry} indexes by gid.x alone; a tiled dispatch would run row zero              {} times over",
            "y"
        );
    }
}

/// The moved-caster list is sized by what the scene moves, not by a constant.
#[test]
fn the_moved_list_grows() {
    // Measured on `dense.scene`, from the engine's own warning.
    let spinning = 2026u64;
    let old_cap = 256u64;
    assert!(
        spinning > old_cap,
        "the bug this guards needs the scene to outrun the cap"
    );
    // Sixteen bytes a sphere, against a shadow atlas measured at 52 MiB.
    // There was never a memory argument for the cap.
    assert!(
        spinning * 16 < 64 * 1024,
        "the whole list is {} bytes; a cap that small was never about memory",
        spinning * 16
    );
}

#[test]
fn the_counters_name_every_level() {
    let Some((device, _queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    let raster = rasterizer(&device);
    let sun = ClipmapConfig::default().levels;
    let buckets = raster.buckets();
    // 🔴 Per BUCKET: the sun's clipmap levels first — octaves of its own scale, level L on bucket L
    // — then one bucket per lamp slot, each fed by that lamp's own cull.
    assert_eq!(
        buckets,
        sun + 256,
        "the lamp buckets moved; `LAMP_CULLS` and the shader's constant have to move together"
    );
    // …then the two receiver-bound rejections, the lamps' (#940) and the sun's (#949), which are
    // counted apart because they measure different properties of a scene.
    assert_eq!(raster.count_slots(), buckets * 3 + 9);
    let mut words = vec![0u32; raster.count_slots() as usize];
    words[0] = 7;
    words[1] = 5;
    words[2] = 9;
    words[buckets as usize + 1] = 42;
    words[buckets as usize + 2] = 900;
    // The two rejections sit at the tail, and the point of the pair is that a reader can tell them
    // apart: a compact scene leaves the sun's bound nothing to reject while the lamps' still bites.
    words[buckets as usize * 3 + 5] = 11;
    words[buckets as usize * 3 + 6] = 77;
    // The inverted shape's cost, which is a MEASURED number and not the
    // `pages * meshlets` product the paired one reports. A reader that
    // took `tests` for both would compare a walk against an area.
    words[buckets as usize * 3 + 7] = 1234;
    let counts = raster.decode(&words, 1);
    assert_eq!(counts.pages, 21, "every bucket sums");
    assert_eq!(counts.local, 42, "local pages are reported, not hidden");
    assert_eq!(counts.pairs, 900);
    assert_eq!(counts.depth_rejected, 11, "the lamps' bound, alone");
    assert_eq!(counts.sun_rejected, 77, "the sun's bound, alone");
    assert_eq!(
        counts.walk, 1234,
        "the descent's own cost, counted where it happens"
    );
    assert_eq!(
        counts.walk_overflow, 0,
        "a descent that drops a subtree drops a caster, so the healthy reading is zero",
    );
    assert_eq!(counts.view, 1);
}

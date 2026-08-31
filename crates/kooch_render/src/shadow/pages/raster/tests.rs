use super::*;

/// What the shader says a struct measures, per WGSL's own layout
/// rules.
/// The compaction as the device sees it: the cluster records, the
/// page table, then the pass. Parsing the pass alone stopped working
/// the moment it reached for `ClusterLight`.
/// The expansion as the device sees it. Same reason as
/// `compact_source`: it reaches for `ClusterLight` — and, since the
/// inversion, for the overlap query over the page pyramid too. Anything
/// this omits is a parse error that only the device would have found.
fn expand_source() -> String {
    format!(
        "{CLUSTER_COMMON}\n{}\n{EXPAND}",
        crate::shadow::pages::pyramid::OVERLAP
    )
}

fn compact_source() -> String {
    // `shader_size` and `shader_offsets` prepend `TABLE` themselves,
    // so this adds only what the pass reaches for beyond it.
    format!("{CLUSTER_COMMON}\n{COMPACT}")
}

fn shader_size(body: &str, name: &str) -> u32 {
    let source = format!("{TABLE}\n{body}");
    let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .expect("the shader has a layout");
    for (handle, ty) in module.types.iter() {
        if ty.name.as_deref() == Some(name) {
            return layouter[handle].size;
        }
    }
    panic!("`{name}` is not declared in this shader");
}

/// Where every field of a shader struct starts, in declaration
/// order.
///
/// 🔴 The half a size check cannot see. Two structs of the same
/// size with two fields swapped measure identical and mean
/// different things — and that is not hypothetical either: a field
/// added after `paint` in the shader and before it in Rust broke
/// the page DEBUG VIEW, a feature the change had not touched.
pub fn shader_offsets(body: &str, name: &str) -> Vec<(String, u32)> {
    let source = format!("{TABLE}\n{body}");
    let module = naga::front::wgsl::parse_str(&source).expect("the shader parses");
    for (_, ty) in module.types.iter() {
        if ty.name.as_deref() != Some(name) {
            continue;
        }
        let naga::TypeInner::Struct { members, .. } = &ty.inner else {
            panic!("`{name}` is not a struct");
        };
        return members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect();
    }
    panic!("`{name}` is not declared in this shader");
}

/// 🔴 The bug class this exists for cost a frame that rendered
/// nothing but validation errors, once per frame forever.
///
/// `ExpandLevel` held a `vec3<u32>` for padding. A `vec3<u32>`
/// **aligns to 16**, so the field started at offset 16 and the
/// struct measured 32 bytes against the Rust mirror's 16. It
/// compiles. It validates. It fails at BIND time — *"bound with
/// size 16 where the shader expects 32"* — which is the one place
/// no test was looking.
///
/// A comment saying "mirrors X field for field" is not a check.
/// This is.
#[test]
fn the_uniform_mirrors_match_the_shader() {
    assert_eq!(
        shader_size(&compact_source(), "PageRaster") as usize,
        std::mem::size_of::<RasterUniform>(),
        "PageRaster",
    );
    assert_eq!(
        shader_size(&expand_source(), "ExpandLevel") as usize,
        std::mem::size_of::<ExpandLevel>(),
        "ExpandLevel",
    );
}

/// The other half: same size, wrong order.
#[test]
fn the_uniform_fields_line_up() {
    let mine = [
        ("space", std::mem::offset_of!(RasterUniform, space)),
        ("views", std::mem::offset_of!(RasterUniform, views)),
        ("pool", std::mem::offset_of!(RasterUniform, pool)),
        ("chain", std::mem::offset_of!(RasterUniform, chain)),
        ("world", std::mem::offset_of!(RasterUniform, world)),
        ("eye", std::mem::offset_of!(RasterUniform, eye)),
        ("sun", std::mem::offset_of!(RasterUniform, sun)),
        ("bias", std::mem::offset_of!(RasterUniform, bias)),
        ("layer", std::mem::offset_of!(RasterUniform, layer)),
    ];
    let theirs = shader_offsets(&compact_source(), "PageRaster");
    assert_eq!(theirs.len(), mine.len(), "field count");
    for ((name, offset), (their_name, their_offset)) in mine.iter().zip(&theirs) {
        assert_eq!(name, their_name, "field order");
        assert_eq!(*offset as u32, *their_offset, "`{name}` starts elsewhere");
    }
}

/// The three runs of counters do not overlap, and the shader that
/// writes the third one agrees with the Rust that reads it.
///
/// Per BUCKET rather than per clipmap level: the sun's levels and a
/// local light's chain levels both get one, so a run sized to the
/// clipmap alone puts the survivors inside the overflow flags.
///
/// 🔴 A counter buffer is one flat array of `u32` shared by four
/// shaders and one `copy_buffer_to_buffer`, addressed by arithmetic
/// written out twice. The first run is per level, the second is
/// filled by a copy from `visible_counts`, the third is written by
/// `count_scatter`. Getting the base of the third wrong does not
/// fail: it lands in the survivor counts, which are plausible
/// numbers, and the panel reports a comparison built on the wrong
/// half of the buffer.
///
/// This session already shipped one defect of exactly this shape —
/// `page_compact.wgsl` reading a two-word table entry with a
/// one-word stride — and it took a screen full of squares to find.
#[test]
fn the_counter_runs_do_not_overlap() {
    for buckets in [1u32, 4, 25, 40] {
        let n = buckets as usize;
        let slots = count_slots(buckets) as usize;
        // Run one: the pages per level. Run two: the survivors,
        // written by the copy at the end of `record`. Run three:
        // the scatter's cells.
        let survivors = n + 5;
        let scatter = n * 2 + 5;
        assert!(
            survivors + n <= scatter,
            "the survivor run runs into the scatter run at {buckets} buckets",
        );
        assert!(
            scatter + n <= slots,
            "the scatter run runs off the end at {buckets} buckets",
        );
    }
    // And the shader addresses the third run the same way `decode`
    // does. A comment claiming they match is not a check.
    assert!(
        EXPAND.contains("page_counts[buckets * 2u + 5u + level]"),
        "`count_scatter` no longer writes where `decode` reads",
    );
}

/// Every buffer this pass copies OUT of declares that it can be.
///
/// 🔴 Written after shipping a `copy_buffer_to_buffer` whose source
/// lacked `COPY_SRC`. It compiles; it passes every test that plants
/// words and decodes them; and it fails at RUNTIME, once per view
/// per frame forever, with the shadow pass producing nothing. The
/// tests around it never ran `record`, which is where the copy is.
///
/// A source check rather than a GPU one, because the question is
/// about a declaration and answering it on a device would mean
/// building a whole frame to observe one flag.
#[test]
fn every_copied_buffer_can_be_copied_from() {
    let source = include_str!("../raster.rs");
    // The buffers this pass copies out of, by the field name the
    // copy uses.
    let mut copied = copied_fields(source);
    assert!(
        !copied.is_empty(),
        "the scan found no copies at all; it has stopped matching the source"
    );
    // 🔴 And every buffer this module hands OUT. The only reason to
    // expose a GPU buffer is for something to read it back, and a
    // reader outside this file is a copy this scan cannot see — that
    // is exactly how `page_list` shipped without the flag, failing
    // only sometimes, because wgpu reports the error whenever it
    // gets round to it.
    let mut labels: Vec<String> = copied
        .iter()
        .map(|field| format!("page_raster_{field}"))
        .collect();
    // The label a buffer carries is not always its field name.
    labels.extend(
        [
            "page_raster_list",
            "page_raster_counts",
            "page_raster_draw_args",
        ]
        .into_iter()
        .map(String::from),
    );
    for label in labels {
        let at = source
            .find(&format!("Some(\"{label}\")"))
            .unwrap_or_else(|| panic!("{label} has no buffer descriptor"));
        let body = &source[at..at + 400];
        let usage = body
            .find("usage:")
            .map(|i| &body[i..body[i..].find(',').map(|j| i + j).unwrap_or(body.len())])
            .unwrap_or("");
        assert!(
            usage.contains("COPY_SRC"),
            "{label} is copied out of and its usage is `{usage}`"
        );
    }
}

/// The field each `copy_buffer_to_buffer` reads FROM — the first
/// `&self.<field>` after the call, whatever the formatter did to the
/// whitespace between them.
fn copied_fields(source: &str) -> std::collections::BTreeSet<&str> {
    let mut out = std::collections::BTreeSet::new();
    let mut rest = source;
    while let Some(at) = rest.find("copy_buffer_to_buffer(") {
        let tail = &rest[at + "copy_buffer_to_buffer(".len()..];
        // 🔴 The FIRST argument only. The destination is the third,
        // and it needs COPY_DST rather than COPY_SRC — a scan that
        // took whichever `&self.` came first would demand the wrong
        // flag of the wrong buffer.
        let first = &tail[..tail.find(',').unwrap_or(0)];
        if let Some(field) = first.trim().strip_prefix("&self.") {
            let end = field
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(field.len());
            if end > 0 {
                out.insert(&field[..end]);
            }
        }
        rest = tail;
    }
    out
}

/// A camera's slice is its own, and nothing else's.
#[test]
fn a_view_owns_its_slice() {
    for views in 1..=4u32 {
        let pool = PoolConfig {
            pages: 2048,
            views,
            row_cap: u32::MAX,
        };
        let slice = pool.slice();
        assert!(slice > 0);
        assert_eq!(pool.total(), slice * views, "every view gets one");
        let bases: Vec<u32> = (0..views).map(|v| pool.base(v)).collect();
        for pair in bases.windows(2) {
            assert_eq!(pair[1] - pair[0], slice, "the slices do not overlap");
        }
        assert_eq!(
            pool.base(views - 1) + slice,
            pool.total(),
            "the last slice ends where the pool does"
        );
    }
}

/// The pool is a budget, not a per-camera one. Splitting it must not
/// multiply what it costs.
#[test]
fn slicing_does_not_grow_the_atlas() {
    let config = PageConfig::default();
    let one = PoolConfig {
        pages: 2048,
        views: 1,
        row_cap: u32::MAX,
    }
    .atlas_bytes(config);
    let two = PoolConfig {
        pages: 2048,
        views: 2,
        row_cap: u32::MAX,
    }
    .atlas_bytes(config);
    assert!(
        two <= one,
        "two cameras cost {two} bytes against one camera's {one}"
    );
}

/// A sub-page step along the sun keeps every level's content.
///
/// 🔴 The defect this file exists to stop coming back. A page's depth
/// was measured from the raw camera, so `write_gens` had to hash the raw
/// `eye.dot(sun)` to stay honest — and that turned over EVERY level's
/// stamp on any movement at all. The sibling test is
/// `a_still_suns_page_caches`: a camera that never moves cached fine,
/// which is why this went unnoticed. Measured on the OneXFly, the depth
/// draw cost 0.064 ms when the cache held and 29.7 ms when it did not,
/// and it did not hold (#948).
///
/// One millimetre, which is a hundredth of the FINEST level's page.
#[test]
fn a_millimetre_along_the_sun_keeps_the_cache() {
    let clipmap = ClipmapConfig::default();
    let side = PageConfig::default().side(0) as f32;
    let sun = Vec3::new(0.3, -1.0, 0.2);
    let eye = Vec3::new(12.0, 3.0, -7.0);
    let step = sun.normalize() * 0.001;

    let before = sun_gens(clipmap, side, 7, eye, sun);
    let after = sun_gens(clipmap, side, 7, eye + step, sun);

    assert_eq!(
        before,
        after,
        "a millimetre along the sun voided {} of {} levels",
        before.iter().zip(&after).filter(|(a, b)| a != b).count(),
        before.len()
    );
}

/// Crossing a level's page along the sun DOES void that level — the
/// stamp is a cache gate, not a promise that depth never goes stale.
///
/// The finest level's page is 1 cm wide, so a metre crosses it and every
/// level below the one whose page is a metre across. The coarsest, whose
/// pages are hundreds of metres, must survive: that is the whole point
/// of snapping per level rather than globally.
#[test]
fn a_metre_voids_the_fine_levels_only() {
    let clipmap = ClipmapConfig::default();
    let side = PageConfig::default().side(0) as f32;
    let sun = Vec3::NEG_Y;
    let eye = Vec3::new(0.0, 40.0, 0.0);

    let before = sun_gens(clipmap, side, 7, eye, sun);
    let after = sun_gens(clipmap, side, 7, eye + Vec3::new(0.0, -1.0, 0.0), sun);

    assert_ne!(
        before[0], after[0],
        "the finest level ignored a whole metre"
    );
    let last = before.len() - 1;
    assert_eq!(
        before[last], after[last],
        "the coarsest level, whose pages are hundreds of metres, redrew for one"
    );
}

/// Every page of a level's window has to sit inside the box its cull
/// runs against.
///
/// # 🔴 The failure is a lit band that crawls with the camera
///
/// `sun_window` places a level's window on the SNAPPED page grid and
/// the cull box used to be centred on the camera, which is not on it.
/// The two are the same size and offset by however far the camera sits
/// into its own page, so the window's lowest band — up to a whole page
/// wide, and 655 m at the coarsest level — lay outside the box.
///
/// Geometry there is culled. The pages there are marked by their
/// receivers and drawn anyway, empty, and an empty page stores far
/// depth under reversed-Z: every reader over it answers "nothing
/// occludes here". A lit band at each level's edge, which is a ring at
/// a fixed distance from the camera, and the offset changes as the
/// camera moves so the ring crawls.
///
/// The window is computed here from `sun_window`'s own formula rather
/// than from the function under test, or this would only prove that a
/// number equals itself.
#[test]
fn the_cull_box_covers_the_window() {
    let clipmap = ClipmapConfig::default();
    let side = PageConfig::default().side(0);
    let s = side as f32;
    let sun = Vec3::new(0.3, -1.0, 0.2);
    let (right, up, _) = sun_frame(sun);

    for level in [0u32, 1, 7, 12, clipmap.levels - 1] {
        let width = clipmap.base * (level as f32).exp2() / s;
        // A whole page of offsets: the defect IS the fraction of a page
        // the camera sits into, so an eye on the grid lines would pass
        // either way.
        for step in 0..8 {
            let frac = step as f32 / 8.0;
            let eye = Vec3::new(frac * width, 3.0, frac * width * 0.5);
            let clip = level_clip(clipmap, side, level, eye, sun);

            let plane = glam::Vec2::new(eye.dot(right), eye.dot(up));
            let low = (plane / width).floor() - glam::Vec2::splat((s * 0.5).floor());
            for corner in [
                low,
                low + glam::Vec2::new(s, 0.0),
                low + glam::Vec2::new(0.0, s),
                low + glam::Vec2::splat(s),
            ] {
                let world = right * (corner.x * width) + up * (corner.y * width);
                let ndc = clip * world.extend(1.0);
                assert!(
                    ndc.x.abs() <= 1.0 + 1e-3 && ndc.y.abs() <= 1.0 + 1e-3,
                    "level {level}, camera {frac} of a page in: the window corner \
                     {corner:?} lands at ({}, {}) — outside the cull box, so casters \
                     there are dropped while their pages are drawn empty and read lit",
                    ndc.x,
                    ndc.y,
                );
            }
        }
    }
}

use super::*;

#[test]
fn substitution_leaves_no_placeholder_behind() {
    let src = inti_pbr_shader(5);
    assert!(
        !src.contains(GROUP_PLACEHOLDER),
        "a surviving placeholder is a shader that fails to parse at \
             pipeline creation, which is a runtime panic and not a test failure",
    );
    assert!(src.contains("@group(5) @binding(0)"));
    assert!(src.contains("@group(5) @binding(1)"));
    // The shadow bindings substitute too. The fourth is the one
    // worth pinning: it was written off as impossible on a
    // bind-group budget that is spent on *groups*, not on bindings
    // inside one, and if it silently disappears the blocker search
    // has nothing to sample and PCSS quietly becomes PCF again.
    assert!(src.contains("@group(5) @binding(2)"));
    assert!(src.contains("@group(5) @binding(3)"));
    assert!(src.contains("@group(5) @binding(4)"));
}

#[test]
fn the_template_is_not_valid_wgsl_on_its_own() {
    // Guards the reverse mistake: someone including the template
    // directly instead of calling the function would get a parse
    // error at pipeline creation. Better to state the contract.
    assert!(INTI_PBR_TEMPLATE.contains(GROUP_PLACEHOLDER));
}

/// The model calls `inti_contact_shadow` and never defines it, so
/// alone it is half a shader. That is deliberate — see
/// [`INTI_CONTACT_SHADOW_STUB`] — and this pins that the missing
/// half is exactly the one named, rather than something else having
/// gone missing.
#[test]
fn the_model_needs_a_contact_shadow_implementation_concatenated() {
    assert!(inti_pbr_shader(0).contains("inti_contact_shadow("));
    assert!(INTI_CONTACT_SHADOW_STUB.contains("fn inti_contact_shadow("));
}

#[test]
fn shading_model_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(&format!(
        "{}\n{}",
        INTI_CONTACT_SHADOW_STUB,
        inti_pbr_shader(0)
    ))
    .expect("inti_pbr.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("inti_pbr.wgsl should validate");
}

/// 🔴 The debug views had nothing validating them.
///
/// `inti_pbr.wgsl` is parsed above and `inti_debug.wgsl` was not, so a
/// typo in a view compiled for the first time when somebody opened it in
/// the editor — a shader panic on a dropdown selection, in a file whose
/// whole purpose is to be reached rarely. This concatenates the two the
/// way the editor's pipeline does and validates the result.
#[test]
fn the_debug_views_parse_and_validate() {
    let module = naga::front::wgsl::parse_str(&format!(
        "{}\n{}\n{}",
        INTI_CONTACT_SHADOW_STUB,
        inti_pbr_shader(0),
        inti_debug_shader(),
    ))
    .expect("inti_debug.wgsl should parse when concatenated after the model");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("inti_debug.wgsl should validate");
}

/// The stub and the real views must present the same call sites, or the
/// production pipeline stops compiling the moment a view is added.
///
/// The names are derived from the stub rather than restated, so a
/// function renamed in one file and not the other fails here instead of
/// in whichever build happens to be compiled next.
#[test]
fn the_stub_matches_the_views_it_replaces() {
    let views = inti_debug_shader();
    let mut found = 0;
    for line in INTI_DEBUG_STUB.lines() {
        let Some(signature) = line.strip_prefix("fn ") else {
            continue;
        };
        let name = signature.split('(').next().unwrap_or_default();
        assert!(
            views.contains(&format!("fn {name}(")),
            "the stub declares `{name}` and inti_debug.wgsl does not",
        );
        found += 1;
    }
    assert!(found >= 2, "the stub should declare both call sites");
}

/// 🔴 Zero means "never skip", and it is the default. A project that
/// never heard of #821 has to render exactly what it rendered before.
///
/// Reads the uniform rather than `SpecularFloor::default()`, which
/// consults the environment — a test that asserted on that would fail
/// for whoever happened to have the variable set while measuring.
#[test]
fn the_default_floor_keeps_every_specular() {
    assert_eq!(IntiFrame::default().specular_floor, 0.0);
}

/// A negative floor would mean nothing, and zero already means never.
#[test]
fn a_negative_floor_is_clamped() {
    assert_eq!(
        IntiFrame::default()
            .with_specular_floor(-5.0)
            .specular_floor,
        0.0
    );
    assert_eq!(
        IntiFrame::default()
            .with_specular_floor(120.0)
            .specular_floor,
        120.0
    );
}

/// The shader has to read the uniform for the control to do anything —
/// and to compare against the irradiance it already computed, not
/// against something it recomputes differently.
#[test]
fn the_shader_gates_on_the_floor() {
    let source = inti_pbr_shader(0);
    assert!(source.contains("inti.specular_floor"));
    assert!(source.contains("reach >= inti.specular_floor"));
}

/// `KOOCH_LIGHT_LIMIT`'s default has to be "every light", or every
/// capture taken without the variable would silently be measuring a
/// truncated scene (#824 follow-up).
#[test]
fn the_light_limit_defaults_to_all() {
    assert_eq!(IntiFrame::default().light_limit, 0);
    assert_eq!(LightLimit::default().0, 0);
}

#[test]
fn the_light_limit_reaches_the_uniform() {
    assert_eq!(IntiFrame::default().with_light_limit(3).light_limit, 3);
}

/// 🔴 Both shading paths must honour the cap. If only one did, the A/B
/// between them would be measuring the cap rather than the paths.
#[test]
fn both_paths_honour_the_light_limit() {
    let inti = crate::inti_pbr_shader(5);
    assert!(inti.contains("inti.light_limit"));
    assert!(
        kooch_render_compute_body().contains("inti.light_limit"),
        "the compute shading path ignores KOOCH_LIGHT_LIMIT",
    );
}

/// The compute body lives in `kooch_render`, which depends on this
/// crate — so it is read from the file rather than imported.
fn kooch_render_compute_body() -> &'static str {
    include_str!("../../kooch_render/shaders/material_pbr_compute.wgsl")
}

/// 🔴 What the old ranking got wrong. Sorted by distance, the dim lamp
/// two metres away outranked the floodlight across the room, and the
/// cubes went to whatever the camera happened to be standing next to.
#[test]
fn a_floodlight_outranks_a_nearby_candle() {
    let eye = glam::Vec3::ZERO;
    let candle = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -2.0), 2.0, 10.0, eye);
    let flood = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -20.0), 30.0, 4000.0, eye);
    assert!(flood > candle, "candle {candle}, floodlight {flood}");
}

/// Distance still decides between equals — the half of the old rule
/// that was right.
#[test]
fn the_nearer_of_two_equal_lamps_wins() {
    let eye = glam::Vec3::ZERO;
    let near = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -10.0), 5.0, 100.0, eye);
    let far = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -40.0), 5.0, 100.0, eye);
    assert!(near > far);
}

/// Inside the sphere the light is all around the viewer, so walking
/// closer to its centre cannot make its shadow cover more screen. Left
/// unclamped, `range / distance` runs to infinity at the centre and one
/// lamp owns every cube the moment the player stands in it.
#[test]
fn being_inside_the_sphere_saturates() {
    let eye = glam::Vec3::ZERO;
    let inside = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -1.0), 5.0, 100.0, eye);
    let deeper = point_shadow_importance(glam::Vec3::new(0.0, 0.0, -0.01), 5.0, 100.0, eye);
    assert_eq!(inside, deeper);
    assert_eq!(inside, 100.0);
}

/// A light at the camera's exact position is authorable and must not
/// produce a NaN — `sort_by` with an inconsistent comparator is allowed
/// to panic, so one bad lamp would take the frame down.
#[test]
fn a_lamp_on_the_camera_is_finite() {
    let eye = glam::Vec3::new(3.0, 1.0, -2.0);
    let on_top = point_shadow_importance(eye, 5.0, 100.0, eye);
    assert!(on_top.is_finite(), "got {on_top}");
}

#[test]
fn the_page_bindings_substitute() {
    let src = inti_pbr_shader(5);
    // The virtual shadow map's three (#866). Worth pinning for the same
    // reason the shadow bindings above are: they were added to a group
    // already believed full, and a group index that failed to
    // substitute is a runtime panic rather than a test failure.
    // Binding 9 held the page hash's key array and is retired with it.
    assert!(src.contains("@group(5) @binding(8)"));
    assert!(src.contains("@group(5) @binding(10)"));
    assert!(src.contains("@group(5) @binding(11)"));
    assert!(
        !src.contains("@group(5) @binding(9)"),
        "binding 9 came back; the flat table has no key array"
    );
}

#[test]
fn the_reader_shares_the_page_arithmetic() {
    let src = inti_pbr_shader(0);
    // 🔴 The structural guarantee against the drift that matters. Four
    // passes in another crate WRITE this table; this one reads it. A
    // reader that reimplemented the id arithmetic would be free to
    // disagree with the writer by one level, and the symptom is a
    // shadow that disappears rather than one that looks wrong.
    assert!(src.contains("fn page_decode("));
    assert!(src.contains("fn sun_page_rect("));
    for helper in [
        "local_level_base(",
        "local_level_floor(",
        "PAGE_ABSENT",
        "PAGE_MISS",
    ] {
        let uses = src.matches(helper).count();
        assert!(
            uses >= 2,
            "`{helper}` is declared but never used by the reader",
        );
    }
    // And the hash stays gone: a probe loop growing back is the 10.4 ms
    // this table was flattened to kill.
    assert!(
        !src.contains("page_probe("),
        "the reader grew a probe loop back"
    );
}

#[test]
fn the_atlas_is_never_hardware_filtered() {
    let src = inti_pbr_shader(0);
    // A filter cannot cross a page border: the neighbouring texels
    // belong to another clipmap level, so a sampler would blend a
    // shadow with one from somewhere else. Taps are loads, clamped
    // inside the page.
    assert!(src.contains("textureLoad(inti_page_atlas"));
    assert!(
        !src.contains("textureSample(inti_page_atlas"),
        "a sampler on the page atlas reads across page borders",
    );
}

/// The page age view says WHITE for a page with no content, and not for
/// a page requested this frame.
///
/// 🔴 A grep, and it exists because the difference is invisible: the
/// view read word 1 — the frame a page was last REQUESTED — and the
/// marking rewrites that every frame it asks for a page. Everything on
/// screen is asked for every frame, so the comparison was zero for
/// every pixel and the view returned white before reaching its own hue
/// or fade. It painted the whole screen, always, and looked like a view
/// of something.
///
/// Word 3 is the content generation: zero means the page was claimed
/// and never drawn into, which is a hole a shadow can actually have.
#[test]
fn the_age_view_paints_missing_content() {
    let source = crate::inti_debug_shader();
    let start = source
        .find("fn inti_page_age_debug(")
        .expect("the view is in the shader");
    let end = source[start..]
        .find("\nfn ")
        .map(|o| o + start + 1)
        .unwrap_or(source.len());
    let body = &source[start..end];
    assert!(
        body.contains("inti_page_slots[page * PAGE_CELL + 3u] == 0u"),
        "white has to mean NO CONTENT — word 3 — not `frame - age`",
    );
    let white = body
        .find("return vec3<f32>(1.0);")
        .expect("the view still paints white somewhere");
    let requested = body.find("inti_pages.views.w - age").unwrap_or(usize::MAX);
    assert!(
        white < requested,
        "the content test has to come FIRST, or the request age shadows it again",
    );
}

use super::*;

/// Each transparent material gets one layer, in the order they came, and opaque ones none.
#[test]
fn each_material_bakes_once() {
    let chosen = layers_for(&[4, 2, 4, 7, 2], |slot| slot != 7);
    assert_eq!(chosen, [4, 2]);
}

/// Past the atlas's layers, the rest cast solid.
#[test]
fn the_atlas_caps_the_layers() {
    let slots: Vec<u32> = (0..100).collect();
    assert_eq!(layers_for(&slots, |_| true).len() as u32, ALPHA_LAYERS);
}

/// The bake frame and the sampling half compile around a transparent surface.
#[test]
fn the_alpha_shaders_compile() {
    let shader = crate::material::Shader::parse(
        "// kind: transparent\nfn surface(input: SurfaceInput) -> SurfaceOutput {\n    var out: SurfaceOutput;\n    out.normal = normalize(input.world_normal);\n    out.alpha = input.uv.x;\n    return out;\n}",
    )
    .unwrap();
    let bake = [
        MATERIAL_SURFACE_PRELUDE,
        &shader.params_wgsl(),
        &shader.source,
        BAKE_FRAME,
    ]
    .join("\n");
    let sample = format!(
        "{}\n@fragment fn fs(@builtin(position) p: vec4<f32>) {{ if !shadow_alpha_keeps(0u, p.xy, p.xy) {{ discard; }} }}",
        shadow_alpha_shader(3)
    );
    for (name, source) in [("bake", bake), ("sample", sample)] {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
    }
}

/// A masked surface's bake compiles: it writes the cut, not the opacity (#452).
#[test]
fn a_masked_bake_compiles() {
    let shader = crate::material::Shader::parse(
        "fn surface(input: SurfaceInput) -> SurfaceOutput {\n    var out: SurfaceOutput;\n    out.alpha = input.uv.x;\n    out.alpha_clip = 0.5;\n    return out;\n}",
    )
    .unwrap();
    let bake = [
        MATERIAL_SURFACE_PRELUDE,
        &shader.params_wgsl(),
        &shader.source,
        BAKE_FRAME,
    ]
    .join("\n");
    let module = naga::front::wgsl::parse_str(&bake)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&bake)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&bake)));
}

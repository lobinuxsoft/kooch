use super::*;

/// A surface that cuts where its uv's x falls below a half.
const CUT: &str = "fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.base_color = vec3<f32>(1.0);
    out.normal = input.world_normal;
    out.alpha = input.uv.x;
    out.alpha_clip = 0.5;
    return out;
}";

fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| e.emit_to_string(source))
}

/// Both targets' rasters compile around a masked surface.
#[test]
fn a_masked_raster_compiles() {
    let shader = crate::material::Shader::parse(CUT).unwrap();
    assert!(shader.masked());
    for target in [MaskedTarget::R64, MaskedTarget::R32] {
        let source = compose_masked_shader(target, &shader.params_wgsl(), &shader.source);
        validate(&source).unwrap_or_else(|why| panic!("{target:?}: {why}"));
    }
    crate::meshlet::validate_surface(&shader.params_wgsl(), &shader.source).unwrap();
}

#[test]
fn the_bins_compile() {
    validate(layouts::BINS_SHADER).unwrap();
}

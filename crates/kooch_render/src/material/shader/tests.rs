use super::*;

#[test]
fn a_headerless_file_is_a_surface() {
    let shader = Shader::parse("fn surface() {}").unwrap();
    assert_eq!(shader.kind, ShaderKind::Surface);
}

#[test]
fn the_kind_line_is_read() {
    let shader = Shader::parse("// The red one.\n//   kind: surface\nfn surface() {}").unwrap();
    assert_eq!(shader.kind, ShaderKind::Surface);
}

#[test]
fn an_unknown_kind_fails() {
    assert!(matches!(
        Shader::parse("// kind: particles\n"),
        Err(ShaderParseError::Kind(name)) if name == "particles"
    ));
}

/// A `kind:` inside the body is code, not a header.
#[test]
fn only_leading_comments_count() {
    let shader = Shader::parse("fn surface() {}\n// kind: particles\n").unwrap();
    assert_eq!(shader.kind, ShaderKind::Surface);
}

const TOON: &str = "\
struct SurfaceParams {
    tint: vec4<f32>,   // @color
    strength: f32,     // @range(0, 4)
    uv: vec2f,
}
var albedo: texture_2d<f32>;
var mask: texture_2d<f32>;   // @default(black)
const SURFACE_DEFAULTS = SurfaceParams(vec4(1.0, 0.5, 0.2, 1.0), 2.0, vec2(3.0));
";

#[test]
fn params_take_offsets_in_order() {
    let shader = Shader::parse(TOON).unwrap();
    let offsets: Vec<(&str, u32)> = shader
        .params
        .iter()
        .map(|p| (p.name.as_str(), p.offset))
        .collect();
    assert_eq!(
        offsets,
        [
            ("tint", 0),
            ("strength", 4),
            ("uv", 5),
            ("albedo", 0),
            ("mask", 1)
        ]
    );
    assert_eq!(shader.params[0].kind, ParamKind::Color);
    assert_eq!(shader.params[1].range, Some([0.0, 4.0]));
    assert_eq!(shader.params[2].kind, ParamKind::Vec2);
    assert_eq!(shader.params[4].texture, TextureDefault::Black);
    assert_eq!(shader.params[0].default, [1.0, 0.5, 0.2, 1.0]);
    assert_eq!(shader.params[1].default[0], 2.0);
    assert_eq!(&shader.params[2].default[..2], &[3.0, 3.0]);
}

/// A constant spread over lines, with a `vec3(0.25)` splat, is still read.
#[test]
fn defaults_span_lines() {
    let shader = Shader::parse(
        "struct SurfaceParams { c: vec3<f32>, k: f32 }\n\
         const SURFACE_DEFAULTS = SurfaceParams(\n    vec3(0.25),\n    1,\n);",
    )
    .unwrap();
    assert_eq!(shader.params[0].default, [0.25, 0.25, 0.25, 0.0]);
    assert_eq!(shader.params[1].default[0], 1.0);
}

/// Starting values are code: a hint cannot stand in for them.
#[test]
fn a_default_hint_on_a_member_fails() {
    assert!(Shader::parse("struct SurfaceParams { a: f32 } // @default(1)").is_err());
}

#[test]
fn a_broken_constant_names_its_line() {
    let error = Shader::parse(
        "struct SurfaceParams { a: f32 }\nconst SURFACE_DEFAULTS = SurfaceParams(vec2(1.0));",
    )
    .unwrap_err();
    assert!(
        error.to_string().starts_with("line 2: SURFACE_DEFAULTS"),
        "{error}"
    );
}

/// Without hints a parameter still exists: zero, unbounded, a vector.
#[test]
fn hints_are_optional() {
    let shader = Shader::parse("struct SurfaceParams { tint: vec4<f32> }").unwrap();
    assert_eq!(shader.params[0].kind, ParamKind::Vec4);
    assert_eq!(shader.params[0].default, [0.0; 4]);
}

/// The engine fills in the bindings on the author's own lines, so line numbers do not move.
#[test]
fn textures_get_bindings_in_place() {
    let shader = Shader::parse(TOON).unwrap();
    let lines: Vec<&str> = shader.source.lines().collect();
    assert_eq!(lines.len(), TOON.lines().count());
    assert!(
        lines[5].starts_with("@group(4) @binding(0) var albedo"),
        "{}",
        lines[5]
    );
    assert!(
        lines[6].starts_with("@group(4) @binding(1) var mask"),
        "{}",
        lines[6]
    );
}

#[test]
fn a_bad_param_names_its_line() {
    let error =
        Shader::parse("// kind: surface\nstruct SurfaceParams {\n  x: u32,\n}").unwrap_err();
    assert!(error.to_string().starts_with("line 3: "), "{error}");
}

#[test]
fn color_needs_a_vec4() {
    assert!(Shader::parse("struct SurfaceParams { c: vec3<f32> } // @color").is_err());
}

#[test]
fn the_scalar_budget_holds() {
    // Sixteen four-wide members fill the budget exactly; the seventeenth is refused, on its own line.
    let fields = (0..17)
        .map(|i| format!("  c{i}: vec4<f32>,\n"))
        .collect::<String>();
    let source = format!("struct SurfaceParams {{\n{fields}}}");
    assert!(matches!(
        Shader::parse(&source),
        Err(ShaderParseError::Param { line: 18, .. })
    ));
}

#[test]
fn the_texture_budget_holds() {
    let source = (0..5)
        .map(|i| format!("var t{i}: texture_2d<f32>;\n"))
        .collect::<String>();
    assert!(matches!(
        Shader::parse(&source),
        Err(ShaderParseError::Param { line: 5, .. })
    ));
}

#[test]
fn a_duplicate_param_fails() {
    assert!(Shader::parse("struct SurfaceParams { a: f32 }\nvar a: texture_2d<f32>;").is_err());
}

/// Bindings are the engine's: one written by hand could collide with the layout.
#[test]
fn a_surface_cannot_bind() {
    let error = Shader::parse("@group(4) @binding(9) var extra: texture_2d<f32>;").unwrap_err();
    assert!(error.to_string().starts_with("line 1: "), "{error}");
}

/// The Inspector filters its picker by the name the `.meta` records, which is the Rust type's.
#[test]
fn the_type_name_is_the_types() {
    assert_eq!(SHADER_TYPE_NAME, std::any::type_name::<Shader>());
}

/// The engine's surface declares its three maps as textures and no scalars.
#[test]
fn the_default_surface_parses() {
    let shader = Shader::default_surface();
    assert_eq!(shader.kind, ShaderKind::Surface);
    let names: Vec<&str> = shader.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["albedo_tex", "normal_tex", "metal_rough_tex"]);
    assert_eq!(shader.params[1].texture, TextureDefault::Normal);
}

/// 🔴 A `/* */` block is a comment, not code: the graph's own block lives in one (#1159), and what
/// it holds must not read as a declaration.
#[test]
fn block_comments_are_not_code() {
    let shader = Shader::parse(
        "/*KOOCH_GRAPH;var mask: texture_2d<f32>; @group(4) @binding(9)*/\n\
         struct SurfaceParams { a: f32 }\n\
         var albedo: texture_2d<f32>; /* the only one */",
    )
    .unwrap();
    let names: Vec<&str> = shader.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["a", "albedo"]);
}

/// A block that spans lines takes the lines with it.
#[test]
fn a_block_can_span_lines() {
    let shader = Shader::parse(
        "/*\nvar hidden: texture_2d<f32>;\nstruct SurfaceParams { b: f32 }\n*/\nvar real: texture_2d<f32>;",
    )
    .unwrap();
    let names: Vec<&str> = shader.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["real"]);
}

/// A whole number is an `f32` hinted `@int`, and takes a range like a float does (#1170).
#[test]
fn an_int_param_has_a_range() {
    let shader =
        Shader::parse("struct SurfaceParams {\n    sides: f32,   // @int @range(3, 12)\n}\n")
            .unwrap();
    assert_eq!(shader.params[0].kind, ParamKind::Int);
    assert_eq!(shader.params[0].range, Some([3.0, 12.0]));
    assert_eq!(shader.params[0].kind.width(), 1);
}

#[test]
fn an_int_needs_a_float() {
    assert!(Shader::parse("struct SurfaceParams {\n    n: vec2<f32>,   // @int\n}\n").is_err());
}

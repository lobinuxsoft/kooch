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

#[test]
fn params_take_offsets_in_order() {
    let shader = Shader::parse(
        "// param tint: color = (1, 0.5, 0.2, 1)\n\
         // param strength: float = 2 range(0, 4)\n\
         // param albedo: texture\n\
         // param uv: vec2 = (1, 1)\n\
         // param mask: texture = black\n",
    )
    .unwrap();
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
            ("albedo", 0),
            ("uv", 5),
            ("mask", 1)
        ]
    );
    assert_eq!(shader.params[1].range, Some([0.0, 4.0]));
    assert_eq!(shader.params[4].texture, TextureDefault::Black);
}

/// One number fills every component, as `vec3<f32>(1.0)` does.
#[test]
fn a_single_default_splats() {
    let shader = Shader::parse("// param c: vec3 = 0.25").unwrap();
    assert_eq!(shader.params[0].default, [0.25, 0.25, 0.25, 0.0]);
}

#[test]
fn a_bad_param_names_its_line() {
    let error = Shader::parse("// kind: surface\n// param 2x: float").unwrap_err();
    assert!(error.to_string().starts_with("line 2: "), "{error}");
}

#[test]
fn the_scalar_budget_holds() {
    let header = (0..5)
        .map(|i| format!("// param c{i}: vec4\n"))
        .collect::<String>();
    assert!(matches!(
        Shader::parse(&header),
        Err(ShaderParseError::Param { line: 5, .. })
    ));
}

#[test]
fn the_texture_budget_holds() {
    let header = (0..5)
        .map(|i| format!("// param t{i}: texture\n"))
        .collect::<String>();
    assert!(matches!(
        Shader::parse(&header),
        Err(ShaderParseError::Param { line: 5, .. })
    ));
}

#[test]
fn a_duplicate_param_fails() {
    assert!(Shader::parse("// param a: float\n// param a: vec2").is_err());
}

/// The Inspector filters its picker by the name the `.meta` records, which is the Rust type's.
#[test]
fn the_type_name_is_the_types() {
    assert_eq!(SHADER_TYPE_NAME, std::any::type_name::<Shader>());
}

#[test]
fn the_default_surface_parses() {
    let shader = Shader::parse(crate::meshlet::DEFAULT_SURFACE_SHADER).unwrap();
    assert_eq!(shader.kind, ShaderKind::Surface);
}

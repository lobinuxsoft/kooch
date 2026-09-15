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

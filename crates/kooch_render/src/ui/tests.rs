/// The shader parses and validates: a broken UI shader is otherwise found on the first frame.
#[test]
fn the_ui_shader_validates() {
    let module = naga::front::wgsl::parse_str(include_str!("../../shaders/ui.wgsl")).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

/// The recipes the engine ships, composed as the pass composes them.
#[test]
fn the_recipes_validate() {
    for source in [
        include_str!("../../../../assets/shaders/ui/ring.shader"),
        include_str!("../../../../assets/shaders/ui/curved_bar.shader"),
    ] {
        let shader = crate::material::Shader::parse(source).unwrap();
        super::validate_ui(&shader).unwrap();
    }
}

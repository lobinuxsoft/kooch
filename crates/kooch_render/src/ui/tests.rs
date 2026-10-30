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

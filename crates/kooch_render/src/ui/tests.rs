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

/// The recipes the engine ships, flattened and composed as the UI's loader and the pass do.
#[test]
fn the_recipes_validate() {
    use kooch_core::shader_uses::{flatten, used_files};
    for source in [
        include_str!("../../../../assets/shaders/ui/ring.wgsl"),
        include_str!("../../../../assets/shaders/ui/curved_bar.wgsl"),
        include_str!("../../../../assets/shaders/ui/bezier_bar.wgsl"),
    ] {
        let mut read = |_: &std::path::Path| Err::<Vec<u8>, String>("no file".to_owned());
        let used = used_files(std::path::Path::new("a.wgsl"), source, &mut read).unwrap();
        let shader = crate::material::Shader::parse(&flatten(source, &used)).unwrap();
        super::validate_ui(&shader).unwrap();
    }
}

/// The ordering the frame loop exists to get right, pinned against the source itself.
#[test]
fn the_scene_is_submitted_before_the_image_is_asked_for() {
    let source = include_str!("mod.rs");

    let scene = source
        .find("render_with_assets_primary(")
        .expect("the frame loop still renders the scene");
    let acquire = source
        .find("get_current_texture()")
        .expect("the frame loop still acquires a swapchain image");

    assert!(
        scene < acquire,
        "the swapchain image is acquired before the scene is submitted: the \
         meshlet stage draws into its own textures and needs no surface, so \
         acquiring first makes the CPU wait out the compositor before \
         recording work the compositor has nothing to do with",
    );
}

/// The counterpart, so the test above cannot pass by accident on a file
/// that stopped acquiring or stopped rendering.
#[test]
fn the_frame_loop_still_does_both() {
    let source = include_str!("mod.rs");
    assert_eq!(
        source.matches("get_current_texture()").count(),
        1,
        "one acquire per frame, and one place to keep after the scene",
    );
    assert_eq!(
        source.matches("render_with_assets_primary(").count(),
        1,
        "one scene render per frame",
    );
}

/// 🔴 Absent means "no opinion", and the system must not invent one.
#[test]
fn no_presentation_means_no_change() {
    let mut resources = kooch_core::resource::Resources::new();
    super::apply_presentation_system(&mut resources);
    assert!(resources.get::<crate::quality::Presentation>().is_none());
}

/// `KOOCH_PRESENT_MODE` outranks the settings asset.
mod present_precedence {
    use super::super::wanted_vsync;

    #[test]
    fn the_asset_decides_when_nobody_overrides() {
        assert!(wanted_vsync(true, None));
        assert!(!wanted_vsync(false, None));
    }

    #[test]
    fn novsync_beats_an_asset_asking_for_it() {
        // The regression: the variable was read at surface creation and
        // then undone here on the first frame, so a measurement run
        // reported the vblank as work.
        assert!(!wanted_vsync(true, Some(false)));
    }

    #[test]
    fn vsync_beats_an_asset_turning_it_off() {
        assert!(wanted_vsync(false, Some(true)));
    }
}

/// 🔴 A generated mesh reaches the GPU only through this store, and its absence reads exactly like
/// "nothing to upload".
#[test]
fn build_inserts_the_generated_mesh_store() {
    use crate::meshlet::GeneratedMeshes;

    let mut app = kooch_core::app::App::new();
    kooch_core::plugin::Plugin::build(&super::RenderPlugin, &mut app);

    assert!(
        app.resources().get::<GeneratedMeshes>().is_some(),
        "a mesh built at runtime has nowhere to go",
    );
}

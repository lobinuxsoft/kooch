use kooch_core::stage::Stage;

/// The ordering the frame loop exists to get right, now a constraint rather than a call order.
#[test]
fn the_scene_runs_before_the_present() {
    let names = render_systems();
    let scene = position(&names, "render_meshlets_system");
    let present = position(&names, "present_frame_system");
    assert!(
        scene < present,
        "the swapchain image is acquired before the scene is submitted: the \
         meshlet stage draws into its own textures and needs no surface, so \
         acquiring first makes the CPU wait out the compositor before \
         recording work the compositor has nothing to do with — {names:?}",
    );
    assert!(
        position(&names, "prepare_frame_system") < scene,
        "{names:?}"
    );
}

/// What the ordering is for: a pass registered later, by name, lands between two engine systems
/// (#392). Registration order alone would leave it last.
#[test]
fn a_late_pass_lands_between_them() {
    use kooch_core::schedule::Order;

    let mut app = plugged();
    app.add_ordered(
        Stage::Render,
        Order::after("render_meshlets_system").and_before("present_frame_system"),
        |_: &mut kooch_core::resource::Resources| {},
    );

    let names = names_of(&app);
    let late = position(&names, "{{closure}}");
    assert!(
        position(&names, "render_meshlets_system") < late,
        "{names:?}"
    );
    assert!(late < position(&names, "present_frame_system"), "{names:?}");
}

/// The counterpart, so the tests above cannot pass on a frame that stopped acquiring or stopped
/// rendering. One of each, each in the system that owns it.
#[test]
fn the_frame_loop_still_does_both() {
    assert_eq!(
        include_str!("meshlets.rs")
            .matches("render_with_assets_primary(")
            .count(),
        1,
        "one scene render per frame",
    );
    assert_eq!(
        include_str!("present.rs")
            .matches("get_current_texture()")
            .count(),
        1,
        "one acquire per frame, and one place to keep after the scene",
    );
}

fn plugged() -> kooch_core::app::App {
    let mut app = kooch_core::app::App::new();
    kooch_core::plugin::Plugin::build(&super::RenderPlugin, &mut app);
    app
}

fn names_of(app: &kooch_core::app::App) -> Vec<String> {
    app.schedule()
        .systems()
        .iter()
        .filter(|system| system.stage == Stage::Render)
        .map(|system| system.short_name().to_owned())
        .collect()
}

fn render_systems() -> Vec<String> {
    names_of(&plugged())
}

fn position(names: &[String], wanted: &str) -> usize {
    names
        .iter()
        .position(|name| name.contains(wanted))
        .unwrap_or_else(|| panic!("{wanted} is not in {names:?}"))
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

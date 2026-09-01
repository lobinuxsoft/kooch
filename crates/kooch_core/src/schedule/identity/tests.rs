use super::*;

/// The name a wrapped project system actually arrives with, copied from
/// what `type_name` printed rather than composed by hand.
const WRAPPED: &str = "kooch_core::run_state::run_if_playing<roll_a_ball::registrations::systems::input::read_player_input>::{{closure}}";

#[test]
fn a_wrapped_system_keeps_its_own_name() {
    assert_eq!(short_name(WRAPPED), "read_player_input");
}

#[test]
fn a_plain_path_loses_its_modules() {
    assert_eq!(
        short_name("kooch_render::plugin::assets::init_material_pipeline"),
        "init_material_pipeline",
    );
}

/// Every anonymous closure would otherwise read as the same row.
#[test]
fn a_closure_keeps_its_module() {
    assert_eq!(
        short_name("kooch_render::plugin::assets::{{closure}}"),
        "assets::{{closure}}",
    );
}

/// Wrappers compose, so the unwrapping has to as well.
#[test]
fn nested_wrappers_all_come_off() {
    let doubled = format!("outer<{WRAPPED}>");
    assert_eq!(short_name(&doubled), "read_player_input");
}

#[test]
fn a_bare_name_survives() {
    assert_eq!(short_name("spin_pivots"), "spin_pivots");
}

use super::*;

/// A bare function's path reaches the system even though the schedule
/// holds it wrapped, because both canonicalise to the inner path.
#[test]
fn a_wrapped_system_is_reachable_by_its_path() {
    let scheduled =
        SystemKey::new("kooch_core::run_state::run_if_playing<game::systems::jump>::{{closure}}");
    let asked_for = SystemKey::new("game::systems::jump");
    assert_eq!(scheduled, asked_for);
}

#[test]
fn an_untouched_build_disables_nothing() {
    let toggles = SystemToggles::new();
    assert!(toggles.is_empty());
    assert!(!toggles.is_disabled(&SystemKey::new("game::systems::jump")));
}

#[test]
fn disable_then_enable_is_a_round_trip() {
    let mut toggles = SystemToggles::new();
    let key = SystemKey::new("game::systems::jump");

    toggles.disable("game::systems::jump");
    assert!(toggles.is_disabled(&key));

    toggles.enable("game::systems::jump");
    assert!(!toggles.is_disabled(&key));
    assert!(toggles.is_empty());
}

/// Two anonymous closures in one module share a `type_name`, so the
/// occurrence is the only thing telling them apart.
#[test]
fn two_closures_are_addressed_apart() {
    let mut toggles = SystemToggles::new();
    let first = SystemKey::nth("a::b::{{closure}}", 0);
    let second = SystemKey::nth("a::b::{{closure}}", 1);

    toggles.disable(second.clone());
    assert!(toggles.is_disabled(&second));
    assert!(
        !toggles.is_disabled(&first),
        "both closures went off at once"
    );
}

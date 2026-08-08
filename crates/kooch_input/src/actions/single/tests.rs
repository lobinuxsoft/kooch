use super::*;
use crate::actions::action::ControlType;
use crate::actions::binding::Binding;
use crate::actions::path::ControlPath;
use crate::ids::KeyCode;

fn jump() -> Action {
    Action::new("jump", ControlType::Button).bind(Binding::to(ControlPath::Key(KeyCode::Space)))
}

/// The point of the asset: one action survives a file on its own,
/// with no map wrapped around it.
#[test]
fn one_action_round_trips_through_its_own_file() {
    let action = jump();
    let text = to_ron(&action).expect("serialise");
    let mut ctx = LoadContext {
        path: std::path::Path::new("jump.inputaction"),
    };
    let back = InputActionLoader
        .load(text.as_bytes(), &mut ctx)
        .expect("load");
    assert_eq!(back, action);
}

/// It installs itself, like every other asset type.
#[test]
fn the_action_type_registers_itself() {
    let found: Vec<&str> = kooch_core::asset_registry::registered_asset_types()
        .map(|registration| (registration.type_name)())
        .collect();
    assert!(
        found.contains(&std::any::type_name::<Action>()),
        "a standalone action is not loadable by any binary: {found:?}"
    );
}

/// 🔴 The cache is what a game's own component reads through.
///
/// A component appears once per entity, so a mechanic needing two
/// actions holds two guids in a component of its own — and then it
/// has no way to turn a guid into a value. This is that way, and it
/// takes no map, no `InputAction` and no asset server.
#[test]
fn a_guid_can_be_evaluated_without_a_component() {
    use crate::mock_backend::MockInputBackend;

    let guid = kooch_core::Guid::new_v4();
    let mut loaded = LoadedActions::default();
    loaded.set(guid, jump(), std::time::SystemTime::UNIX_EPOCH);

    let mut backend = MockInputBackend::new();
    backend.press_key(KeyCode::Space);

    let value = loaded
        .evaluate(Some(guid), &backend)
        .expect("a loaded action must evaluate");
    assert!(value.pressed, "the key did not reach the action");

    assert!(
        loaded.evaluate(None, &backend).is_none(),
        "an unset reference must read as nothing, not as pressed"
    );
    assert!(
        loaded
            .evaluate(Some(kooch_core::Guid::new_v4()), &backend)
            .is_none(),
        "an unknown guid must read as nothing"
    );
}

/// Loading twice keeps one entry: the cache is keyed by guid, and a
/// duplicate would make which copy answers depend on insertion order.
#[test]
fn reloading_an_action_replaces_it() {
    let guid = kooch_core::Guid::new_v4();
    let mut loaded = LoadedActions::default();
    loaded.set(guid, jump(), std::time::SystemTime::UNIX_EPOCH);
    loaded.set(
        guid,
        Action::new("renamed", crate::actions::action::ControlType::Button),
        std::time::SystemTime::UNIX_EPOCH,
    );

    assert_eq!(loaded.by_guid.len(), 1);
    assert_eq!(loaded.get(guid).map(|a| a.name.as_str()), Some("renamed"));
}

/// 🔴 An action edited on disk is picked up without a restart.
///
/// Reported from use: saving a rebind in the panel changed nothing
/// until the remote process was killed and relaunched, which reads as
/// "assets need a recompile" when nothing needs compiling. The cache
/// skipped any guid it already held, so a file was read once per
/// process and never again.
#[test]
fn a_newer_file_is_considered_stale() {
    let guid = kooch_core::Guid::new_v4();
    let mut loaded = LoadedActions::default();

    let read_at = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
    loaded.set(guid, jump(), read_at);

    assert!(
        !loaded.is_stale(guid, read_at),
        "an unchanged file was reloaded, which re-reads it every frame"
    );
    assert!(
        loaded.is_stale(guid, read_at + std::time::Duration::from_secs(1)),
        "a file saved after it was read was not picked up — the edit is \
             invisible until the process restarts"
    );
    assert!(
        loaded.is_stale(kooch_core::Guid::new_v4(), read_at),
        "an action never read must count as stale, or it never loads"
    );
}

/// 🔴 Disabled means silent, per action — the thing a map cannot do,
/// and the reason for going without one.
#[test]
fn a_disabled_action_reads_as_nothing() {
    let mut input = InputAction::new();
    input.value = ActionValue {
        vector: glam::Vec3::new(1.0, 0.0, 0.0),
        pressed: true,
    };

    assert!(input.pressed());
    assert_eq!(input.axis(), 1.0);

    input.enabled = false;
    assert!(!input.pressed(), "a disabled action still reported held");
    assert_eq!(input.axis(), 0.0);
    assert_eq!(input.vector(), glam::Vec2::ZERO);
}

/// Edges come from comparing frames, so a held button fires once.
#[test]
fn a_held_action_is_pressed_once() {
    let mut input = InputAction::new();
    input.value.pressed = true;
    assert!(input.just_pressed());

    input.was_pressed = true;
    assert!(!input.just_pressed(), "a held action fired twice");
    assert!(input.pressed());

    input.value.pressed = false;
    assert!(input.just_released());
}

/// Disabling mid-press must not leave a release nobody sees: with the
/// action silent, `just_released` is what a listener gets.
#[test]
fn disabling_a_held_action_reads_as_a_release() {
    let mut input = InputAction::new();
    input.value.pressed = true;
    input.was_pressed = true;
    input.enabled = false;

    assert!(!input.pressed());
    assert!(input.just_released());
}

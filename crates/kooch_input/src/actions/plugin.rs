//! Wiring the action model into a frame.
//!
//! Without this the model is data nobody reads — a shape the engine has
//! produced before, and the reason `docs/CAPABILITIES.md` exists. So it
//! lands with its consumer rather than ahead of it.
//!
//! # Where it sits
//!
//! `Stage::Input`, right after the backend has been pumped and before
//! anything in `Update`. A gameplay system therefore reads the action as
//! of *this* frame, not last one.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;

use super::action::ActionMap;
use super::state::ActionState;
use crate::backend::InputBackend;

/// The action map a game is currently playing under.
///
/// One for now. Stacking several with priority — so a vehicle map can sit
/// over an on-foot one and consume what it handles — is the remaining
/// half of #55, and deliberately not built until something needs it: the
/// field is on [`ActionMap`] already, and a second consumer will say more
/// about the shape than guessing does.
#[derive(Debug, Default, Clone)]
pub struct ActiveActionMap(pub ActionMap);

impl ActiveActionMap {
    pub fn new(map: ActionMap) -> Self {
        Self(map)
    }
}

impl Default for ActionMap {
    fn default() -> Self {
        ActionMap::new("default")
    }
}

/// Reads the active map into [`ActionState`] once per frame.
///
/// Add it after [`InputPlugin`](crate::InputPlugin): this reads what that
/// one pumps, and plugin order is system order within a stage.
#[derive(Default)]
pub struct ActionsPlugin {
    /// The map to start under. A game hands its own; the editor's panel
    /// will replace it wholesale when #58 lands.
    pub map: ActionMap,
}

impl ActionsPlugin {
    pub fn new(map: ActionMap) -> Self {
        Self { map }
    }
}

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        let map = self.map.clone();
        app.insert_resource(ActionState::for_map(&map))
            .insert_resource(ActiveActionMap::new(map))
            .add_system(Stage::Input, update_actions);
    }

    fn name(&self) -> &str {
        "ActionsPlugin"
    }
}

/// Resolves every action against the backend.
fn update_actions(resources: &mut Resources) {
    // Three resources, and the borrow checker allows one at a time — so
    // the map is cloned out before the state is taken mutably. It is a
    // handful of strings once a frame, and the alternative is threading a
    // borrow through two other resources.
    let Some(map) = resources
        .get::<ActiveActionMap>()
        .map(|active| active.0.clone())
    else {
        return;
    };
    let Some(backend) = resources.get::<Box<dyn InputBackend>>() else {
        return;
    };
    // Read the backend into a value the state can be updated from without
    // holding two borrows at once.
    let backend: &dyn InputBackend = &**backend;
    let mut next = resources
        .get::<ActionState>()
        .cloned()
        .unwrap_or_else(|| ActionState::for_map(&map));
    next.update(&map, backend);
    drop(backend);

    if let Some(state) = resources.get_mut::<ActionState>() {
        *state = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{Action, ControlType};
    use crate::actions::binding::Binding;
    use crate::actions::path::ControlPath;
    use crate::ids::KeyCode;
    use crate::mock_backend::MockInputBackend;

    /// The point of this module: after a frame, a game asking the state
    /// gets the answer without touching a backend.
    #[test]
    fn a_frame_leaves_the_action_readable() {
        let map = ActionMap::new("gameplay").add(
            Action::new("jump", ControlType::Button)
                .bind(Binding::to(ControlPath::Key(KeyCode::Space))),
        );
        let jump = map.resolve("jump").unwrap();

        let mut resources = Resources::new();
        let mut backend = MockInputBackend::new();
        backend.press_key(KeyCode::Space);
        let backend: Box<dyn InputBackend> = Box::new(backend);
        resources.insert(backend);
        resources.insert(ActionState::for_map(&map));
        resources.insert(ActiveActionMap::new(map));

        update_actions(&mut resources);

        let state = resources.get::<ActionState>().expect("state");
        assert!(state.pressed(jump), "the frame did not reach the action");
        assert!(state.just_pressed(jump));
    }

    /// Missing pieces must be a quiet no-op, not a panic: a headless host
    /// or a test harness legitimately has no backend.
    #[test]
    fn a_missing_backend_is_not_fatal() {
        let mut resources = Resources::new();
        resources.insert(ActiveActionMap::default());
        update_actions(&mut resources);
    }
}

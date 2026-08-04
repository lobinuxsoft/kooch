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
pub struct ActiveActionMap {
    pub map: ActionMap,
    /// The asset this came from, when it came from one.
    ///
    /// What lets the sync system answer "is this already the map the
    /// component asks for" without re-reading the file to find out.
    pub source: Option<kooch_core::Guid>,
    /// Bumped every time this is replaced.
    ///
    /// [`ActionMap::resolve`] is a string compare, so gameplay is meant
    /// to hold an [`ActionId`](super::action::ActionId) rather than
    /// re-look-up a name per frame. Holding one is only correct if there
    /// is a way to know it went stale — an id is an **index into this
    /// map**, so under a different map it silently points at another
    /// action, or past the end.
    ///
    /// Compare this against what a cached id was resolved under, and
    /// re-resolve when they differ. Cheaper than a string compare and,
    /// unlike `source`, it also catches a map replaced by the same asset
    /// reloaded.
    pub generation: u32,
}

impl ActiveActionMap {
    pub fn new(map: ActionMap) -> Self {
        Self {
            map,
            source: None,
            generation: 0,
        }
    }

    pub fn from_asset(map: ActionMap, guid: kooch_core::Guid) -> Self {
        Self {
            map,
            source: Some(guid),
            generation: 0,
        }
    }

    /// Replaces what is active, moving the generation forward.
    ///
    /// The only way a map should be swapped: constructing a fresh value
    /// and inserting it would reset the generation to zero, and a cached
    /// id resolved under the *previous* generation zero would look
    /// current while pointing into a different map.
    pub fn replace(resources: &mut kooch_core::resource::Resources, next: Self) {
        let generation = resources
            .get::<Self>()
            .map(|active| active.generation.wrapping_add(1))
            .unwrap_or(0);
        resources.insert(Self { generation, ..next });
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
pub struct ActionsPlugin {
    /// Where the bindings come from.
    source: Source,
}

/// Where `ActionsPlugin` gets its map.
#[derive(Default)]
enum Source {
    /// Declared in code. Fine for a test or a prototype, and a dead end
    /// for anything a player should be able to rebind: nothing the editor
    /// does can reach a value compiled into the binary.
    Literal(ActionMap),
    /// Whatever the scene's [`InputMapSource`] points at.
    ///
    /// The default, because it is the one that needs no code: the field
    /// is filled in the Inspector from the asset picker, travels with the
    /// scene, and swaps live when changed.
    ///
    /// [`InputMapSource`]: super::component::InputMapSource
    FromScene,
    /// An `.inputmap` asset, by guid.
    ///
    /// A guid rather than a path, for the same reason every other asset
    /// reference is one: renaming or moving the file keeps the binding,
    /// because the identity lives in the `.meta` beside it rather than in
    /// the name.
    Asset(kooch_core::Guid),
    #[default]
    Empty,
}

impl Default for ActionsPlugin {
    /// Follows the scene's `InputMapSource`.
    fn default() -> Self {
        Self {
            source: Source::FromScene,
        }
    }
}

impl ActionsPlugin {
    /// A map declared in code.
    pub fn new(map: ActionMap) -> Self {
        Self {
            source: Source::Literal(map),
        }
    }

    /// The map in the `.inputmap` asset with this guid.
    ///
    /// This is the one a shipped game wants: the bindings are data an
    /// editor authored, not a literal recompiled into the binary.
    pub fn from_asset(guid: kooch_core::Guid) -> Self {
        Self {
            source: Source::Asset(guid),
        }
    }
}

/// Declares [`InputMapSource`] without running any input.
///
/// The editor needs the component to exist as data — to inspect it, to
/// offer the asset picker, to mirror it — while gameplay lives in the
/// project's process. Same split `CameraComponentsPlugin` makes, and for
/// the same reason: a host that authors is not a host that plays.
///
/// [`InputMapSource`]: super::component::InputMapSource
pub struct InputComponentsPlugin;

impl Plugin for InputComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>() {
                registry.register_cpu_reflected::<super::component::InputMapSource>();
                // The per-mechanic half: one action per component, each
                // enabled on its own. Registered beside the map's source
                // because both are things a scene authors.
                registry.register_cpu_reflected::<super::single::InputAction>();
            }
        });
    }

    fn name(&self) -> &str {
        "InputComponentsPlugin"
    }
}

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        // The component has to exist wherever the actions do, or a scene
        // pointing at a map would load into a world that cannot hold the
        // pointer.
        InputComponentsPlugin.build(app);
        match &self.source {
            Source::Literal(map) => {
                let map = map.clone();
                app.insert_resource(ActionState::for_map(&map))
                    .insert_resource(ActiveActionMap::new(map));
            }
            Source::FromScene => {
                app.insert_resource(ActiveActionMap::default())
                    .insert_resource(ActionState::default())
                    // Before the actions are read, so the first frame
                    // under a new map already resolves against it.
                    .add_system(Stage::PreUpdate, sync_map_from_scene);
            }
            Source::Asset(guid) => {
                // Loaded in `Startup` rather than here: the asset server
                // is built by the plugin that owns assets, and a plugin
                // reaching for a resource another may not have inserted
                // yet is the ordering bug this codebase has hit twice.
                let guid = *guid;
                app.insert_resource(ActiveActionMap::default())
                    .insert_resource(ActionState::default())
                    .add_system(Stage::Startup, move |resources: &mut Resources| {
                        load_map_asset(resources, guid);
                    });
            }
            Source::Empty => {
                app.insert_resource(ActiveActionMap::default())
                    .insert_resource(ActionState::default());
            }
        }
        app.add_system(Stage::Input, update_actions);
        // Standalone actions, evaluated per component. Independent of
        // the active map: an entity reading one needs no map at all.
        app.add_system(Stage::Input, super::single::read_input_actions);
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
        .map(|active| active.map.clone())
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
        ActiveActionMap::replace(&mut resources, ActiveActionMap::new(map));

        update_actions(&mut resources);

        let state = resources.get::<ActionState>().expect("state");
        assert!(state.pressed(jump), "the frame did not reach the action");
        assert!(state.just_pressed(jump));
    }

    /// 🔴 A failed load is remembered, so it is reported once.
    ///
    /// Reported from a real run: fourteen identical `input map could not
    /// be loaded` lines, one per frame, burying the first — the only one
    /// that says anything. It fails the same way every frame, so retrying
    /// is a log flood rather than a recovery path.
    #[test]
    fn a_failed_load_is_remembered_once() {
        let guid = kooch_core::Guid::new_v4();
        let other = kooch_core::Guid::new_v4();
        let mut failed = FailedMapLoads::default();

        assert!(!failed.contains(guid), "nothing has failed yet");
        failed.record(guid);
        failed.record(guid);
        assert!(failed.contains(guid));
        assert_eq!(failed.guids.len(), 1, "the same guid was recorded twice");
        assert!(
            !failed.contains(other),
            "one failure blocked an unrelated map"
        );
    }

    /// And the load itself reports failure rather than pretending, which
    /// is what the guard above is driven by. No asset server is the
    /// cheapest way to fail for the same reason on every frame.
    #[test]
    fn a_load_without_a_server_reports_failure() {
        let mut resources = Resources::new();
        assert!(
            !load_map_asset(&mut resources, kooch_core::Guid::new_v4()),
            "a load with no server claimed to have succeeded"
        );
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

/// Pulls the map out of its asset and makes it the active one.
/// Returns whether the map is now the active one.
fn load_map_asset(resources: &mut Resources, guid: kooch_core::Guid) -> bool {
    let Some(mut server) = resources.remove::<kooch_core::asset_loader::AssetServer>() else {
        tracing::error!(%guid, "no asset server, so the input map cannot be loaded");
        return false;
    };
    let loaded = server.load_by_guid::<ActionMap>(guid, resources);
    resources.insert(server);

    match loaded {
        Ok(handle) => {
            let Some(map) = resources
                .get::<kooch_core::assets::Assets<ActionMap>>()
                .and_then(|assets| assets.get(handle).cloned())
            else {
                tracing::error!(%guid, "input map loaded but not in storage");
                return false;
            };
            tracing::info!(%guid, actions = map.actions.len(), "input map active");
            resources.insert(ActionState::for_map(&map));
            ActiveActionMap::replace(resources, ActiveActionMap::from_asset(map, guid));
            true
        }
        // Named rather than silent: with no map every action resolves to
        // nothing, so the game simply does not respond — which reads as
        // broken input rather than as a missing file.
        Err(e) => {
            tracing::error!(%guid, error = %e, "input map could not be loaded");
            false
        }
    }
}

/// Makes the scene's [`InputMapSource`] the active map, when it changes.
///
/// Reads the component, compares against what is already active, and
/// loads only on a mismatch. Loading every frame would re-parse a file to
/// arrive at the same answer — and would overwrite whatever the editor's
/// panel is in the middle of editing.
///
/// [`InputMapSource`]: super::component::InputMapSource
fn sync_map_from_scene(resources: &mut Resources) {
    let wanted = {
        let query = kooch_ecs::Query::<&super::component::InputMapSource>::new(resources);
        // First one wins. The active map is a property of the session, so
        // two entities asking for different ones is a scene that has not
        // decided — and picking the earlier one is at least stable.
        query.iter().find_map(|source| source.map)
    };

    let Some(guid) = wanted else {
        return;
    };
    if resources
        .get::<ActiveActionMap>()
        .is_some_and(|active| active.source == Some(guid))
    {
        return;
    }
    // A load that failed fails the same way next frame, so retrying it is
    // one identical error line per frame — which buries the first one,
    // the only one that says anything. Recorded before the attempt so a
    // panic on the way in does not loop either.
    if resources
        .get::<FailedMapLoads>()
        .is_some_and(|failed| failed.contains(guid))
    {
        return;
    }
    if !load_map_asset(resources, guid) {
        match resources.get_mut::<FailedMapLoads>() {
            Some(failed) => failed.record(guid),
            None => {
                let mut failed = FailedMapLoads::default();
                failed.record(guid);
                resources.insert(failed);
            }
        }
    }
}

/// Maps that failed to load, so the error is reported once.
///
/// Cleared by nothing: a guid that failed will keep failing until
/// something changes that requires a restart anyway — the asset was
/// missing, or its type was not registered. Retrying is not a recovery
/// path, it is a log flood.
#[derive(Debug, Default)]
pub struct FailedMapLoads {
    guids: Vec<kooch_core::Guid>,
}

impl FailedMapLoads {
    fn contains(&self, guid: kooch_core::Guid) -> bool {
        self.guids.contains(&guid)
    }

    fn record(&mut self, guid: kooch_core::Guid) {
        if !self.contains(guid) {
            self.guids.push(guid);
        }
    }
}

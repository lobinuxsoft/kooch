//! An action on its own: `.inputaction` as an asset, and the component
//! that reads one.
//!
//! # Why without a map
//!
//! A map groups actions that turn on and off **together**. That is worth
//! having, and it is not what a mechanic wants: jumping and moving are
//! two capabilities of an entity, enabled and disabled for their own
//! reasons, and the code that reads one should not have to name it inside
//! a shared list.
//!
//! Naming is the concrete cost. With a map, gameplay writes
//! `map.resolve("jump")` — the action's *name* becomes the contract, so
//! renaming it in the panel silently stops the control, and every
//! consumer spells the string out. As an asset, the action is referenced
//! by guid from a field in the Inspector, exactly like a mesh: no string,
//! and renaming the file changes nothing.
//!
//! Unity supports the same thing and calls them singleton actions —
//! *"actions can stand on their own and do not necessarily need to belong
//! to a map"*. It wraps each in a hidden map of one, because its
//! evaluator requires a map. Ours never did: [`evaluate`] takes an
//! action.
//!
//! # What is kept
//!
//! Everything an action *is*: composites, parts, processors, several
//! devices at once. This changes where an action lives and how it is
//! referenced, not what it can express — the same `Action` type is
//! serialised, so a `.inputaction` is one entry of a `.inputmap`.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

use super::action::Action;
use super::state::ActionValue;

/// Extension the asset database routes to [`InputActionLoader`].
pub const INPUT_ACTION_EXTENSION: &str = "inputaction";

/// Reads a `.inputaction` file into an [`Action`].
#[derive(Debug, Default, Clone, Copy)]
pub struct InputActionLoader;

impl AssetLoader<Action> for InputActionLoader {
    fn extensions(&self) -> &[&'static str] {
        &[INPUT_ACTION_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Action> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        let mut action: Action =
            ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))?;
        // A file written before actions had ids gets one derived from its
        // name, so it is referenceable from the first load.
        action.ensure_id("");
        Ok(action)
    }
}

kooch_core::register_asset!(Action, InputActionLoader);

/// Serialises one action for writing.
pub fn to_ron(action: &Action) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(action, ron::ser::PrettyConfig::default())
}

/// Writes `action` to `path` and gives it an asset identity.
///
/// The identity is the point, and the lesson is the same one `prefab` and
/// `.inputmap` both record: the database registers a file only when a
/// `.meta` sits beside it and never invents one, so an action written
/// without this is a file nothing can reference.
pub fn save(action: &Action, path: &std::path::Path) -> Result<kooch_core::Guid, String> {
    let text = to_ron(action).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    let meta = kooch_core::asset_meta::read_or_create_typed(path, std::any::type_name::<Action>())
        .map_err(|e| e.to_string())?;
    Ok(meta.guid)
}

/// One action an entity reads, and whether it is listening.
///
/// Put one per mechanic: a `move` on the player, a `jump` beside it, a
/// different `move` on an enemy. Each is enabled on its own, which is
/// what a map cannot do — a map is all or nothing.
///
/// The value is written by [`read_input_actions`] once per frame and read
/// by gameplay in the same frame.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(category = "Input")]
pub struct InputAction {
    /// The `.inputaction` this reads. `None` is an action that never
    /// fires — harmless, so a component added and not filled in does not
    /// break the entity it is on.
    #[reflect(asset = "kooch_input::actions::action::Action")]
    pub action: Option<kooch_core::Guid>,
    /// Whether it is listening. This is the per-action on/off a map gives
    /// only in bulk: pausing one mechanic without touching the rest.
    pub enabled: bool,
    /// This frame's value. Not serialised — it is the *result* of input,
    /// and a scene that stored it would load with a jump half-pressed.
    #[reflect(skip)]
    pub value: ActionValue,
    /// Last frame's `pressed`, so edges are derived rather than
    /// remembered. Same reason as [`ActionState`](super::state::ActionState):
    /// a dropped frame self-corrects, where a queued event would leave
    /// the action stuck down.
    #[reflect(skip)]
    pub was_pressed: bool,
}

impl Component for InputAction {}

impl InputAction {
    /// Enabled, pointing at nothing yet.
    pub fn new() -> Self {
        Self {
            action: None,
            enabled: true,
            value: ActionValue::default(),
            was_pressed: false,
        }
    }

    /// Enabled, pointing at `asset`.
    pub fn to(asset: kooch_core::Guid) -> Self {
        Self {
            action: Some(asset),
            ..Self::new()
        }
    }

    /// Held right now.
    pub fn pressed(&self) -> bool {
        self.enabled && self.value.pressed
    }

    /// True only on the frame it went down.
    pub fn just_pressed(&self) -> bool {
        self.pressed() && !self.was_pressed
    }

    pub fn just_released(&self) -> bool {
        !self.pressed() && self.was_pressed
    }

    /// The scalar value — a trigger, an axis.
    pub fn axis(&self) -> f32 {
        if self.enabled { self.value.axis() } else { 0.0 }
    }

    /// The 2D value — a stick, WASD.
    pub fn vector(&self) -> glam::Vec2 {
        if self.enabled {
            self.value.vector2()
        } else {
            glam::Vec2::ZERO
        }
    }

    /// The 3D value, for an action bound to a 3D composite.
    pub fn vector3(&self) -> glam::Vec3 {
        if self.enabled {
            self.value.vector
        } else {
            glam::Vec3::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
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
}

/// Every `.inputaction` this frame's components asked for, by guid.
///
/// Exists because a component can only appear **once per entity**, so a
/// mechanic that reads two actions cannot hold two [`InputAction`]s — it
/// holds two guids in a component of its own. Loading them is the part
/// that needs the asset server, which is awkward from a game system and
/// identical for everyone, so the engine keeps the result here and a game
/// just looks up what it points at:
///
/// ```ignore
/// let loaded = resources.get::<LoadedActions>()?;
/// let value = loaded.evaluate(player.jump, backend)?;
/// ```
#[derive(Debug, Default)]
pub struct LoadedActions {
    by_guid: Vec<(kooch_core::Guid, Action)>,
    /// When each was last read off disk, so an edit is noticed.
    ///
    /// Without this a `.inputaction` is read once per process: saving a
    /// rebind in the panel changed nothing until the game was restarted,
    /// and the only way to see an edit was to relaunch — which reads as
    /// "assets need a recompile" when nothing needs compiling at all.
    read_at: Vec<(kooch_core::Guid, std::time::SystemTime)>,
}

impl LoadedActions {
    /// The action with this guid, if it has been loaded.
    pub fn get(&self, guid: kooch_core::Guid) -> Option<&Action> {
        self.by_guid
            .iter()
            .find(|(candidate, _)| *candidate == guid)
            .map(|(_, action)| action)
    }

    /// Reads whatever `reference` points at. `None` when it points at
    /// nothing, or at an action that could not be loaded.
    pub fn evaluate(
        &self,
        reference: Option<kooch_core::Guid>,
        backend: &dyn crate::backend::InputBackend,
    ) -> Option<ActionValue> {
        let action = self.get(reference?)?;
        Some(super::state::evaluate(action, backend))
    }

    fn set(&mut self, guid: kooch_core::Guid, action: Action, modified: std::time::SystemTime) {
        match self.by_guid.iter_mut().find(|(g, _)| *g == guid) {
            Some(slot) => slot.1 = action,
            None => self.by_guid.push((guid, action)),
        }
        match self.read_at.iter_mut().find(|(g, _)| *g == guid) {
            Some(slot) => slot.1 = modified,
            None => self.read_at.push((guid, modified)),
        }
    }

    /// Whether what is on disk is newer than what was read.
    fn is_stale(&self, guid: kooch_core::Guid, on_disk: std::time::SystemTime) -> bool {
        self.read_at
            .iter()
            .find(|(g, _)| *g == guid)
            .is_none_or(|(_, read)| on_disk > *read)
    }
}

/// Says once how many `.inputaction` assets the database knows about.
fn report_once(registered: usize, already_loaded: usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static REPORTED: AtomicBool = AtomicBool::new(false);

    if REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    if registered == 0 {
        tracing::warn!(
            "no .inputaction assets are registered, so every action reference \
             reads as nothing — is there a .meta beside each one, and is the \
             project's assets/ being scanned?"
        );
    } else {
        tracing::info!(registered, already_loaded, "input actions found");
    }
}

/// Loads every `.inputaction` the project has into [`LoadedActions`].
///
/// All of them rather than the ones currently referenced, because a
/// component the engine does not know about — a game's own `PlayerInput`
/// holding two guids — is exactly the case this exists for, and the
/// engine cannot ask it what it wants. A project has a handful of
/// actions, and each is loaded once: `load_by_guid` returns the handle it
/// already has.
pub fn load_input_actions(resources: &mut kooch_core::resource::Resources) {
    use kooch_core::assets::Assets;

    let wanted: Vec<(kooch_core::Guid, std::path::PathBuf, std::time::SystemTime)> = {
        let Some(database) = resources.get::<kooch_core::asset_database::AssetDatabase>() else {
            tracing::warn!("no asset database, so no input action can be found");
            return;
        };
        let known = resources
            .get::<LoadedActions>()
            .map(|loaded| loaded.by_guid.iter().map(|(g, _)| *g).collect::<Vec<_>>())
            .unwrap_or_default();
        let all: Vec<(kooch_core::Guid, std::path::PathBuf, std::time::SystemTime)> = database
            .entries_of_type(std::any::type_name::<Action>())
            .map(|(guid, entry)| {
                let path = entry.path.clone();
                // The file's own mtime rather than the entry's: the entry
                // records when the scan saw it, and the running game does
                // not rescan — so an edit made while it plays would never
                // show up.
                let on_disk = entry
                    .path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                (guid, path, on_disk)
            })
            .collect();
        // Said once, even when it is zero: "no actions registered" is the
        // answer to "why does nothing respond", and a silent early return
        // is what made that question unanswerable.
        report_once(all.len(), known.len());
        let stale = resources.get::<LoadedActions>();
        all.into_iter()
            .filter(|(guid, _, on_disk)| {
                stale.is_none_or(|loaded| loaded.is_stale(*guid, *on_disk))
            })
            .collect()
    };
    if wanted.is_empty() {
        return;
    }

    let Some(mut server) = resources.remove::<kooch_core::asset_loader::AssetServer>() else {
        return;
    };
    let mut loaded = Vec::new();
    for (guid, path, on_disk) in wanted {
        // Forgotten first, or `load_by_guid` hands back the copy already
        // in memory and a reload reloads nothing.
        server.forget::<Action>(&path);
        match server.load_by_guid::<Action>(guid, resources) {
            Ok(handle) => loaded.push((guid, handle, on_disk)),
            Err(e) => tracing::error!(%guid, error = %e, "input action could not be loaded"),
        }
    }
    resources.insert(server);

    let values: Vec<(kooch_core::Guid, Action, std::time::SystemTime)> = {
        let Some(assets) = resources.get::<Assets<Action>>() else {
            return;
        };
        loaded
            .into_iter()
            .filter_map(|(guid, handle, at)| assets.get(handle).map(|a| (guid, a.clone(), at)))
            .collect()
    };

    let mut cache = resources.remove::<LoadedActions>().unwrap_or_default();
    let names: Vec<String> = values.iter().map(|(_, a, _)| a.name.clone()).collect();
    for (guid, action, at) in values {
        cache.set(guid, action, at);
    }
    // Once, on the frame they arrive. Silence here is indistinguishable
    // from a component pointing at nothing, which is how a player that
    // does not move looks from the outside.
    tracing::info!(
        loaded = names.len(),
        actions = ?names,
        total = cache.by_guid.len(),
        "input actions available",
    );
    resources.insert(cache);
}

/// Reads every enabled [`InputAction`] against the backend, once a frame.
///
/// Runs in `Stage::Input`, after the backend is pumped and before
/// anything in `Update`, so a gameplay system sees this frame's value.
///
/// A disabled action keeps its last value rather than being zeroed, and
/// every reader already treats it as silent. Zeroing would make
/// re-enabling report a release nobody performed.
pub fn read_input_actions(resources: &mut kooch_core::resource::Resources) {
    use kooch_ecs::Query;

    let Some(backend) = resources.remove::<Box<dyn crate::backend::InputBackend>>() else {
        return;
    };
    let Some(loaded) = resources.remove::<LoadedActions>() else {
        resources.insert(backend);
        return;
    };

    {
        let query = Query::<&mut InputAction>::new(resources);
        query.for_each(|input| {
            input.was_pressed = input.value.pressed;
            if !input.enabled {
                return;
            }
            if let Some(value) = loaded.evaluate(input.action, &*backend) {
                input.value = value;
            }
        });
    }

    resources.insert(loaded);
    resources.insert(backend);
}

//! The `.inputmap` asset — bindings on disk.
//!
//! # Its own extension, not `.ron`
//!
//! Materials are `.ron`, and the loader for those carries a note saying
//! every `.ron` under `assets/` is parsed as a Material until something
//! else needs discriminating. This is that something else. A distinct
//! extension answers it without a tag-sniffing tier: the asset database
//! routes by extension, the editor can offer "New Input Map" without
//! asking what kind of `.ron` you meant, and a file whose name says what
//! it is beats one that has to be opened to find out.
//!
//! The contents are still RON — the same encoding the rest of the engine
//! authors in, so a binding is diffable and hand-editable when the panel
//! is not the fastest way.
//!
//! # One map per file
//!
//! Unity puts every action map of a game into one `.inputactions`. This
//! holds one, because the unit a game pushes and pops **is** a map: "on
//! foot" and "in vehicle" are two files, and a game that wants them
//! together loads two assets. Splitting later would break every path; a
//! game merging two files does not break anything.

use std::fmt;

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::action::ActionMap;

/// Extension the asset database routes to [`InputMapLoader`].
pub const INPUT_MAP_EXTENSION: &str = "inputmap";

/// Reads `.inputmap` files into an [`ActionMap`].
#[derive(Debug, Default, Clone, Copy)]
pub struct InputMapLoader;

impl AssetLoader<ActionMap> for InputMapLoader {
    fn extensions(&self) -> &[&'static str] {
        &[INPUT_MAP_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<ActionMap> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(InputMapParseError::Utf8(e))))?;
        let mut map: ActionMap = ron::from_str(text)
            .map_err(|e| AssetError::Loader(Box::new(InputMapParseError::Ron(e))))?;
        // A file written before actions had ids gets them here, derived
        // from its names, so a reference stored in a scene resolves on
        // the first load rather than only after a save.
        map.assign_missing_ids();

        // A duplicate name makes `resolve` a coin toss, and the failure
        // lands far from here — a control that works or does not
        // depending on which of two identical entries was written first.
        // Refusing the file names the problem where it can be fixed.
        let duplicates = map.duplicate_names();
        if !duplicates.is_empty() {
            return Err(AssetError::Loader(Box::new(
                InputMapParseError::DuplicateActions {
                    map: map.name.clone(),
                    names: duplicates.iter().map(|s| (*s).to_owned()).collect(),
                },
            )));
        }
        Ok(map)
    }
}

// Declared here, beside the loader, so any binary linking this crate can
// load an `.inputmap` with nothing added to a list anywhere else. Both
// the facade and the editor used to register this by hand, and neither
// installed the storage — which failed every load with `Assets<ActionMap>
// resource missing`, once per frame.
kooch_core::register_asset!(ActionMap, InputMapLoader);

/// Serialises a map for writing. The editor's save path.
pub fn to_ron(map: &ActionMap) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(map, ron::ser::PrettyConfig::default())
}

/// Writes `map` to `path` and gives it an asset identity.
///
/// The identity is the point, and it is the same lesson `prefab::save`
/// records: `AssetDatabase`'s scan registers a file only when a `.meta`
/// sits beside it and never invents one, so a map written without this is
/// a file with no guid — the picker cannot list it and nothing can
/// reference it. Creating the identity in the act that creates the asset
/// is what stops that depending on who loads it first.
pub fn save(map: &ActionMap, path: &std::path::Path) -> Result<kooch_core::Guid, String> {
    let text = to_ron(map).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    let meta =
        kooch_core::asset_meta::read_or_create_typed(path, std::any::type_name::<ActionMap>())
            .map_err(|e| e.to_string())?;
    Ok(meta.guid)
}

#[derive(Debug)]
pub enum InputMapParseError {
    Utf8(std::str::Utf8Error),
    Ron(ron::error::SpannedError),
    DuplicateActions { map: String, names: Vec<String> },
}

impl fmt::Display for InputMapParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "input map is not valid UTF-8: {e}"),
            Self::Ron(e) => write!(f, "input map parse failed: {e}"),
            Self::DuplicateActions { map, names } => write!(
                f,
                "input map `{map}` declares {} more than once; a name has to \
                 resolve to one action",
                names.join(", ")
            ),
        }
    }
}

impl std::error::Error for InputMapParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Utf8(e) => Some(e),
            Self::Ron(e) => Some(e),
            Self::DuplicateActions { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{Action, ControlType};
    use crate::actions::binding::{Binding, Composite, PartName, VectorMode};
    use crate::actions::path::ControlPath;
    use crate::actions::processor::Processor;
    use crate::ids::{GamepadAxis, GamepadButton, KeyCode};

    fn gameplay() -> ActionMap {
        ActionMap::new("gameplay")
            .add(Action::new("move", ControlType::Vector2).bind_all([
                Binding::composite(Composite::Vector2 {
                    mode: VectorMode::DigitalNormalized,
                }),
                Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
                Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
                Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
                Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
            ]))
            .add(
                Action::new("jump", ControlType::Button)
                    .bind(Binding::to(ControlPath::Key(KeyCode::Space)))
                    .bind(
                        Binding::to(ControlPath::Button(GamepadButton::South))
                            .with(Processor::Scale { factor: 1.0 }),
                    ),
            )
    }

    fn load(text: &str) -> AssetResult<ActionMap> {
        let path = std::path::Path::new("gameplay.inputmap");
        let mut ctx = LoadContext { path };
        InputMapLoader.load(text.as_bytes(), &mut ctx)
    }

    /// The whole reason the model stopped being generic: it has to
    /// survive a file.
    #[test]
    fn a_map_survives_the_round_trip_through_disk() {
        let map = gameplay();
        let text = to_ron(&map).expect("serialise");
        let back = load(&text).expect("load");
        assert_eq!(back, map);
    }

    /// 🔴 The `.inputmap` type installs itself: no list anywhere names
    /// it, and a binary that links this crate can load one.
    ///
    /// The shipped failure this pins: the loader was written out by hand
    /// in the facade *and* in the editor's bootstrap, and the storage in
    /// neither — so every load failed with `Assets<ActionMap> resource
    /// missing`, once per frame. Both halves now come from one
    /// declaration next to the loader.
    #[test]
    fn the_input_map_type_registers_itself() {
        let found: Vec<&str> = kooch_core::asset_registry::registered_asset_types()
            .map(|registration| (registration.type_name)())
            .collect();
        assert!(
            found.contains(&std::any::type_name::<ActionMap>()),
            "the .inputmap type is not in the link-time registry, so no \
             binary can load one: {found:?}"
        );
    }

    /// And it brings its storage, not just its loader — the half that
    /// was missing.
    #[test]
    fn registering_installs_both_the_loader_and_the_storage() {
        use kooch_core::asset_loader::AssetServer;
        use kooch_core::resource::Resources;

        let mut server = AssetServer::new();
        let mut resources = Resources::new();
        for registration in kooch_core::asset_registry::registered_asset_types() {
            if (registration.type_name)() == std::any::type_name::<ActionMap>() {
                (registration.register_loader)(&mut server);
                (registration.install_storage)(&mut resources);
            }
        }

        assert!(server.has_loader::<ActionMap>(), "no loader was installed");
        assert!(
            resources
                .get::<kooch_core::assets::Assets<ActionMap>>()
                .is_some(),
            "the loader has nowhere to put what it loads"
        );
    }

    /// Every composite the editor can create survives a save and a
    /// load. A composite that cannot round-trip is one the panel offers
    /// and the file silently loses.
    #[test]
    fn every_composite_survives_the_file() {
        use crate::actions::binding::{Composite, PartName};

        for composite in Composite::ALL.iter().copied() {
            let mut action =
                Action::new("a", composite.control_type()).bind(Binding::composite(composite));
            for name in PartName::of(composite) {
                action = action.bind(Binding::part(*name, ControlPath::Key(KeyCode::Space)));
            }
            let map = ActionMap::new("gameplay").add(action);

            let text = to_ron(&map).expect("serialise");
            let back = load(&text).unwrap_or_else(|e| panic!("{composite:?} does not load: {e}"));
            assert_eq!(back, map, "{composite:?} changed on the way through disk");
        }
    }

    /// 🔴 A file written before `Action::processors` existed still
    /// loads. Every `.inputmap` on disk predates the field, so without
    /// `#[serde(default)]` this release would refuse to open any of them.
    #[test]
    fn a_map_without_the_processors_field_still_loads() {
        let text = r#"(
            name: "gameplay",
            priority: 0,
            actions: [
                (
                    name: "jump",
                    control_type: Button,
                    bindings: [(role: Whole(Key(Space)), processors: [])],
                ),
            ],
        )"#;

        let map = load(text).expect("a map written before the field must still load");
        assert_eq!(map.actions[0].name, "jump");
        assert!(
            map.actions[0].processors.is_empty(),
            "a missing list must default to empty, not to something"
        );
    }

    /// And the game's own shipped map, which is exactly such a file.
    #[test]
    fn the_shipped_roll_a_ball_map_still_loads() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../roll-a-ball/assets/inputs/PlayerInputs.inputmap"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            // The game is a sibling checkout, not a dependency: absent is
            // not a failure, it just means this check did not run.
            return;
        };
        let map = load(&text).expect("the shipped input map no longer loads");
        assert!(map.resolve("move").is_some() && map.resolve("jump").is_some());
    }

    /// 🔴 A file written before actions had ids gets the **same** ids on
    /// every load.
    ///
    /// Random ids would be worse than none: a reference stored in a
    /// scene would point at nothing until someone opened the map and
    /// saved it, and would break again on the next machine. Derived from
    /// the names, an old file is referenceable from the first load.
    #[test]
    fn ids_derived_for_an_old_file_are_stable_across_loads() {
        let text = r#"(
            name: "gameplay",
            priority: 0,
            actions: [
                (name: "move", control_type: Vector2, bindings: []),
                (name: "jump", control_type: Button, bindings: []),
            ],
        )"#;

        let first = load(text).expect("load");
        let second = load(text).expect("load again");

        assert_eq!(
            first.actions[0].id, second.actions[0].id,
            "the same file produced different ids, so a stored reference \
             would break on the next load"
        );
        assert_ne!(
            first.actions[0].id, first.actions[1].id,
            "two actions share an id, so a reference is ambiguous"
        );
        assert_ne!(
            first.actions[0].id,
            kooch_core::Guid::from_bytes([0; 16]),
            "the id was left unassigned"
        );
    }

    /// And the game's own map, which is exactly such a file: every action
    /// in it is referenceable right now, with no save first.
    #[test]
    fn the_shipped_map_is_referenceable_without_being_resaved() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../roll-a-ball/assets/inputs/PlayerInputs.inputmap"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let map = load(&text).expect("the shipped map no longer loads");

        for action in &map.actions {
            assert_ne!(
                action.id,
                kooch_core::Guid::from_bytes([0; 16]),
                "`{}` has no id, so nothing can point at it",
                action.name
            );
            assert_eq!(
                map.resolve_ref(action.id).map(|id| id.index()),
                map.resolve(&action.name).map(|id| id.index()),
                "`{}` resolves differently by id than by name",
                action.name
            );
        }
    }

    /// Written by the editor, read by a human — the format has to be
    /// legible or the panel becomes the only way to fix anything.
    #[test]
    fn the_file_reads_as_what_it_binds() {
        let text = to_ron(&gameplay()).expect("serialise");
        for expected in ["gameplay", "move", "jump", "KeyW", "Space", "South"] {
            assert!(
                text.contains(expected),
                "`{expected}` is not findable in the file:\n{text}"
            );
        }
    }

    /// A duplicate name makes `resolve` pick one of two arbitrarily, and
    /// the symptom appears wherever that action is read. Refusing the
    /// file puts the error where it can be fixed.
    #[test]
    fn a_duplicate_action_name_is_refused_at_load() {
        let map = ActionMap::new("gameplay")
            .add(Action::new("jump", ControlType::Button))
            .add(Action::new("jump", ControlType::Button));
        let text = to_ron(&map).expect("serialise");

        let err = load(&text).expect_err("a duplicate name must not load");
        let message = err.to_string();
        assert!(
            message.contains("jump"),
            "the error does not name the offending action: {message}"
        );
    }

    /// Broken RON has to say so rather than produce an empty map, which
    /// would read as "no bindings" and send someone hunting elsewhere.
    #[test]
    fn a_malformed_file_fails_rather_than_loading_empty() {
        assert!(load("this is not ron").is_err());
        assert!(load("").is_err());
    }

    /// A field added later must not invalidate files written today.
    #[test]
    fn an_unknown_extension_is_not_claimed() {
        assert_eq!(InputMapLoader.extensions(), &["inputmap"]);
        assert!(
            !InputMapLoader.extensions().contains(&"ron"),
            "claiming .ron would collide with materials, whose loader \
             parses every .ron under assets/ as a Material"
        );
    }
}

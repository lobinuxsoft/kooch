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
        let map: ActionMap = ron::from_str(text)
            .map_err(|e| AssetError::Loader(Box::new(InputMapParseError::Ron(e))))?;

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

/// Serialises a map for writing. The editor's save path.
pub fn to_ron(map: &ActionMap) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(map, ron::ser::PrettyConfig::default())
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
    use crate::actions::binding::{Binding, Composite, PartName, Vector2Mode};
    use crate::actions::path::ControlPath;
    use crate::actions::processor::Processor;
    use crate::ids::{GamepadAxis, GamepadButton, KeyCode};

    fn gameplay() -> ActionMap {
        ActionMap::new("gameplay")
            .add(Action::new("move", ControlType::Vector2).bind_all([
                Binding::composite(Composite::Vector2 {
                    mode: Vector2Mode::DigitalNormalized,
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

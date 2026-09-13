//! Which document a Ctrl+Z belongs to.

use std::path::{Path, PathBuf};

use kooch_core::Guid;

use crate::state::EditorTab;

pub(crate) mod documents;
pub(crate) mod merge;

pub(crate) use merge::MergeKey;

/// A thing the editor edits, with a history of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Document {
    /// The scene — the project's world in remote mode, this editor's own ECS otherwise. Its history
    /// is [`crate::actions::remote_undo`] or the local [`crate::undo::UndoStack`]; both are older
    /// than this module and neither lives here.
    World,
    /// A prefab opened in the Inspector, editing its cached document
    /// rather than any instance of it.
    Prefab(Guid),
    /// A material, or any asset registered with `register_reflected_asset!`.
    Asset(Guid),
    /// The input map open in its panel, keyed by file — the panel holds
    /// one at a time, and opening another must not inherit its undo.
    InputMap(PathBuf),
}

/// What kind of document an asset guid names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetKind {
    Prefab,
    Asset,
}

/// The document a chord should reach, given what is focused.
pub(crate) fn resolve(
    focused_tab: Option<EditorTab>,
    selected_asset: Option<(Guid, AssetKind)>,
    input_map: Option<&Path>,
) -> Option<Document> {
    match focused_tab? {
        // The two panels that show the scene. A selected asset does not
        // change that: clicking a material in the Asset Browser and then
        // pressing Ctrl+Z over the viewport is about the viewport.
        EditorTab::World | EditorTab::View => Some(Document::World),
        // The Inspector shows one subject at a time, and the subject is
        // the document — an entity's fields are the world's.
        EditorTab::Inspector => match selected_asset {
            Some((guid, AssetKind::Prefab)) => Some(Document::Prefab(guid)),
            Some((guid, AssetKind::Asset)) => Some(Document::Asset(guid)),
            None => Some(Document::World),
        },
        EditorTab::InputMap => input_map.map(|path| Document::InputMap(path.to_path_buf())),
        _ => None,
    }
}

impl Document {
    /// Whether this is the scene, whose history lives elsewhere.
    pub fn is_world(&self) -> bool {
        matches!(self, Document::World)
    }

    /// What the Edit menu calls it, for the tooltip that says which
    /// history a Ctrl+Z would reach.
    pub fn describe(&self) -> &'static str {
        match self {
            Document::World => "the scene",
            Document::Prefab(_) => "this prefab",
            Document::Asset(_) => "this asset",
            Document::InputMap(_) => "this input map",
        }
    }
}

#[cfg(test)]
mod tests;

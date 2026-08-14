//! Which document a Ctrl+Z belongs to.
//!
//! # Not one history, and not one per panel
//!
//! A single stack for the whole editor is the Unity/Unreal model, and it
//! works there because those editors hold **one** document open: a scene
//! tree in memory, and everything else is a file. This editor holds
//! several at once — the project's world, whatever prefabs have been
//! opened, an input map, a material — and with one stack, two Ctrl+Z in
//! the Inspector can undo a prefab edit and then a rename you made in
//! another panel five minutes ago, which is how an undo history eats work
//! instead of protecting it.
//!
//! A history *per panel* is the other wrong answer. The Inspector is not
//! a document: it edits whatever is selected, so its undo belongs to the
//! entity, the prefab or the material it happens to be showing. Give the
//! panel a stack and it holds edits to three different things.
//!
//! So: **one history per document, chosen by what the focused panel is
//! showing.** This is Godot 4's model — `EditorUndoRedoManager` keys
//! histories by scene, with a global one for everything that belongs to
//! no scene and a separate `REMOTE_HISTORY` for the live-edited remote
//! world. The last one is worth noticing: they hit the same
//! editor-drives-another-process problem and also concluded it needed its
//! own history rather than sharing.
//!
//! # Files are not documents
//!
//! Deliberately absent: renaming, deleting and importing assets. Undoing
//! a file operation is a promise the editor cannot keep — between the
//! delete and the Ctrl+Z there is a filesystem watcher, an importer and
//! whatever else the machine is running. Unity does not put the Project
//! window in its undo stack either. That gap is closed with a
//! confirmation and a trash can, not with a history.

use std::path::{Path, PathBuf};

use kooch_core::Guid;

use crate::state::EditorTab;

pub(crate) mod documents;
pub(crate) mod merge;

pub(crate) use merge::MergeKey;

/// A thing the editor edits, with a history of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Document {
    /// The scene — the project's world in remote mode, this editor's own
    /// ECS otherwise. Its history is [`crate::actions::remote_undo`] or
    /// the local [`crate::undo::UndoStack`]; both are older than this
    /// module and neither lives here.
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
///
/// The Inspector shows both through the same panel, and only the
/// snapshot differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetKind {
    Prefab,
    Asset,
}

/// The document a chord should reach, given what is focused.
///
/// Pure, and separate from the resources it would take to answer it, so
/// the rule can be read in one place and tested without a world — the
/// same shape as [`crate::input_focus::resolve`] and
/// [`crate::shortcuts::allowed`].
///
/// `None` is a panel with nothing to undo: the Console, the Asset
/// Browser (its edits are files), the Build panel.
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

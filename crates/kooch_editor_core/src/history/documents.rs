//! Undo for everything that is not the scene.
//!
//! # Snapshots, not inverses
//!
//! The world's history keeps inverse *edits*, because the world is large
//! and lives in another process — copying it per keystroke is not on the
//! table. A prefab, an input map and a material are none of those
//! things: each is a small value that already knows how to serialise
//! itself, and the whole document fits in the space one inverse edit
//! would take.
//!
//! So these keep the document as it was, and undo puts it back. That is
//! what Unity and Unreal do for everything (they serialise the object
//! into the transaction), and it buys the property that matters here:
//! **there is no per-edit inverse code to forget an arm of.** A new kind
//! of edit to a prefab is undoable the day it is written, without
//! touching this file.
//!
//! # What that costs
//!
//! Memory, in principle: a hundred copies of a document. In practice a
//! prefab is a few kilobytes of RON and the merge rule means a drag is
//! one snapshot rather than sixty. If a document ever gets big enough
//! for this to matter, it is the document that needs looking at.

use std::collections::HashMap;

use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::scene::SceneDocument;

use super::Document;
use super::merge::{self, MergeKey};

/// How deep each document's history goes, matching the world's.
const DEPTH: usize = 100;

/// A document as it was, in whatever form that document takes.
#[derive(Clone)]
pub(crate) enum DocumentState {
    Prefab(SceneDocument),
    InputMap(kooch_input::actions::ActionMap),
    Material(kooch_render::material::Material),
    /// Any asset registered with `register_reflected_asset!`, as the
    /// field values its registration reads.
    AssetFields(Vec<(String, ReflectValue)>),
}

/// One step: what the document was, and what to call putting it back.
struct Snapshot {
    label: String,
    key: Option<MergeKey>,
    state: DocumentState,
}

/// The undo history of one document.
#[derive(Default)]
struct History {
    done: Vec<Snapshot>,
    undone: Vec<Snapshot>,
}

/// Every open document's history, and whether the current run of edits
/// has been closed.
#[derive(Default)]
pub(crate) struct DocumentHistories {
    histories: HashMap<Document, History>,
    /// Set when something ends a continuous edit — a mouse button
    /// released, focus moving. Read once by the next recording and
    /// cleared, so a seal closes exactly one group.
    sealed: bool,
}

impl DocumentHistories {
    /// Ends the current run of edits, so the next one starts a step.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn can_undo(&self, document: &Document) -> bool {
        self.histories
            .get(document)
            .is_some_and(|history| !history.done.is_empty())
    }

    pub fn can_redo(&self, document: &Document) -> bool {
        self.histories
            .get(document)
            .is_some_and(|history| !history.undone.is_empty())
    }

    pub fn undo_description(&self, document: &Document) -> Option<&str> {
        self.histories
            .get(document)?
            .done
            .last()
            .map(|snapshot| snapshot.label.as_str())
    }

    pub fn redo_description(&self, document: &Document) -> Option<&str> {
        self.histories
            .get(document)?
            .undone
            .last()
            .map(|snapshot| snapshot.label.as_str())
    }

    /// Drops a document's history, for when the document itself is gone.
    pub fn forget(&mut self, document: &Document) {
        self.histories.remove(document);
    }
}

/// Files the document's current state before an edit changes it.
///
/// Called at the top of every handler that mutates a document — one
/// line, before the mutation, which is the only place the previous state
/// still exists.
///
/// A `key` that matches the step already on top merges into it: the
/// older snapshot is the one an undo wants, so the new one is *dropped*
/// rather than pushed. That is the whole of the coalescing rule, and it
/// is free here in a way it is not for an inverse-based history.
pub(crate) fn record(
    resources: &mut Resources,
    document: &Document,
    label: &str,
    key: Option<MergeKey>,
) {
    let Some(state) = capture(resources, document) else {
        // Nothing to remember means nothing to undo. A document that
        // could not be read is one the panel should not have been
        // editing, and a step that cannot restore is worse than none.
        tracing::debug!(
            target: "kooch_editor_core::history",
            ?document,
            "not recording an edit to a document that could not be read",
        );
        return;
    };

    if resources.get::<DocumentHistories>().is_none() {
        resources.insert(DocumentHistories::default());
    }
    let Some(histories) = resources.get_mut::<DocumentHistories>() else {
        return;
    };
    let sealed = std::mem::take(&mut histories.sealed);
    let history = histories.histories.entry(document.clone()).or_default();

    if merge::continues(history.done.last().and_then(|step| step.key), key, sealed) {
        return;
    }

    history.done.push(Snapshot {
        label: label.to_owned(),
        key,
        state,
    });
    history.undone.clear();
    while history.done.len() > DEPTH {
        history.done.remove(0);
    }
}

/// Takes one step in a document's history.
///
/// Symmetric by construction: the current state goes onto the opposite
/// stack as it is replaced, so redo is undo run the other way and
/// neither direction has code of its own.
pub(crate) fn step(resources: &mut Resources, document: &Document, undo: bool) -> bool {
    let Some(current) = capture(resources, document) else {
        return false;
    };
    let Some(histories) = resources.get_mut::<DocumentHistories>() else {
        return false;
    };
    let Some(history) = histories.histories.get_mut(document) else {
        return false;
    };
    let taken = match undo {
        true => history.done.pop(),
        false => history.undone.pop(),
    };
    let Some(snapshot) = taken else {
        return false;
    };
    let label = snapshot.label.clone();
    let key = snapshot.key;
    match undo {
        true => history.undone.push(Snapshot {
            label,
            key,
            state: current,
        }),
        false => history.done.push(Snapshot {
            label,
            key,
            state: current,
        }),
    }

    restore(resources, document, snapshot.state);
    true
}

/// Reads a document out of the editor.
fn capture(resources: &mut Resources, document: &Document) -> Option<DocumentState> {
    match document {
        // Its history is the world's, and the world's is elsewhere.
        Document::World => None,
        Document::Prefab(guid) => Some(DocumentState::Prefab(
            cached_document(resources, *guid)?.clone(),
        )),
        Document::InputMap(path) => {
            let open = resources.get::<crate::state::OpenInputMap>()?;
            // Keyed by path so a history cannot be restored into a
            // different map that happens to be open now.
            (open.path == *path).then(|| DocumentState::InputMap(open.map.clone()))
        }
        Document::Asset(guid) => capture_asset(resources, *guid),
    }
}

/// Materials first, then anything with a reflected registration.
///
/// Two paths because materials predate `register_reflected_asset!` and
/// have their own action; the Inspector draws them through a dedicated
/// panel, so they never arrive as reflected fields.
fn capture_asset(resources: &mut Resources, guid: Guid) -> Option<DocumentState> {
    if let Some(material) = material_of(resources, guid) {
        return Some(DocumentState::Material(material));
    }
    let type_name = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|db| db.entry(guid)?.type_name.clone())?;
    let registration = kooch_ecs::reflect::reflected_asset(&type_name)?;
    Some(DocumentState::AssetFields((registration.read)(
        resources, guid,
    )?))
}

/// Puts a document back.
fn restore(resources: &mut Resources, document: &Document, state: DocumentState) {
    match (document, state) {
        (Document::Prefab(guid), DocumentState::Prefab(snapshot)) => {
            if let Some(cached) = cached_document(resources, *guid) {
                *cached = snapshot;
            }
            // The file is behind either way — an undo puts the *document*
            // back, and saving is still the user's decision. Marking it
            // dirty is what keeps the Inspector's save button honest.
            if resources.get::<crate::actions::DirtyPrefabs>().is_none() {
                resources.insert(crate::actions::DirtyPrefabs::default());
            }
            if let Some(dirty) = resources.get_mut::<crate::actions::DirtyPrefabs>() {
                dirty.mark(*guid);
            }
        }
        (Document::InputMap(path), DocumentState::InputMap(snapshot)) => {
            if let Some(open) = resources.get_mut::<crate::state::OpenInputMap>()
                && open.path == *path
            {
                open.map = snapshot;
            }
        }
        (Document::Asset(guid), DocumentState::Material(snapshot)) => {
            crate::actions::handlers::write_material(resources, *guid, &snapshot);
        }
        (Document::Asset(guid), DocumentState::AssetFields(fields)) => {
            let type_name = resources
                .get::<kooch_core::asset_database::AssetDatabase>()
                .and_then(|db| db.entry(*guid)?.type_name.clone());
            let Some(registration) =
                type_name.and_then(|name| kooch_ecs::reflect::reflected_asset(&name))
            else {
                return;
            };
            for (field, value) in fields {
                (registration.write)(resources, *guid, &field, value);
            }
            let path = resources
                .get::<kooch_core::asset_database::AssetDatabase>()
                .and_then(|db| Some(db.entry(*guid)?.path.clone()));
            if let Some(path) = path {
                crate::actions::handlers::persist_asset(resources, *guid, registration, &path);
            }
        }
        // A state that does not match its document is a bug in this
        // module rather than something a user can cause: `capture` is
        // the only producer and it keys off the same enum.
        (document, _) => tracing::error!(
            target: "kooch_editor_core::history",
            ?document,
            "a snapshot was filed under a document of another kind",
        ),
    }
}

/// The cached, possibly-edited document behind a prefab guid.
fn cached_document(resources: &mut Resources, guid: Guid) -> Option<&mut SceneDocument> {
    let mut server = resources.remove::<kooch_core::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<SceneDocument>(guid, resources).ok();
    resources.insert(server);
    resources
        .get_mut::<kooch_core::assets::Assets<SceneDocument>>()?
        .get_mut(handle?)
}

/// The loaded material behind a guid, or `None` if it is not one.
fn material_of(resources: &mut Resources, guid: Guid) -> Option<kooch_render::material::Material> {
    let mut server = resources.remove::<kooch_core::asset_loader::AssetServer>()?;
    let handle = server
        .load_by_guid::<kooch_render::material::Material>(guid, resources)
        .ok();
    resources.insert(server);
    resources
        .get::<kooch_core::assets::Assets<kooch_render::material::Material>>()?
        .get(handle?)
        .cloned()
}

#[cfg(test)]
mod tests;

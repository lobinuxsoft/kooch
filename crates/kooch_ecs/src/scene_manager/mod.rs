//! Tracks which scenes are loaded and centralizes save/load.
//!
//! The runtime container is the **world**; scenes are content loaded into
//! it. [`SceneManager`] is the registry of what is currently open, which
//! file each came from, and which one new entities land in.
//!
//! This used to hold a single `current: Option<PathBuf>`, where loading
//! replaced the world — "the whole world in one section", in the terms of
//! #566. Several scenes can now be open at once, which is what makes
//! "close the station" and "walk away from the station" expressible as
//! different operations.
//!
//! The manager itself is agnostic of which components form a "starter"
//! scene — bootstrapping a default scene file lives in the editor crate,
//! since it is a UI-policy decision (one Camera entity + Sky), not a core
//! ECS concern.

use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::resource::Resources;

use crate::scene::{SceneDocument, SceneError, despawn_scene, spawn_scene_into, sync_scene_to_ecs};

/// One open scene.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedScene {
    /// Identity, matching [`SceneDocument::id`].
    pub id: Guid,
    /// Where it came from, or `None` for a scene never yet saved.
    pub path: Option<PathBuf>,
    /// Whether it has edits not on disk.
    ///
    /// Per scene, not global: with two scenes open, saving one must not
    /// claim the other's edits are safe.
    pub dirty: bool,
}

/// Registry of the scenes currently loaded.
///
/// Inserted as a [`Resources`] entry by [`EcsPlugin`](crate::plugin::EcsPlugin).
/// Both editor and play binary read/write through this resource so callers
/// never touch [`SceneDocument`] directly.
#[derive(Debug)]
pub struct SceneManager {
    scenes: Vec<LoadedScene>,
    active: Option<Guid>,
}

impl Default for SceneManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneManager {
    /// Starts with one unsaved scene open.
    ///
    /// There is always a scene, the way an editor always has one even
    /// before anything is saved. Starting empty would mean entities
    /// created before the first save belong to nothing, so they would be
    /// written to no file and marking the world dirty would have nowhere
    /// to record it.
    pub fn new() -> Self {
        let id = Guid::new_v4();
        Self {
            scenes: vec![LoadedScene {
                id,
                path: None,
                dirty: false,
            }],
            active: Some(id),
        }
    }

    // -- The open set ----------------------------------------------------

    /// Every loaded scene, in the order they were opened.
    pub fn scenes(&self) -> &[LoadedScene] {
        &self.scenes
    }

    /// Identity of the scene new entities are authored into.
    pub fn active_id(&self) -> Option<Guid> {
        self.active
    }

    /// The active scene's entry.
    pub fn active(&self) -> Option<&LoadedScene> {
        self.active.and_then(|id| self.scene(id))
    }

    /// Looks up one open scene.
    pub fn scene(&self, id: Guid) -> Option<&LoadedScene> {
        self.scenes.iter().find(|scene| scene.id == id)
    }

    /// Opens a new empty scene beside the others and makes it active.
    ///
    /// Returns its identity. Unsaved and unnamed: it has no `path` until
    /// somebody saves it, which is exactly the state [`Self::new`]
    /// describes for the scene an editor starts with.
    ///
    /// What "start something new" means when there is already a world
    /// open. An entity has to belong to a scene, so creating one is what
    /// makes "put this somewhere of its own" answerable at all.
    pub fn new_scene(&mut self) -> Guid {
        let id = Guid::new_v4();
        self.scenes.push(LoadedScene {
            id,
            path: None,
            // Nothing to lose yet — it holds nothing. It goes dirty the
            // moment something is authored into it, which is what puts
            // the asterisk on a scene that has never been written.
            dirty: false,
        });
        self.active = Some(id);
        id
    }

    /// Makes `id` the scene new entities land in.
    ///
    /// Returns `false` if no such scene is open, rather than pointing the
    /// active slot at something that does not exist.
    pub fn set_active(&mut self, id: Guid) -> bool {
        let known = self.scene(id).is_some();
        if known {
            self.active = Some(id);
        }
        known
    }

    /// Whether any open scene has unsaved edits.
    ///
    /// What a "you have unsaved changes" prompt should ask, since
    /// [`Self::is_dirty`] only speaks for the active one.
    pub fn any_dirty(&self) -> bool {
        self.scenes.iter().any(|scene| scene.dirty)
    }

    // -- The active scene, for callers that only ever have one -----------

    /// Path of the active scene, if it has one.
    pub fn current(&self) -> Option<&Path> {
        self.active()?.path.as_deref()
    }

    /// Sets the active scene's path without touching the ECS or disk.
    pub fn set_current(&mut self, path: PathBuf) {
        if let Some(scene) = self.active_mut() {
            scene.path = Some(path);
        }
    }

    /// Forgets every open scene and starts a fresh unsaved one.
    ///
    /// Does not touch the ECS. There is still a scene afterwards — see
    /// [`Self::new`].
    pub fn clear_current(&mut self) {
        *self = Self::new();
    }

    pub fn is_dirty(&self) -> bool {
        self.active().is_some_and(|scene| scene.dirty)
    }

    pub fn mark_dirty(&mut self) {
        if let Some(scene) = self.active_mut() {
            scene.dirty = true;
        }
    }

    /// Records that one open scene has edits not on disk.
    ///
    /// Returns `false` if it is not open — an edit to an entity whose
    /// scene nothing has loaded, which is not something to record.
    ///
    /// 🔴 The scene of the entity that changed, not the active one.
    /// [`Self::mark_dirty`] marks whichever scene new entities land in,
    /// which is the wrong scene the moment two are open: editing
    /// something in the second while the first is active would put the
    /// asterisk on the file that did not change, and leave it off the
    /// one that did.
    pub fn mark_scene_dirty(&mut self, id: Guid) -> bool {
        match self.scenes.iter_mut().find(|scene| scene.id == id) {
            Some(scene) => {
                scene.dirty = true;
                true
            }
            None => false,
        }
    }

    pub fn mark_clean(&mut self) {
        if let Some(scene) = self.active_mut() {
            scene.dirty = false;
        }
    }

    fn active_mut(&mut self) -> Option<&mut LoadedScene> {
        let active = self.active?;
        self.scenes.iter_mut().find(|scene| scene.id == active)
    }

    // -- Loading ---------------------------------------------------------

    /// Loads `path`, replacing every non-ephemeral entity and closing every
    /// other open scene.
    ///
    /// "Open this scene and only this scene". Use [`Self::open_additive`]
    /// to add one beside what is already loaded. Ephemeral entities (editor
    /// camera, gizmos…) survive, because [`sync_scene_to_ecs`] honours
    /// [`EphemeralComponents`](crate::ephemeral::EphemeralComponents).
    pub fn load(&mut self, path: &Path, resources: &mut Resources) -> Result<(), SceneError> {
        // 🔴 Through the pack-aware reader, and read once. A packaged
        // game has no `scenes/` directory: its scenes are inside the
        // pack, and reading the disk here is how a shipped game starts
        // empty (#758).
        let bytes = kooch_core::asset_loader::read_game_file(resources, path)?;
        let text = String::from_utf8_lossy(&bytes);
        let doc = SceneDocument::parse(&text)?;
        let needs_id = Self::lacks_stored_id(&text);
        sync_scene_to_ecs(&doc, resources)?;

        self.scenes.clear();
        self.scenes.push(LoadedScene {
            id: doc.id,
            path: Some(path.to_path_buf()),
            // A file written before scenes had identity was just given one.
            // Marking it dirty is what persists that id on the next save;
            // leaving it clean would hand out a different id every load and
            // silently break references into this scene.
            dirty: needs_id,
        });
        self.active = Some(doc.id);
        Ok(())
    }

    /// Loads `path` beside the scenes already open, and makes it active.
    ///
    /// Returns the loaded scene's identity. Loading the same file twice is
    /// refused: two copies would share every entity id, so references into
    /// the scene could not say which copy they meant. Instancing a scene
    /// more than once needs per-instance id remapping, which is its own
    /// piece of work.
    pub fn open_additive(
        &mut self,
        path: &Path,
        resources: &mut Resources,
    ) -> Result<Guid, SceneError> {
        // 🔴 Through the pack-aware reader, and read once. A packaged
        // game has no `scenes/` directory: its scenes are inside the
        // pack, and reading the disk here is how a shipped game starts
        // empty (#758).
        let bytes = kooch_core::asset_loader::read_game_file(resources, path)?;
        let text = String::from_utf8_lossy(&bytes);
        let doc = SceneDocument::parse(&text)?;
        let needs_id = Self::lacks_stored_id(&text);

        if self.scene(doc.id).is_some() {
            return Err(SceneError::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("scene {} is already open", doc.id),
            )));
        }

        spawn_scene_into(&doc, resources)?;
        self.scenes.push(LoadedScene {
            id: doc.id,
            path: Some(path.to_path_buf()),
            dirty: needs_id,
        });
        self.active = Some(doc.id);
        Ok(doc.id)
    }

    /// Closes one scene, despawning only its entities.
    ///
    /// Returns `false` if it was not open. Unsaved edits are discarded —
    /// asking about them is the caller's job, since only the caller can
    /// prompt.
    pub fn close(&mut self, id: Guid, resources: &mut Resources) -> bool {
        if self.scene(id).is_none() {
            return false;
        }
        despawn_scene(id, resources);
        self.scenes.retain(|scene| scene.id != id);
        if self.active == Some(id) {
            self.active = self.scenes.first().map(|scene| scene.id);
        }
        true
    }

    /// Whether a file on disk predates scene identity.
    ///
    /// `SceneDocument::id` defaults on deserialisation, so by the time the
    /// document exists a freshly generated id is indistinguishable from a
    /// stored one. This re-reads the file asking only whether the field is
    /// there.
    ///
    /// Parsed rather than searched for `"id:"`: entity names are free text,
    /// so a scene holding an entity called `grid:floor` would answer yes to
    /// a substring check and never persist the id it was just given.
    fn lacks_stored_id(text: &str) -> bool {
        /// Reads the identity field and ignores everything else.
        #[derive(serde::Deserialize)]
        struct IdProbe {
            #[serde(default)]
            id: Option<Guid>,
        }

        // An unparseable file is not this function's problem — the real
        // load is about to report it properly.
        ron::from_str::<IdProbe>(text).is_ok_and(|probe| probe.id.is_none())
    }

    // -- Saving ----------------------------------------------------------

    /// Saves the active scene to its own path.
    ///
    /// Returns [`SceneError::Io`] with [`std::io::ErrorKind::NotFound`]
    /// when there is no active scene or it has no path — callers should
    /// fall back to [`Self::save_as`].
    pub fn save(&mut self, resources: &mut Resources) -> Result<(), SceneError> {
        let Some(active) = self.active else {
            return Err(Self::no_path());
        };
        self.save_scene(active, resources)
    }

    /// Saves one open scene to its own path.
    pub fn save_scene(&mut self, id: Guid, resources: &mut Resources) -> Result<(), SceneError> {
        let path = self
            .scene(id)
            .and_then(|scene| scene.path.clone())
            .ok_or_else(Self::no_path)?;

        if self.active == Some(id) {
            adopt_unowned(resources, id);
        }
        SceneDocument::from_ecs_scene(resources, id).save(&path)?;
        if let Some(scene) = self.scenes.iter_mut().find(|scene| scene.id == id) {
            scene.dirty = false;
        }
        Ok(())
    }

    /// Saves the active scene to `path` and adopts it.
    pub fn save_as(&mut self, path: PathBuf, resources: &mut Resources) -> Result<(), SceneError> {
        let Some(id) = self.active else {
            return Err(Self::no_path());
        };
        self.save_scene_as(id, path, resources)
    }

    /// Saves one open scene to `path` and adopts it.
    ///
    /// "Save As" for a scene that is not the active one — which a panel
    /// listing several of them needs, since the scene the user
    /// right-clicked is not necessarily the one new entities land in.
    ///
    /// Captures **only that scene's entities**. Saving one scene must
    /// never drag another's into the file: the next load would spawn
    /// them twice, once from each.
    pub fn save_scene_as(
        &mut self,
        id: Guid,
        path: PathBuf,
        resources: &mut Resources,
    ) -> Result<(), SceneError> {
        if self.scene(id).is_none() {
            return Err(SceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("scene {id} is not open"),
            )));
        }

        // Only for the active scene. An entity nothing recorded the
        // origin of belongs in the one being worked in — adopting it
        // into whichever scene happened to be right-clicked would move
        // somebody's work into a file they did not touch.
        if self.active == Some(id) {
            adopt_unowned(resources, id);
        }
        SceneDocument::from_ecs_scene(resources, id).save(&path)?;
        if let Some(scene) = self.scenes.iter_mut().find(|scene| scene.id == id) {
            scene.path = Some(path);
            scene.dirty = false;
        }
        Ok(())
    }

    fn no_path() -> SceneError {
        SceneError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no current scene path; call save_as first",
        ))
    }
}

// ---------------------------------------------------------------------------

/// Hands every entity with no scene of its own to `scene`.
///
/// An entity spawned in the editor has no membership — nothing loaded it
/// from a file. Without this, saving would walk right past everything the
/// user just created and write an empty scene.
///
/// Only ever called for the active scene: "the one you are working in" is
/// the only defensible home for something whose origin nothing recorded.
fn adopt_unowned(resources: &mut Resources, scene: Guid) {
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::component::ComponentRegistry;
    use crate::entity::Entity;
    use crate::ephemeral::EphemeralComponents;
    use crate::scene_member::SceneMember;

    // Ephemeral entities belong to no scene by design — the editor camera
    // is not content, and adopting it would write it into the user's file.
    let ephemeral = resources
        .get::<EphemeralComponents>()
        .cloned()
        .unwrap_or_default();

    let orphans: Vec<Entity> = resources
        .get::<ArchetypeRegistry>()
        .zip(resources.get::<ComponentRegistry>())
        .map(|(archetypes, components)| {
            let owned = components.get_cpu::<SceneMember>();
            archetypes
                .iter_matching(&[])
                .filter(|archetype| !ephemeral.intersects(archetype.components()))
                .flat_map(|archetype| archetype.entities().iter().copied())
                .filter(|entity| owned.is_none_or(|storage| storage.get(*entity).is_none()))
                .collect()
        })
        .unwrap_or_default();

    if orphans.is_empty() {
        return;
    }

    if let Some(components) = resources.get_mut::<ComponentRegistry>() {
        components.register_cpu::<SceneMember>();
        if let Some(storage) = components.get_cpu_mut::<SceneMember>() {
            for &entity in &orphans {
                storage.insert(entity, SceneMember::new(scene));
            }
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        let member_tid = std::any::TypeId::of::<SceneMember>();
        for &entity in &orphans {
            if let Some(current) = archetypes.entity_archetype(entity) {
                let next = archetypes.archetype_after_add_dynamic(current, member_tid);
                archetypes.register_entity(entity, next);
            }
        }
    }
}

#[cfg(test)]
mod tests;

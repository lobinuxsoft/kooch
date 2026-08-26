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

use crate::scene::{
    SceneDocument, SceneError, despawn_scene, spawn_scene_as, spawn_scene_into, sync_scene_to_ecs,
};

/// One open scene.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedScene {
    /// Identity of **this open copy**.
    ///
    /// 🔴 Not the file's. They were one field, which is why opening the
    /// same file twice had to be refused: both copies would claim one
    /// identity and every `(scene, entity)` pair would alias.
    ///
    /// Unity DOTS answers this a level up — instances of a subscene are
    /// "exact copies of each other", and the load hands back a *scene meta
    /// entity* naming the new instance. The entities inside keep the ids
    /// the file gives them; the copies are told apart by the instance,
    /// not by their contents.
    ///
    /// The first copy of a file keeps the file's own id, so a scene that
    /// is only ever opened once behaves exactly as it did — and so does
    /// every reference already written to disk.
    pub id: Guid,
    /// Identity of the **file** this copy came from, when it came from
    /// one.
    ///
    /// What a reference stored on disk names, and what a save writes back
    /// as the document's id. A second copy has a different [`Self::id`]
    /// and the same `source`.
    pub source: Option<Guid>,
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
    /// Bumped whenever the set of loaded scenes changes.
    ///
    /// # 🔴 What reads this, and why it is not a detail
    ///
    /// Caches that key on "the world as it was" cannot notice a scene
    /// being swapped out. The shadow page cache invalidates on a
    /// **movement diff** — which instances moved since last frame — and
    /// **despawning is not moving**: the outgoing scene's entities did
    /// not move, they ceased to exist, and their pages stayed resident
    /// holding geometry that no longer had anything to cast it (#971).
    ///
    /// Loading is the mirror image and just as quiet: newly spawned
    /// entities did not move either, so an additive load's geometry
    /// casts nothing until something else forces a redraw.
    ///
    /// A counter rather than an event, because the reader is a renderer
    /// that runs once a frame and only needs to answer "is this the
    /// world I last drew?" — a question a comparison answers and a
    /// queue of events complicates.
    epoch: u32,
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
                source: None,
                path: None,
                dirty: false,
            }],
            active: Some(id),
            epoch: 0,
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

    /// How many times the set of loaded scenes has changed.
    ///
    /// A renderer compares this against what it last drew: different
    /// means the world was replaced, and any cache keyed on continuity —
    /// a movement diff, a page stamp — is answering about a world that
    /// is gone. See the field's own docs for why movement is not enough.
    pub fn epoch(&self) -> u32 {
        self.epoch
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
        self.epoch = self.epoch.wrapping_add(1);
        let id = Guid::new_v4();
        self.scenes.push(LoadedScene {
            id,
            source: None,
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
        self.epoch = self.epoch.wrapping_add(1);
        // 🔴 At the SOURCE of the count, because a reader far away found
        // it stuck at zero and could not tell "never bumped" from "read
        // from a different manager". One line per load, which is rare.
        tracing::info!(
            target: "kooch_ecs::scene",
            epoch = self.epoch,
            manager = self as *const Self as usize,
            "scene load: the epoch moved",
        );
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
            source: Some(doc.id),
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
    /// Returns the **instance's** identity, which is the file's own the
    /// first time it is opened and a fresh one for every copy after that.
    ///
    /// # Opening the same file twice
    ///
    /// This used to be refused, because a scene's identity *was* its
    /// file's: two copies would claim one id and every `(scene, entity)`
    /// pair would alias.
    ///
    /// Unity DOTS answers it a level up. Instances of a subscene are
    /// "exact copies of each other" — the same bytes loaded again — and
    /// the load hands back a meta entity naming the new instance. The
    /// entities keep the ids the file gives them; the copies are told
    /// apart by the instance, not by anything inside them.
    ///
    /// So the entity half of the pair stays verbatim from disk, which is
    /// what makes a scene reload to exactly the identities it was saved
    /// with, and the scene half says which copy.
    pub fn open_additive(
        &mut self,
        path: &Path,
        resources: &mut Resources,
    ) -> Result<Guid, SceneError> {
        // ⚠️ Additive too, and the temptation to skip it is the trap.
        // Loading beside what is already there destroys nothing, so it
        // reads as safe — but the incoming entities did not *move*
        // either, and a movement diff cannot see something that was
        // never anywhere. Skipped, the new scene casts no shadows until
        // something unrelated forces a redraw (#971).
        self.epoch = self.epoch.wrapping_add(1);
        // 🔴 Through the pack-aware reader, and read once. A packaged
        // game has no `scenes/` directory: its scenes are inside the
        // pack, and reading the disk here is how a shipped game starts
        // empty (#758).
        let bytes = kooch_core::asset_loader::read_game_file(resources, path)?;
        let text = String::from_utf8_lossy(&bytes);
        let doc = SceneDocument::parse(&text)?;
        let needs_id = Self::lacks_stored_id(&text);

        // The first copy keeps the file's id, so a scene opened once
        // behaves exactly as it did — and so does every reference already
        // written to disk naming it.
        let instance = match self.scene(doc.id).is_some() {
            true => Guid::new_v4(),
            false => doc.id,
        };

        spawn_scene_as(&doc, resources, instance)?;
        self.scenes.push(LoadedScene {
            id: instance,
            source: Some(doc.id),
            path: Some(path.to_path_buf()),
            dirty: needs_id,
        });
        self.active = Some(instance);
        Ok(instance)
    }

    /// Every open copy of the file identified by `source`.
    ///
    /// What a reference stored on disk has to be resolved against: it
    /// names a file, and which copy it means is only answerable when
    /// exactly one is open.
    pub fn instances_of(&self, source: Guid) -> impl Iterator<Item = &LoadedScene> {
        self.scenes
            .iter()
            .filter(move |scene| scene.source == Some(source))
    }

    /// Throws away one scene's edits and reads it back from its file.
    ///
    /// Despawns only that scene's entities and loads the file again, so
    /// the other open scenes are untouched. The scene keeps its place in
    /// the open set and stays active if it was.
    ///
    /// Refused for a scene with no file: there is nothing to revert *to*,
    /// and despawning its entities would delete work rather than undo it
    /// — which is the one thing "discard changes" must never be mistaken
    /// for.
    pub fn revert(&mut self, id: Guid, resources: &mut Resources) -> Result<(), SceneError> {
        self.epoch = self.epoch.wrapping_add(1);
        let Some(path) = self.scene(id).and_then(|scene| scene.path.clone()) else {
            return Err(SceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "scene has never been saved; there is nothing to revert to",
            )));
        };

        let bytes = kooch_core::asset_loader::read_game_file(resources, &path)?;
        let text = String::from_utf8_lossy(&bytes);
        // Parsed BEFORE anything is despawned. A file that has been
        // deleted or hand-edited into nonsense would otherwise leave the
        // scene empty and unrecoverable — "discard changes" that discards
        // the scene as well.
        let doc = SceneDocument::parse(&text)?;

        despawn_scene(id, resources);
        spawn_scene_into(&doc, resources)?;

        let was_active = self.active == Some(id);
        if let Some(scene) = self.scenes.iter_mut().find(|scene| scene.id == id) {
            // The document's identity, not the old one: reverting to a
            // file means becoming what the file says, id included.
            scene.id = doc.id;
            scene.dirty = Self::lacks_stored_id(&text);
        }
        if was_active {
            self.active = Some(doc.id);
        }
        Ok(())
    }

    /// Closes one scene, despawning only its entities.
    ///
    /// Returns `false` if it was not open. Unsaved edits are discarded —
    /// asking about them is the caller's job, since only the caller can
    /// prompt.
    pub fn close(&mut self, id: Guid, resources: &mut Resources) -> bool {
        self.epoch = self.epoch.wrapping_add(1);
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
        let as_id = self.scene(id).and_then(|scene| scene.source).unwrap_or(id);
        SceneDocument::from_ecs_instance(resources, id, as_id).save(&path)?;
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
        // The file's identity, not this copy's. A second copy has an
        // instance id of its own, and writing that would rename the file
        // and break every reference naming it.
        let as_id = self.scene(id).and_then(|scene| scene.source).unwrap_or(id);
        SceneDocument::from_ecs_instance(resources, id, as_id).save(&path)?;
        if let Some(scene) = self.scenes.iter_mut().find(|scene| scene.id == id) {
            // Saved to a file, so from here it *is* that file's copy —
            // which matters for "Save As", where a copy becomes the sole
            // instance of a new file.
            scene.source = Some(as_id);
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
        components.register_cpu_reflected::<SceneMember>();
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

//! Tracks the currently-active scene file and centralizes save/load.
//!
//! [`SceneManager`] is a [`Resources`]-owned handle that records which
//! `.ome_scene` file is loaded and whether unsaved edits exist. All scene
//! I/O in the editor and play binary should funnel through it so the rest
//! of the engine has a single source of truth for "what scene am I in?".
//!
//! The manager itself is agnostic of which components form a "starter"
//! scene — bootstrapping a default scene file lives in the editor crate,
//! since it is a UI-policy decision (one Camera entity + Sky), not a core
//! ECS concern.

use std::path::{Path, PathBuf};

use ome_core::resource::Resources;

use crate::scene::{SceneDocument, SceneError, sync_scene_to_ecs};

/// Tracks the active scene path and dirty state.
///
/// Inserted as a [`Resources`] entry by [`EcsPlugin`](crate::plugin::EcsPlugin).
/// Both editor and play binary read/write through this resource so callers
/// never touch [`SceneDocument`] directly.
#[derive(Debug, Default)]
pub struct SceneManager {
    current: Option<PathBuf>,
    dirty: bool,
}

impl SceneManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Path of the active scene, if one has been loaded or saved.
    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    /// Sets the active scene path without touching the ECS or disk.
    pub fn set_current(&mut self, path: PathBuf) {
        self.current = Some(path);
    }

    /// Drops the active scene reference.
    pub fn clear_current(&mut self) {
        self.current = None;
        self.dirty = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Loads `path` into the live ECS, replacing every non-ephemeral entity.
    ///
    /// On success the path becomes the new `current` and `dirty` is cleared.
    /// Ephemeral entities (editor camera, gizmos…) survive the swap because
    /// [`sync_scene_to_ecs`] honors [`EphemeralComponents`](crate::ephemeral::EphemeralComponents).
    pub fn load(&mut self, path: &Path, resources: &mut Resources) -> Result<(), SceneError> {
        let doc = SceneDocument::load(path)?;
        sync_scene_to_ecs(&doc, resources)?;
        self.current = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    /// Saves the current ECS state to `current`.
    ///
    /// Returns [`SceneError::Io`] with [`io::ErrorKind::NotFound`] when there
    /// is no active path — callers should fall back to [`Self::save_as`].
    pub fn save(&mut self, resources: &Resources) -> Result<(), SceneError> {
        let path = self.current.clone().ok_or_else(|| {
            SceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no current scene path; call save_as first",
            ))
        })?;
        let doc = SceneDocument::from_ecs(resources);
        doc.save(&path)?;
        self.dirty = false;
        Ok(())
    }

    /// Saves the current ECS state to `path` and adopts it as `current`.
    pub fn save_as(&mut self, path: PathBuf, resources: &Resources) -> Result<(), SceneError> {
        let doc = SceneDocument::from_ecs(resources);
        doc.save(&path)?;
        self.current = Some(path);
        self.dirty = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::EntityAllocator;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::commands::Commands;
    use crate::component::{Component, ComponentRegistry};
    use crate::ephemeral::EphemeralComponents;
    use crate::query::{AccessTracker, Query};
    use crate::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};

    #[derive(Debug, Clone, PartialEq)]
    struct Health {
        hp: u32,
    }

    impl Component for Health {}

    impl Reflect for Health {
        fn reflect_fields(&self) -> &'static [FieldMeta] {
            static FIELDS: &[FieldMeta] = &[FieldMeta {
                name: "hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                asset_type: "",
            }];
            FIELDS
        }

        fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
            match field {
                "hp" => Some(ReflectValue::U32(self.hp)),
                _ => None,
            }
        }

        fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
            match (field, value) {
                ("hp", ReflectValue::U32(v)) => {
                    self.hp = v;
                    Ok(())
                }
                (other, _) => Err(ReflectError::FieldNotFound(other.into())),
            }
        }

        fn reflect_default() -> Self {
            Health { hp: 100 }
        }
    }

    struct Ephemeral;
    impl Component for Ephemeral {}

    fn setup_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());
        resources.insert(Commands::new());
        resources.insert(EphemeralComponents::new());
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Health>();
        resources
    }

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("ome_scene_manager_test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn save_as_then_load_round_trips() {
        let mut resources = setup_resources();

        // Spawn a single Health entity.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 42 });
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        let mut sm = SceneManager::new();
        let path = tmp_path("round_trip.ome_scene");
        sm.save_as(path.clone(), &resources).unwrap();

        assert_eq!(sm.current(), Some(path.as_path()));
        assert!(!sm.is_dirty());

        // Mutate the live ECS so we can verify load wipes it.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 999 });
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        sm.mark_dirty();
        sm.load(&path, &mut resources).unwrap();

        assert!(!sm.is_dirty(), "load must clear dirty flag");

        let query = Query::<&Health>::new(&resources);
        let healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
        assert_eq!(healths, vec![42]);
    }

    #[test]
    fn save_without_current_returns_not_found() {
        let resources = setup_resources();
        let mut sm = SceneManager::new();
        let err = sm.save(&resources).expect_err("save with no current should fail");
        match err {
            SceneError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io(NotFound), got {other:?}"),
        }
    }

    #[test]
    fn load_preserves_ephemeral_entities() {
        let mut resources = setup_resources();

        // Mark Ephemeral as ephemeral and spawn one of each.
        resources
            .get_mut::<EphemeralComponents>()
            .unwrap()
            .insert(std::any::TypeId::of::<Ephemeral>());

        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 1 });
            commands
                .spawn(&mut resources)
                .insert_reflected(Health { hp: 99 })
                .insert(Ephemeral);
            commands.apply(&mut resources);
            resources.insert(commands);
        }

        // Save a scene containing only the persistent entity (hp=1).
        let mut sm = SceneManager::new();
        let path = tmp_path("ephemeral.ome_scene");
        sm.save_as(path.clone(), &resources).unwrap();

        // Reload — ephemeral hp=99 entity must survive the swap.
        sm.load(&path, &mut resources).unwrap();

        let query = Query::<&Health>::new(&resources);
        let mut healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
        healths.sort();
        assert_eq!(healths, vec![1, 99]);
    }

    #[test]
    fn dirty_lifecycle() {
        let mut sm = SceneManager::new();
        assert!(!sm.is_dirty());
        sm.mark_dirty();
        assert!(sm.is_dirty());
        sm.mark_clean();
        assert!(!sm.is_dirty());
    }
}

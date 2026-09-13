//! Prefabs as assets: a blueprint a game spawns from at runtime.
//!
//! # What a prefab is here
//!
//! A **blueprint**, not a linked instance. It describes an entity, its
//! children, and their components; spawning it builds those entities and
//! the result has no memory of where it came from. Editing the file
//! afterwards does not reach anything already spawned.
//!
//! That is a deliberate model rather than a missing feature. The linked
//! kind — where editing a prefab updates every placed copy, with per-field
//! overrides — is #611 Phase B, and it answers a different question:
//! keeping *authored* instances in step. Nothing about spawning a bullet
//! wants it.
//!
//! # Why the engine ships no spawner component
//!
//! *When* and *where* to spawn is a game's decision, and every game's is
//! different. What the engine owes is the capability and a field type, so a
//! project can write its own:
//!
//! ```ignore
//! #[derive(Reflect)]
//! struct Spawner {
//!     #[reflect(asset = "kooch_ecs::scene::document::SceneDocument")]
//!     prefab: Option<Guid>,
//!     interval: f32,
//! }
//! ```
//!
//! and the Inspector shows a picker filtered to prefabs, the same way
//! `MeshRenderer.mesh` lists meshes.

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetLoader, AssetServer, LoadContext};
use kooch_core::prelude::{AssetError, AssetResult};
use kooch_core::resource::Resources;

use super::document::SceneDocument;
use super::error::SceneError;

/// Reads a `.prefab` file into a [`SceneDocument`].
pub struct PrefabLoader;

impl AssetLoader<SceneDocument> for PrefabLoader {
    fn extensions(&self) -> &[&'static str] {
        &[kooch_core::scene_paths::PREFAB_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<SceneDocument> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(PrefabParseError::Utf8(e))))?;
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(PrefabParseError::Ron(e))))
    }
}

/// Registers [`PrefabLoader`] on `server`.
pub fn register_loader(server: &mut AssetServer) {
    server.register_loader::<SceneDocument, _>(PrefabLoader);
}

/// Writes `document` to `path` and gives it an asset identity.
pub fn save(document: &SceneDocument, path: &std::path::Path) -> Result<Guid, SceneError> {
    document.save(path)?;
    let meta =
        kooch_core::asset_meta::read_or_create_typed(path, std::any::type_name::<SceneDocument>())
            .map_err(|e| SceneError::AssetIdentity {
                detail: e.to_string(),
            })?;
    Ok(meta.guid)
}

/// Spawns the prefab registered under `prefab`, returning its root entity.
pub fn spawn(prefab: Guid, resources: &mut Resources) -> Result<crate::entity::Entity, SceneError> {
    spawn_members(prefab, resources).map(|(root, _)| root)
}

/// Spawns a prefab and hands back its root **and** every entity it built, in document order.
pub fn spawn_members(
    prefab: Guid,
    resources: &mut Resources,
) -> Result<(crate::entity::Entity, Vec<crate::entity::Entity>), SceneError> {
    // 🔴 Only correct when somebody is *there* to answer. A scene load lifts the `SceneManager` out
    // of `Resources` to call `SceneManager::load`, so an instance built during a load asks an empty
    // room and gets a fresh random `Guid` — a scene that exists nowhere, one per instance.
    let into = resources
        .get::<crate::scene_manager::SceneManager>()
        .and_then(|scenes| scenes.active_id())
        .unwrap_or_else(Guid::new_v4);
    spawn_members_into(prefab, resources, into)
}

/// [`spawn_members`], into a scene the caller names.
pub fn spawn_members_into(
    prefab: Guid,
    resources: &mut Resources,
    into: Guid,
) -> Result<(crate::entity::Entity, Vec<crate::entity::Entity>), SceneError> {
    // Taken out and put back so `load_by_guid` can borrow `resources` for
    // the load it may have to perform.
    let mut server = resources
        .remove::<AssetServer>()
        .ok_or(SceneError::NoAssetServer)?;
    let handle = server.load_by_guid::<SceneDocument>(prefab, resources);
    resources.insert(server);

    let handle = handle.map_err(|e| SceneError::PrefabUnavailable {
        prefab,
        detail: e.to_string(),
    })?;

    // Cloned out of the store: `instantiate` needs `&mut Resources`, and the
    // document is borrowed from a resource inside it. A prefab is small
    // relative to the entities it is about to create.
    let document = resources
        .get::<kooch_core::assets::Assets<SceneDocument>>()
        .and_then(|assets| assets.get(handle).cloned())
        .ok_or(SceneError::PrefabUnavailable {
            prefab,
            detail: "loaded but absent from Assets<SceneDocument>".to_owned(),
        })?;

    super::sync::instantiate_members(&document, resources, into)
}

/// Why a `.prefab` file could not be parsed.
#[derive(Debug)]
pub enum PrefabParseError {
    Utf8(std::str::Utf8Error),
    Ron(ron::error::SpannedError),
}

impl std::fmt::Display for PrefabParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Utf8(e) => write!(f, "prefab is not valid UTF-8: {e}"),
            Self::Ron(e) => write!(f, "failed to parse prefab RON: {e}"),
        }
    }
}

impl std::error::Error for PrefabParseError {}

#[cfg(test)]
mod tests;

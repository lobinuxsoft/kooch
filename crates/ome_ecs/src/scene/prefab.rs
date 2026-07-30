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
//!     #[reflect(asset = "ome_ecs::scene::document::SceneDocument")]
//!     prefab: Option<Guid>,
//!     interval: f32,
//! }
//! ```
//!
//! and the Inspector shows a picker filtered to prefabs, the same way
//! `MeshRenderer.mesh` lists meshes.

use ome_core::Guid;
use ome_core::asset_loader::{AssetLoader, AssetServer, LoadContext};
use ome_core::prelude::{AssetError, AssetResult};
use ome_core::resource::Resources;

use super::document::SceneDocument;
use super::error::SceneError;

/// Reads a `.prefab` file into a [`SceneDocument`].
///
/// A prefab and a scene are the same document in the same format; only the
/// extension differs, and it names an invariant — a prefab has exactly one
/// root. Registering a loader for it is what gives prefabs a [`Guid`], so a
/// component field can reference one and [`spawn`] can find it without a
/// path.
///
/// It is also the cache: `Assets<SceneDocument>` holds the parsed document,
/// so stamping out a hundred copies re-reads nothing.
pub struct PrefabLoader;

impl AssetLoader<SceneDocument> for PrefabLoader {
    fn extensions(&self) -> &[&'static str] {
        &[ome_core::scene_paths::PREFAB_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<SceneDocument> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| AssetError::Loader(Box::new(PrefabParseError::Utf8(e))))?;
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(PrefabParseError::Ron(e))))
    }
}

/// Registers [`PrefabLoader`] on `server`.
///
/// A free function rather than something `EcsPlugin` does, because the
/// `AssetServer` is built by the plugin that owns assets and a plugin
/// reaching into a resource another plugin may not have inserted yet is an
/// ordering bug waiting to happen.
pub fn register_loader(server: &mut AssetServer) {
    server.register_loader::<SceneDocument, _>(PrefabLoader);
}

/// Writes `document` to `path` and gives it an asset identity.
///
/// The identity is the point. `AssetDatabase`'s scan registers a file only
/// if a `.meta` sits beside it — it never invents one — so a prefab saved
/// without this is a file the picker cannot list and [`spawn`] cannot find.
/// Writing it here means the identity is created by the act that creates
/// the asset, rather than by whoever happens to load it first.
///
/// Re-saving an existing prefab keeps its [`Guid`], so every component
/// already pointing at it still does.
pub fn save(document: &SceneDocument, path: &std::path::Path) -> Result<Guid, SceneError> {
    document.save(path)?;
    let meta =
        ome_core::asset_meta::read_or_create_typed(path, std::any::type_name::<SceneDocument>())
            .map_err(|e| SceneError::AssetIdentity {
                detail: e.to_string(),
            })?;
    Ok(meta.guid)
}

/// Spawns the prefab registered under `prefab`, returning its root entity.
///
/// This is the runtime entry point — what a project's own spawner calls.
///
/// # Why there is no scene parameter
///
/// [`instantiate`](super::sync::instantiate) takes the scene an instance
/// becomes a member of, which is what saving needs to know. A bullet spawned
/// mid-frame is never saved, and asking a game to name a scene to spawn one
/// would be asking about a concept it has no reason to hold. The active
/// scene is used when there is one.
///
/// # Cost
///
/// The first call for a given prefab reads and parses the file; every one
/// after that is a lookup in `Assets<SceneDocument>` plus the spawn itself.
/// Spawning is proportional to the prefab's entity count, not its file size.
pub fn spawn(prefab: Guid, resources: &mut Resources) -> Result<crate::entity::Entity, SceneError> {
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
        .get::<ome_core::assets::Assets<SceneDocument>>()
        .and_then(|assets| assets.get(handle).cloned())
        .ok_or(SceneError::PrefabUnavailable {
            prefab,
            detail: "loaded but absent from Assets<SceneDocument>".to_owned(),
        })?;

    let into = resources
        .get::<crate::scene_manager::SceneManager>()
        .and_then(|scenes| scenes.active_id())
        .unwrap_or_else(Guid::new_v4);

    super::sync::instantiate(&document, resources, into)
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
mod tests {
    use super::*;

    fn ctx_free_load(bytes: &[u8]) -> AssetResult<SceneDocument> {
        // The context only carries the source path, which this loader does
        // not consult — a prefab's contents are self-describing.
        let mut ctx = LoadContext {
            path: std::path::Path::new("x.prefab"),
        };
        PrefabLoader.load(bytes, &mut ctx)
    }

    /// The extension is the one that names the single-root invariant, not
    /// the scene one — registering both against the same loader would make
    /// every scene show up in a prefab picker.
    #[test]
    fn the_loader_claims_prefabs_and_not_scenes() {
        let extensions = PrefabLoader.extensions();
        assert!(extensions.contains(&ome_core::scene_paths::PREFAB_EXTENSION));
        assert!(!extensions.contains(&ome_core::scene_paths::SCENE_EXTENSION));
    }

    #[test]
    fn a_prefab_round_trips_through_the_loader() {
        let document = SceneDocument {
            id: Guid::new_v4(),
            name: "Ball".into(),
            version: "0.1.0".into(),
            entities: Vec::new(),
        };
        let text = ron::ser::to_string(&document).unwrap();
        let loaded = ctx_free_load(text.as_bytes()).expect("its own output should parse");
        assert_eq!(loaded.name, "Ball");
        assert_eq!(loaded.id, document.id);
    }

    /// A truncated or hand-edited file has to fail as an error rather than
    /// panic: it arrives from disk, so it is input, not a bug.
    #[test]
    fn a_malformed_prefab_is_an_error() {
        assert!(ctx_free_load(b"(id: ").is_err());
        assert!(ctx_free_load(&[0xff, 0xfe]).is_err(), "invalid UTF-8");
    }
}

//! [`AssetPlugin`] — wires the engine's asset infrastructure into the
//! `App`'s `Resources`.
//!
//! At plugin build time the following resources are inserted:
//!
//! - [`AssetServer`](kooch_core::asset_loader::AssetServer) with the
//!   configured asset root and every loader registered (Mesh /
//!   MeshletMesh / Image — extend here when new asset types arrive).
//! - [`AssetDatabase`](kooch_core::asset_database::AssetDatabase)
//!   populated by an initial recursive scan of the asset root. Sidecar
//!   `.meta` files found there register their GUIDs immediately so
//!   scene-side `load_by_guid` lookups work without an explicit prior
//!   `load(path)`.
//! - `Assets<T>` storages for each asset type the loaders produce.
//!
//! The plugin is independent of [`super::RenderPlugin`] — game tools,
//! servers, and headless test harnesses can install asset loading
//! without pulling in the GPU render pipeline.

mod eager;
mod plugin;
#[cfg(test)]
mod tests;

pub use eager::eager_import_with;
pub use plugin::AssetPlugin;

//! Asset loader trait + `AssetServer` resource.
//!
//! Defines the contract every asset loader implements ([`AssetLoader<T>`]),
//! a load-time context the loader uses to side-load dependencies
//! ([`LoadContext`]), and the central registry the engine consults when
//! game code requests an asset by path ([`AssetServer`]).
//!
//! # Pipeline
//!
//! ```text
//! game code ──load("mesh.glb")──▶ AssetServer
//!                                      │
//!                                      ▼
//!                          read bytes from filesystem
//!                                      │
//!                                      ▼
//!                       lookup AssetLoader<Mesh> by TypeId
//!                                      │
//!                                      ▼
//!                          loader.load(bytes, ctx) ─▶ Mesh
//!                                      │
//!                                      ▼
//!                  Assets<Mesh>.insert(mesh) ─▶ Handle<Mesh>
//!                                      │
//!                                      ▼
//!                       cache (path -> handle) for next load
//! ```
//!
//! # Out of scope (other issues)
//!
//! - Concrete loaders (glTF #129, image #131, RON scene): each lives with
//!   its asset type's load issue.
//! - Async / background loading: arrives when streaming demands it.
//! - Hot-reload: file-watcher integration is its own feature.

mod error;
mod server;
mod trait_def;

#[cfg(test)]
mod tests;

pub use error::{AssetError, AssetResult};
pub use server::AssetServer;
pub use trait_def::{AssetLoader, LoadContext};

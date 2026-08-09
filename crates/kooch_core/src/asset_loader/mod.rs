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
//!
//! # Reloading
//!
//! [`asset_written`] is what a save calls, and
//! [`AssetServer::reload_path`] is what it uses. There is deliberately no
//! file watcher: the editor writes these files, so it already knows when
//! they change — see [`written`] for why polling would be both slower and
//! wrong here.

mod error;
mod pack_scan;
mod packs;
mod server;
mod trait_def;
mod written;

// 🔴 Both gated, separately. `#[cfg(test)]` applies to the item that
// follows it and nothing else, so inserting a module between the
// attribute and `mod tests;` left the second one unconditional — it
// compiled here, where the file exists, and broke the vendored engine,
// where test files deliberately do not travel.
#[cfg(test)]
mod pack_tests;
#[cfg(test)]
mod tests;

pub use error::{AssetError, AssetResult};
// Re-exported so crates that configure a pack — the renderer's
// `AssetPlugin`, the facade — need not take the dependency themselves.
pub use kooch_pack::{PackKey, SHARES_ENV, SplitKey, key_from_shares, shares_for_build};
pub use pack_scan::{PackScan, scan_packs};
pub use packs::read_game_file;
pub use server::AssetServer;
pub use trait_def::{AssetLoader, LoadContext};
pub use written::{Written, asset_written};

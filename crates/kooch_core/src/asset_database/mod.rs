//! Project-wide asset registry — `Guid ↔ path` bidirectional map.
//!
//! Mirrors Unity's `AssetDatabase`: the engine scans the project's
//! `assets/` tree at startup, reads every `<file>.meta` sidecar, and
//! caches the resulting `(Guid, path, mtime)` triples. From then on,
//! components and scenes that reference a [`Guid`] can resolve it back
//! to a path without touching the filesystem on the hot path.
//!
//! Scope notes:
//! - Watcher / hot-reload integration is deferred — re-scan on startup
//!   is the contract for now.
//! - Per-type import settings (Unity's "Import Settings" panel) live in
//!   the `.meta` schema's TOML body but are not parsed here yet —
//!   [`AssetMeta`] only exposes `guid` until the type-specific needs
//!   surface.

mod database;
mod entry;
mod error;
mod report;
mod scan;

#[cfg(test)]
mod tests;

pub use database::AssetDatabase;
pub use entry::AssetEntry;
pub use error::AssetDatabaseError;
pub use report::ScanReport;

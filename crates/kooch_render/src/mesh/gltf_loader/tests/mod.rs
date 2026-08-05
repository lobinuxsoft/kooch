//! Test sub-modules for the gltf_loader.
//!
//! Split per meta-issue #495 (file-size ceiling). Grouped by feature
//! domain — basic loader paths vs URI-aware buffer resolution. Shared
//! fixtures live in `helpers.rs` as `pub(super)` so every leaf can
//! reach them via `use super::helpers::*`.
//!
//! Parent `gltf_loader/mod.rs` declares `#[cfg(test)] mod tests;` so
//! everything below is already gated; no extra `#[cfg(test)]` needed.

mod basic;
mod helpers;
mod uri;

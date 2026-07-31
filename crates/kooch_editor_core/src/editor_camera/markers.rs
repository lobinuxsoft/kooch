//! Marker components used by the editor camera entity.
//!
//! These markers are intentionally **not reflected**: they do not appear
//! in the Components panel and cannot be added to user entities through
//! the editor UI. They exist only as ECS bookkeeping for the editor
//! itself.

use kooch_ecs::component::Component;

/// Generic marker for entities that belong to the editor — never persisted
/// to scene files, never wiped on scene load.
///
/// Registered with [`EphemeralComponents`](kooch_ecs::EphemeralComponents)
/// at editor startup so that scene save/load skips and preserves any
/// entity carrying this marker. Future editor-owned entities (gizmos,
/// grid, debug lights) should also carry `EditorOnly` so they ride the
/// same filter without per-type plumbing.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorOnly;

impl Component for EditorOnly {}

/// Specific marker identifying *the* editor's navigation camera entity.
///
/// Used by the input system to locate the camera each frame and by the
/// play/edit toggle to flip its `active` flag without relying on a
/// global handle.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorCamera;

impl Component for EditorCamera {}

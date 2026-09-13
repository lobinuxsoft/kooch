//! Marker components used by the editor camera entity.

use kooch_ecs::component::Component;

/// Generic marker for entities that belong to the editor — never persisted to scene files, never
/// wiped on scene load.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorOnly;

impl Component for EditorOnly {}

/// Specific marker identifying *the* editor's navigation camera entity.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditorCamera;

impl Component for EditorCamera {}

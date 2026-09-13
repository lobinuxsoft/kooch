//! Editor drag-and-drop payload types.

use kooch_core::Guid;
use kooch_ecs::component::ComponentId;

/// Payload dropped by the Components panel onto the World or Inspector.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DraggedComponent(pub ComponentId);

/// Payload dragged from the Asset Browser onto a typed asset slot.
#[derive(Debug, Clone)]
pub(crate) struct DraggedAsset {
    pub guid: Guid,
    pub type_name: String,
}

/// The canonical type name a prefab asset is registered under.
pub(crate) const PREFAB_TYPE_NAME: &str = "kooch_ecs::scene::document::SceneDocument";

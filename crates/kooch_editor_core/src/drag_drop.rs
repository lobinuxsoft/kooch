//! Editor drag-and-drop payload types.
//!
//! egui's `DragAndDrop` system identifies payloads by Rust type, so
//! every logical payload gets its own newtype. Two payloads of different
//! types coexist without interfering (e.g. entity reparent drags use
//! `Entity`; component drops use `DraggedComponent`).

use kooch_core::Guid;
use kooch_ecs::component::ComponentId;

/// Payload dropped by the Components panel onto the World or Inspector.
///
/// Carries the portable [`ComponentId`] of the component the user wants
/// to add to the drop target entity (or entities), so the drop emits a
/// sendable `AddComponent` action.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DraggedComponent(pub ComponentId);

/// Payload dragged from the Asset Browser onto a typed asset slot.
///
/// Carries the canonical type name alongside the GUID so a slot only
/// accepts what it can hold — a mesh dragged over a `Material` field is
/// rejected rather than assigned as a dangling reference.
#[derive(Debug, Clone)]
pub(crate) struct DraggedAsset {
    pub guid: Guid,
    pub type_name: String,
}

/// The canonical type name a prefab asset is registered under.
///
/// `std::any::type_name` of the document a `.prefab` parses into. A drop
/// target compares against this the way an asset slot compares against its
/// own field type, so dragging a mesh into the viewport does not try to
/// instance it.
pub(crate) const PREFAB_TYPE_NAME: &str = "kooch_ecs::scene::document::SceneDocument";

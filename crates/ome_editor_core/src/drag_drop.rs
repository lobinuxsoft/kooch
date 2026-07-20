//! Editor drag-and-drop payload types.
//!
//! egui's `DragAndDrop` system identifies payloads by Rust type, so
//! every logical payload gets its own newtype. Two payloads of different
//! types coexist without interfering (e.g. entity reparent drags use
//! `Entity`; component drops use `DraggedComponent`).

use ome_core::Guid;
use ome_ecs::component::ComponentId;

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

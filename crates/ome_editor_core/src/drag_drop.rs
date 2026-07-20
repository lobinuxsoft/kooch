//! Editor drag-and-drop payload types.
//!
//! egui's `DragAndDrop` system identifies payloads by Rust type, so
//! every logical payload gets its own newtype. Two payloads of different
//! types coexist without interfering (e.g. entity reparent drags use
//! `Entity`; component drops use `DraggedComponent`).

use std::any::TypeId;

use ome_core::Guid;

/// Payload dropped by the Components panel onto the World or Inspector.
///
/// Carries the `TypeId` of the component type the user wants to add to
/// the drop target entity (or entities).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DraggedComponent(pub TypeId);

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

//! Editor drag-and-drop payload types.
//!
//! egui's `DragAndDrop` system identifies payloads by Rust type, so
//! every logical payload gets its own newtype. Two payloads of different
//! types coexist without interfering (e.g. entity reparent drags use
//! `Entity`; component drops use `DraggedComponent`).

use std::any::TypeId;

/// Payload dropped by the Components panel onto the World or Inspector.
///
/// Carries the `TypeId` of the component type the user wants to add to
/// the drop target entity (or entities).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DraggedComponent(pub TypeId);

//! Scene hierarchy — Parent/Children components, GlobalTransform, and systems for keeping them in
//! sync.

pub mod children;
pub mod descendants;
pub mod global_transform;
pub mod hierarchy_sync;
pub mod parent;
pub mod position_only;
mod reparent;
pub mod transform_propagation;

pub use children::Children;
pub use descendants::collect_descendants;
pub use global_transform::GlobalTransform;
pub use hierarchy_sync::hierarchy_sync_system;
pub use parent::Parent;
pub use position_only::{PositionOnly, parent_frame};
pub use reparent::reparent;
pub use transform_propagation::{propagate_subtrees, transform_propagation_system};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

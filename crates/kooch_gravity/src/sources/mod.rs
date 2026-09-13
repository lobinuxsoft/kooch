//! The shapes a gravity field comes in, as separate components — an entity is never two sources, so
//! the archetype answers the query.
//! Fields add; [`GravityPriority`] replaces.

mod area;
mod box_field;
mod global;
mod plane;
mod point;
mod priority;

pub use area::AreaGravity;
pub use box_field::BoxGravity;
pub use global::GlobalGravity;
pub use plane::PlaneGravity;
pub use point::PointGravity;
pub use priority::GravityPriority;

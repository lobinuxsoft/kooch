//! The shapes a gravity field comes in.
//!
//! # Separate components, not one with a discriminant
//!
//! Physics spells its variants as a reflected `u32` — `Collider.shape`,
//! `Joint.kind` — because reflection has no enum representation and a
//! collider is *one* shape at a time. Gravity is different in the way that
//! matters: a scene has many sources at once, they are queried
//! independently, and an entity is never "a point source that is also an
//! area". Separate components let the archetype answer the query instead
//! of a filter over a discriminant, and make an invalid combination
//! unrepresentable rather than merely unlikely.
//!
//! # Fields add
//!
//! Overlapping sources sum, because that is what gravity does. Two planets
//! near each other pull along the vector sum, and the transition between
//! them is smooth without anyone choosing a blending weight — superposition
//! is the blend.
//!
//! What summing does not express is a zone that *replaces*: "inside this
//! room, down is -X, ignore the planet". That wants a priority rather than
//! a weight, and it is [`GravityPriority`] — a separate component, so a
//! scene that never needs one never carries it.
//!
//! # A volume you are inside, a solid you are outside, and a floor
//!
//! [`AreaGravity`] and [`BoxGravity`] are both boxes and are not variants
//! of each other. An area is a *region* with one uniform down — a corridor
//! that runs up a wall — and it acts on whatever is inside it. A box source
//! is a *solid* you stand on the outside of, and its direction differs at
//! every point around it. Merging them would take a flag that inverts the
//! meaning of every other field.
//!
//! [`PlaneGravity`] is neither: bounded in one axis and unbounded in the
//! other two, which is the shape a level floor actually has.

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

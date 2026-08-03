//! [`PhysicsBody`] and [`Collider`] — what an entity is physically.
//!
//! Split by the two components rather than by kind of item: each file
//! carries its struct together with the choice sets and field conditions
//! the Inspector reads for it, so adding a shape or a body kind touches
//! one file.
//!
//! See the [module docs](super) for why a variant is a `u32` discriminant
//! rather than an enum.

mod collider;
mod physics_body;
#[cfg(test)]
mod tests;

pub use collider::*;
pub use physics_body::*;

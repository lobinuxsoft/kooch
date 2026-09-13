//! [`PhysicsBody`] and [`Collider`], each file holding its struct with the choice sets and
//! conditions the Inspector reads. See [module docs](super) for discriminants.

mod collider;
mod physics_body;
#[cfg(test)]
mod tests;

pub use collider::*;
pub use physics_body::*;

//! Gravity fields summed here and applied to Rapier as impulses — a force, never a second solver.
//! Every dynamic body in range is pulled; overlapping fields add, and a [`GravityPriority`] zone
//! overrules the levels below it.

pub mod plugin;
pub mod sources;

pub use plugin::{
    GravityComponentsPlugin, GravityPlugin, gravity_at, gravity_dominant, gravity_up,
};
pub use sources::{
    AreaGravity, BoxGravity, GlobalGravity, GravityPriority, PlaneGravity, PointGravity,
};

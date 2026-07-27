//! Gravity that points somewhere other than down.
//!
//! One global vector is the whole of gravity in most engines, and it is
//! what this one had. A planet needs more: standing on a sphere means down
//! is towards its centre, differently for every body.
//!
//! Rapier has no concept of a gravity field, so this is not a solver
//! feature being exposed — it is acceleration summed here and handed to the
//! solver as an impulse. That stays inside the standing rule: applying a
//! force is using the solver, not running a second one. What must never
//! happen here is integrating a position.
//!
//! # Every dynamic body in range is affected
//!
//! No opt-in component. A body inside a source's reach is pulled by it,
//! which is what gravity means and what an author expects from placing a
//! planet in a scene.
//!
//! # Fields add
//!
//! Overlapping sources sum. Superposition is the blend: two planets pull
//! along the vector sum with no weight for anyone to choose, and a body
//! moving between them transitions smoothly because the arithmetic already
//! says so.

pub mod plugin;
pub mod sources;

pub use plugin::{GravityPlugin, gravity_at};
pub use sources::{AreaGravity, GlobalGravity, PointGravity};

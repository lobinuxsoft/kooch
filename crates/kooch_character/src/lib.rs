//! A character that stands on ground, wherever the ground happens to be.
//!
//! # A floating capsule, and why it is dynamic
//!
//! The capsule **does not touch the floor**. A sphere is swept downward
//! to measure the gap, and a damped spring holds the body at a rest
//! height above whatever it finds. Its orientation is turned to stand on
//! the local up and face where it is being steered.
//!
//! Everything a character needs falls out of that instead of being
//! special-cased:
//!
//! - **Steps and slopes** are climbed because the spring is continuous.
//!   A discrete snap-to-ground has a threshold, and a threshold is
//!   something to be exactly at.
//! - **The body stays dynamic.** It pushes crates, takes hits, and the
//!   gravity acting on it is the real one. A kinematic controller gives
//!   all of that up: it moves *through* the world rather than being part
//!   of it.
//! - **No parasitic friction.** Not touching the floor means no contact
//!   to catch on a ledge or snag on the seam between two colliders.
//! - **Arbitrary gravity is free.** Every term is written against the
//!   local up, so orbiting a planet is not a special case.
//!
//! # Why a sweep and not a ray
//!
//! A ray is a line of zero width. It finds the *lip* of a step instead of
//! the step, slips through the seam between two floor tiles, and misses
//! the edge a body is half standing on. The probe is a sphere for the
//! same reason the character is a capsule: the question is about a shape.
//!
//! # Without a gravity field
//!
//! `gravity_up` answers world up where nothing reaches, so a scene with
//! no sources gets a character that stands upright along `+Y`. That is
//! the right answer and not a fallback — it is what the solver is doing
//! too.

pub mod controller;
pub mod facing;
pub mod grounded;
pub mod jump;
pub mod plugin;
pub mod sprint;
pub mod touching;
pub mod walk;
pub mod wall_slide;

pub use controller::CharacterController;
pub use facing::Facing;
pub use grounded::Grounded;
pub use jump::{Jump, WallJump};
pub use plugin::{CharacterComponentsPlugin, CharacterPlugin};
pub use sprint::Sprint;
pub use touching::Touching;
pub use walk::Walk;
pub use wall_slide::WallSlide;

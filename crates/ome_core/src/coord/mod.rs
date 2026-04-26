//! Hierarchical coordinate system for planet-scale worlds.
//!
//! f32 from a single fixed origin loses meaningful precision past about
//! 5 km — sufficient for a single scene but not for a planet of radius
//! 6000 km. This module splits world position into:
//!
//! - [`UniverseCoord`] — top-level absolute position. Stored as integer
//!   sector + f64 offset within the sector. Survives any distance.
//! - [`LocalCoord`] — position relative to a celestial body (planet,
//!   moon, station). f32 precision is fine within a single planet.
//! - [`CameraRelativeCoord`] — position relative to the active camera.
//!   What the GPU consumes; always near zero, so f32 has full precision.
//!
//! The conversion path is `UniverseCoord → LocalCoord → CameraRelativeCoord`,
//! with widening / narrowing happening at each step. Origin rebasing
//! (re-anchoring `UniverseCoord` to the player's current sector when it
//! drifts too far) keeps the system stable over arbitrarily long play
//! sessions.
//!
//! See `feedback_planet_scale_gpu_driven` (memory) and issue #50.

pub mod local;
pub mod universe;

pub use local::{CelestialBodyRef, LocalCoord};
pub use universe::{SECTOR_HALF, SECTOR_SIZE_METERS, UniverseCoord};

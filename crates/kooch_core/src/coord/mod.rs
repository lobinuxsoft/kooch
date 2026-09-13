//! Hierarchical coordinate system for planet-scale worlds.

pub mod active_origin;
pub mod camera_relative;
pub mod local;
pub mod plugin;
pub mod rebase;
pub mod universe;

pub use active_origin::ActiveOrigin;
pub use camera_relative::CameraRelativeCoord;
pub use local::{CelestialBodyRef, LocalCoord};
pub use plugin::OriginPlugin;
pub use rebase::{DEFAULT_REBASE_THRESHOLD_METERS, RebaseOutcome, check_rebase};
pub use universe::{SECTOR_HALF, SECTOR_SIZE_METERS, UniverseCoord};

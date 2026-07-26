//! [`RapierBackend`] — CPU rigid body solver via Rapier3D 0.22.
//!
//! Owns the Rapier pipeline + body / collider sets. Maps engine
//! [`BodyHandle`]s (slotmap keys) to Rapier's internal handles. All public
//! API uses glam types; nalgebra conversions are confined to this module.
//!
//! # Defaults
//!
//! - Gravity: `(0, -9.81, 0)`. Override via [`RapierBackend::set_gravity`].
//! - Integration parameters: Rapier defaults (60 Hz hint, default solver
//!   iterations, CCD enabled per-body but on-demand).
//! - Friction / restitution coefficients: Rapier defaults (0.5 / 0.0).
//!   PR-1 doesn't expose material override; lands with #137.

mod backend;
mod conv;
#[cfg(feature = "debug-render")]
mod debug;
mod joints;

#[cfg(test)]
mod tests;

pub use backend::RapierBackend;

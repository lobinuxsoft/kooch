//! [`RapierBackend`]: the CPU solver on Rapier3D 0.34, mapping engine [`BodyHandle`](crate::backend::BodyHandle)s (slotmap
//! keys) to Rapier's handles. Gravity `(0, -9.81, 0)`; Rapier's integration defaults; surfaces per
//! collider ([`SurfaceMaterial`](crate::backend::SurfaceMaterial)).

mod backend;
mod conv;
#[cfg(feature = "debug-render")]
mod debug;
mod events;
mod joints;
mod shapes;

#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod tests;

pub use backend::RapierBackend;
pub use shapes::{decompose, hull_of};

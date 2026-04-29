//! Tests for [`crate::sparse::lookup`] — split into three submodules
//! to stay under the no-monolithic threshold:
//!
//! - [`harness`] — shared `run_lookup_probes` infrastructure
//!   (probe-pipeline harness + bounds + readback bundles). GPU tests
//!   use it; the WGSL-only tests pull `PROBE_HARNESS_WGSL` from it for
//!   the default-layout parse case.
//! - [`wgsl`] — CPU/naga parse + validate + constant-drift checks.
//!   Run without a GPU.
//! - [`sampling`] — GPU end-to-end tests asserting the actual lookup
//!   semantics (corner exact, midpoint trilinear, sentinel paths).

mod harness;
mod sampling;
mod wgsl;

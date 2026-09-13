//! [`harness`] runs the probes, [`wgsl`] checks the shader with naga and no GPU, [`sampling`]
//! asserts lookup semantics on the GPU.

mod harness;
mod sampling;
mod wgsl;

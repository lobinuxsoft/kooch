//! Shared lifecycle infrastructure for the build and refit handles.
//!
//! Both [`super::BvhGpuBuild`] and [`super::BvhGpuRefit`] are poll-
//! driven on top of a small set of `AtomicBool` flags wired into wgpu
//! `map_async` callbacks ([`MapState`]). The single non-blocking,
//! GPU-resident view returned to downstream consumers is
//! [`GpuBvhHandle`].

use std::sync::atomic::AtomicBool;

/// Shared state between the orchestrator's `map_async` callbacks and
/// the build / refit `poll`. Atomic loads/stores on the booleans
/// avoid any locking on the hot poll path.
///
/// The `done_*` fields back the `cfg(debug_assertions)` AABB
/// convergence invariant check both handles run before consuming the
/// staging buffers. In release builds nothing ever sets them; the
/// field shape stays identical to keep the struct simple.
#[derive(Default)]
pub(super) struct MapState {
    pub(super) nodes_done: AtomicBool,
    pub(super) indices_done: AtomicBool,
    pub(super) nodes_err: AtomicBool,
    pub(super) indices_err: AtomicBool,
    pub(super) done_done: AtomicBool,
    pub(super) done_err: AtomicBool,
}

/// Lightweight view of a completed (or in-flight + fenced) GPU BVH
/// for downstream traversal kernels. Raymarch culling and broadphase
/// consume this without ever going through CPU readback.
///
/// `nodes_buffer` is a borrow of the build / refit handle's
/// refcounted clone of the builder's nodes buffer — it stays valid
/// for the lifetime of the handle (or longer; the underlying GPU
/// buffer is shared with the builder's reusable storage).
pub struct GpuBvhHandle<'a> {
    pub nodes_buffer: &'a wgpu::Buffer,
    pub n: u32,
}

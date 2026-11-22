//! Meshes a compute pass writes straight into the pool (#313). The producer asks for ranges, the
//! stage grants them and hands back the live buffers, and the producer's pass fills them; the
//! mesh never crosses to the CPU.

use kooch_core::Guid;

use super::pool::{GpuGlobalMeshPool, GpuMeshSizes, MeshBounds, MeshDescriptor};

/// The pool buffers a producer writes into, as they stand after this frame's sync.
#[derive(Clone, Debug)]
pub struct PoolBuffers {
    pub vertices: wgpu::Buffer,
    pub meshlet_vertices: wgpu::Buffer,
    pub meshlet_triangles: wgpu::Buffer,
    pub meshlets: wgpu::Buffer,
}

impl PoolBuffers {
    pub(crate) fn of(pool: &GpuGlobalMeshPool) -> Self {
        Self {
            vertices: pool.vertices.clone(),
            meshlet_vertices: pool.meshlet_vertices.clone(),
            meshlet_triangles: pool.meshlet_triangles.clone(),
            meshlets: pool.meshlets.clone(),
        }
    }
}

/// The channel between a producer and the meshlet stage.
///
/// 🔴 A grant is written the frame after it is made: the buffers that cover it exist once the
/// stage has synced, and the producer's pass must reach the queue before the next sync moves them.
#[derive(Debug, Default)]
pub struct GpuMeshes {
    reserve: Vec<(Guid, GpuMeshSizes, MeshBounds)>,
    release: Vec<Guid>,
    granted: Vec<(Guid, MeshDescriptor)>,
    refused: Vec<Guid>,
    buffers: Option<PoolBuffers>,
    usage: Option<PoolUsage>,
}

/// How full the pool's arrays are against what their bindings accept, so a producer can budget
/// what it asks for (#1968).
#[derive(Clone, Copy, Debug, Default)]
pub struct PoolUsage {
    pub used: GpuMeshSizes,
    pub room: GpuMeshSizes,
}

impl GpuMeshes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for ranges under `guid`, replacing whatever it held. The sweep never frees them; see
    /// [`Self::release`].
    pub fn reserve(&mut self, guid: Guid, sizes: GpuMeshSizes, bounds: MeshBounds) {
        self.reserve.push((guid, sizes, bounds));
    }

    /// Gives `guid`'s ranges back. Its renderer must already have stopped drawing it.
    pub fn release(&mut self, guid: Guid) {
        self.release.push(guid);
    }

    /// The ranges granted since the last call; each descriptor names where its arrays start.
    pub fn take_granted(&mut self) -> Vec<(Guid, MeshDescriptor)> {
        std::mem::take(&mut self.granted)
    }

    /// The asks the pool turned down since the last call: granting them would cross a binding.
    pub fn take_refused(&mut self) -> Vec<Guid> {
        std::mem::take(&mut self.refused)
    }

    /// The pool's fill as of the last sync; `None` before the first.
    pub fn usage(&self) -> Option<PoolUsage> {
        self.usage
    }

    pub(crate) fn refuse(&mut self, guid: Guid) {
        self.refused.push(guid);
    }

    pub(crate) fn publish_usage(&mut self, usage: PoolUsage) {
        self.usage = Some(usage);
    }

    /// The buffers to write the granted ranges into; `None` before the pool first syncs.
    pub fn buffers(&self) -> Option<&PoolBuffers> {
        self.buffers.as_ref()
    }

    /// What the producer asked since the last call: releases first, so reserves reuse their ranges.
    #[allow(clippy::type_complexity)]
    pub(crate) fn take_asked(&mut self) -> (Vec<Guid>, Vec<(Guid, GpuMeshSizes, MeshBounds)>) {
        (
            std::mem::take(&mut self.release),
            std::mem::take(&mut self.reserve),
        )
    }

    pub(crate) fn grant(&mut self, guid: Guid, descriptor: MeshDescriptor) {
        self.granted.push((guid, descriptor));
    }

    /// Points every grant not taken yet at where its mesh now sits; one gone is dropped.
    pub(crate) fn refresh(&mut self, current: impl Fn(Guid) -> Option<MeshDescriptor>) {
        self.granted
            .retain_mut(|(guid, descriptor)| match current(*guid) {
                Some(now) => {
                    *descriptor = now;
                    true
                }
                None => false,
            });
    }

    pub(crate) fn publish(&mut self, buffers: PoolBuffers) {
        self.buffers = Some(buffers);
    }
}

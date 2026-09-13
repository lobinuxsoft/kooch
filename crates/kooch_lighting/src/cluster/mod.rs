//! Clustering (#780): pixels × lights reaching a cell, not every light — the full loop was the
//! OneXFly frame. Cells reserve ranges beyond lights (VSM, fog). [`GpuClusters::update`] sizes,
//! [`GpuClusters::record`] records.

mod buffers;
mod grid;
mod passes;
mod readback;

#[cfg(test)]
mod tests;

use glam::{Mat4, Vec2};

pub use buffers::{ClusterDraw, ClusterViewUniform};
pub use grid::{ClusterGrid, ClusterSettings};

use buffers::ClusterBuffers;
use passes::ClusterPasses;
use readback::ClusterReadback;

/// What the grid needs about its camera; a position-only caller uses [`Self::unclustered`] instead
/// of an identity matrix that clusters a nonexistent camera.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterCamera {
    pub position: glam::Vec3,
    /// World to view, and its projection. `None` means no grid: shading
    /// walks every light, the way it did before #780.
    pub matrices: Option<(Mat4, Mat4)>,
    pub viewport: Vec2,
}

impl ClusterCamera {
    pub fn new(position: glam::Vec3, view: Mat4, proj: Mat4, viewport: Vec2) -> Self {
        Self {
            position,
            matrices: Some((view, proj)),
            viewport,
        }
    }

    /// A camera the grid cannot be built from.
    pub fn unclustered(position: glam::Vec3) -> Self {
        Self {
            position,
            matrices: None,
            viewport: Vec2::ONE,
        }
    }
}

/// The grid's GPU residency, its pipelines, and the frame-late channel
/// that sizes the index list.
pub struct GpuClusters {
    buffers: ClusterBuffers,
    passes: ClusterPasses,
    readback: ClusterReadback,
    grid: ClusterGrid,
    build_bg: Option<wgpu::BindGroup>,
    raster_bg: Option<wgpu::BindGroup>,
    /// The light buffer the bind groups were built against. A grown
    /// light buffer is a replaced one, and a bind group naming the old
    /// one would cluster a buffer nothing shades from.
    lights: Option<wgpu::Buffer>,
    pending_readback: Option<usize>,
    light_count: u32,
}

impl GpuClusters {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffers: ClusterBuffers::new(device),
            passes: ClusterPasses::new(device),
            readback: ClusterReadback::new(device),
            grid: ClusterGrid::new(&ClusterSettings::default(), Vec2::new(1280.0, 720.0)),
            build_bg: None,
            raster_bg: None,
            lights: None,
            pending_readback: None,
            light_count: 0,
        }
    }

    /// The grid this view is being clustered with.
    pub fn grid(&self) -> &ClusterGrid {
        &self.grid
    }

    /// The busiest cell's count and the mean over filled cells (#820); `None` until the async
    /// readback lands, a frame or two in.
    pub fn occupancy(&self) -> Option<(u32, f32)> {
        let draw = self.readback.last()?;
        let filled = draw.filled_cells.max(1);
        Some((draw.peak_cell, draw.index_size as f32 / filled as f32))
    }

    /// Per-cell offsets and counts, for Inti's bind group.
    pub fn cells(&self) -> &wgpu::Buffer {
        &self.buffers.cells
    }

    /// The per-frame view uniform, handed out so outside passes use the grid's own record — a
    /// fourth copy of the slice maths is a fourth way to disagree.
    pub fn view_uniform(&self) -> &wgpu::Buffer {
        &self.buffers.view
    }

    /// The shared index list, for Inti's bind group.
    pub fn indices(&self) -> &wgpu::Buffer {
        &self.buffers.indices
    }

    /// How many indices the list can hold. The shading loop clamps
    /// against it, because an overflowing frame leaves later cells
    /// pointing past the end.
    pub fn index_capacity(&self) -> u32 {
        self.buffers.index_capacity
    }

    /// Sizes the grid and writes what the passes read; `true` when Inti's bind group must rebuild.
    /// Call before the encoder exists.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        settings: &ClusterSettings,
        view: Mat4,
        proj: Mat4,
        viewport: Vec2,
        lights: &wgpu::Buffer,
        light_count: u32,
    ) -> bool {
        // Last frame's copy is mapped now: `map_async` needs its encoder submitted, and a frame
        // later is the cheapest proof.
        self.submit_readback();
        self.readback.drain_ready();
        self.grid = ClusterGrid::new(settings, viewport);
        self.light_count = light_count;

        let cells = self.grid.cluster_count();
        // A light cannot appear in more slices than the grid is deep, so
        // the work list's worst case is exact and needs no readback.
        // Only the index list depends on how the scene is lit.
        let work = light_count.max(1) * self.grid.dimensions.z;
        let mut rebuilt = self.buffers.ensure_capacity(device, cells, work);
        if let Some(draw) = self.readback.last() {
            rebuilt |= self.buffers.ensure_indices(device, draw.index_size);
        }
        // A replaced light buffer invalidates the bind groups the same
        // way one of ours does.
        if rebuilt || self.lights.as_ref() != Some(lights) {
            self.lights = Some(lights.clone());
            self.build_bg = Some(self.passes.build_bind_group(device, &self.buffers, lights));
            self.raster_bg = Some(self.passes.raster_bind_group(device, &self.buffers, lights));
        }
        self.passes
            .ensure_target(device, self.grid.dimensions.x, self.grid.dimensions.y);

        let uniform = ClusterViewUniform::new(&self.grid, view, proj, viewport, light_count)
            .with_capacities(self.buffers.work_capacity, self.buffers.index_capacity);
        queue.write_buffer(&self.buffers.view, 0, bytemuck::bytes_of(&uniform));
        queue.write_buffer(
            &self.buffers.draw,
            0,
            bytemuck::bytes_of(&ClusterDraw::empty()),
        );
        rebuilt
    }

    /// Records the four passes, and the copy that will tell a later
    /// frame how big the index list needed to be.
    pub fn record(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(build_bg), Some(raster_bg)) = (self.build_bg.as_ref(), self.raster_bg.as_ref())
        else {
            return;
        };
        self.passes.record(
            encoder,
            &self.buffers,
            build_bg,
            raster_bg,
            self.light_count,
            self.grid.cluster_count(),
        );
        self.pending_readback = self.readback.record_copy(encoder, &self.buffers.draw);
    }

    /// Hands the pending readback slot to wgpu. Called by
    /// [`Self::update`] a frame after the copy was recorded.
    fn submit_readback(&mut self) {
        if let Some(slot) = self.pending_readback.take() {
            self.readback.submit(slot);
        }
    }
}

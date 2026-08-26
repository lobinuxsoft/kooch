//! **Clustering** — the froxel grid that turns *pixels × every light*
//! into *pixels × the lights that reach this cell* (#780).
//!
//! # Why this exists
//!
//! Until this landed, `inti_shade` looped over every light in the scene
//! for every pixel on screen. Measured on the OneXFly, that loop was the
//! frame: `raster + shade` scaled worse than linearly with resolution
//! because each new pixel paid for the whole light list again, shadow
//! samples included.
//!
//! # What it is
//!
//! The view frustum is diced into a grid of cells, logarithmic along the
//! view axis, and each cell is given the list of lights whose volume
//! reaches it. Shading looks up its own cell and walks that list.
//!
//! 🔴 **The grid is not a light structure.** Reflection probes,
//! irradiance volumes and decals are all bound to a region of space in
//! exactly the same way, and each cell's record has a range reserved for
//! them from the start. It is also the structure virtual shadow maps
//! (#477) mark pages with, and the one volumetric fog (#731) integrates
//! through. Building it once is the point.
//!
//! # The shape of a frame
//!
//! [`GpuClusters::update`] sizes the buffers and writes the view
//! uniform; [`GpuClusters::record`] records the four passes; the shading
//! pass reads the two buffers through Inti's bind group.

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

/// What the grid needs to know about the camera it is being built for.
///
/// One struct rather than four arguments because every caller has all
/// four together, and because a path that has only a position — a
/// headless test, a pass with no projection — says so by using
/// [`Self::unclustered`] rather than by passing an identity matrix that
/// would silently cluster the scene against a camera that does not
/// exist.
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

    /// What the busiest cell held, and the mean over the cells that held
    /// anything (#820).
    ///
    /// `None` until the first readback lands — a frame or two in, which
    /// is what an async readback costs and is why this is a debug
    /// readout rather than anything the frame depends on.
    ///
    /// The mean divides the index list's length by the filled cells, so
    /// it describes the cells that exist instead of being halved by the
    /// empty part of the grid.
    pub fn occupancy(&self) -> Option<(u32, f32)> {
        let draw = self.readback.last()?;
        let filled = draw.filled_cells.max(1);
        Some((draw.peak_cell, draw.index_size as f32 / filled as f32))
    }

    /// Per-cell offsets and counts, for Inti's bind group.
    pub fn cells(&self) -> &wgpu::Buffer {
        &self.buffers.cells
    }

    /// The shared index list, for Inti's bind group.
    /// The per-frame view uniform, so a pass outside this module can
    /// use `cluster_common.wgsl`'s helpers against the **same** record
    /// the grid was built from.
    ///
    /// 🔴 Handing out the buffer rather than the numbers is the point:
    /// `cluster_z_slice` already exists in three copies and the file
    /// says why. A fourth reader that rebuilt the record from its own
    /// matrices would be a fourth chance to disagree with the grid about
    /// which cell a fragment is in.
    pub fn view_uniform(&self) -> &wgpu::Buffer {
        &self.buffers.view
    }

    pub fn indices(&self) -> &wgpu::Buffer {
        &self.buffers.indices
    }

    /// How many indices the list can hold. The shading loop clamps
    /// against it, because an overflowing frame leaves later cells
    /// pointing past the end.
    pub fn index_capacity(&self) -> u32 {
        self.buffers.index_capacity
    }

    /// Sizes the grid for this view and writes everything the passes
    /// read. Returns `true` when a buffer Inti's bind group names was
    /// replaced, which means that bind group has to be rebuilt.
    ///
    /// Call **before** the frame's encoder exists: growing a buffer
    /// replaces it, and a replaced buffer must not be one a recorded
    /// pass already references.
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
        // Last frame's copy is asked for here rather than right after
        // its submit: `map_async` needs the encoder carrying the copy to
        // have been submitted, and being one frame into the next one is
        // the cheapest proof of that there is. It also keeps the render
        // path from having to call anything after its submit.
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

    /// What the GPU reported the last time a readback landed. Reported
    /// by the editor's stats overlay so "the grid overflowed" is
    /// something a person can see rather than something that shows up as
    /// missing light.
    pub fn last_draw(&self) -> Option<ClusterDraw> {
        self.readback.last()
    }
}

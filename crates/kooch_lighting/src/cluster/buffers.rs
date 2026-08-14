//! GPU residency for the froxel grid: the view uniform, the work list,
//! the per-cell records and the shared index list.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2};

use super::grid::ClusterGrid;

/// Indices the list holds before it has to grow.
///
/// Bevy starts at the same number. A cell's run is as long as the lights
/// that reach it, so the total is a property of how the scene is lit,
/// not of how many lights it has — which is why it is measured rather
/// than derived.
pub(super) const INITIAL_INDEX_CAPACITY: u32 = 65_536;

/// Vertices the rasterizer draws per work item: two triangles, with no
/// vertex buffer behind them.
pub(super) const QUAD_VERTICES: u32 = 6;

/// What every clustering pass knows about the view and the grid.
///
/// Mirrors `ClusterView` in `cluster_common.wgsl`. Nothing checks the
/// correspondence at compile time — [`super::tests`] is what stands in
/// for the missing compiler.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClusterViewUniform {
    pub view_from_world: [[f32; 4]; 4],
    pub clip_from_view: [[f32; 4]; 4],
    pub view_from_clip: [[f32; 4]; 4],
    /// The reciprocal of the camera's world scale, per axis.
    pub view_scale: [f32; 4],
    /// xyz = grid dimensions, w = their product.
    pub dimensions: [u32; 4],
    /// xy = the logarithmic slice constants, zw = near and far.
    pub z_factors: [f32; 4],
    /// xy = viewport in pixels, zw = one cell in pixels.
    pub viewport: [f32; 4],
    /// x = lights, y = work-list capacity, z = index-list capacity.
    pub counts: [u32; 4],
}

impl ClusterViewUniform {
    /// Builds the uniform for one view.
    ///
    /// `view` is the camera's world-to-view matrix and `proj` its
    /// projection.
    pub fn new(grid: &ClusterGrid, view: Mat4, proj: Mat4, viewport: Vec2, lights: u32) -> Self {
        let dims = grid.dimensions;
        // The camera's scale, inverted, so a world radius becomes a view
        // radius by multiplication. `world_from_view` is the inverse of
        // the view matrix, and its columns' lengths are that scale.
        let world_from_view = view.inverse();
        let scale = |axis: glam::Vec4| 1.0 / axis.truncate().length().max(1e-6);
        Self {
            view_from_world: view.to_cols_array_2d(),
            clip_from_view: proj.to_cols_array_2d(),
            view_from_clip: proj.inverse().to_cols_array_2d(),
            view_scale: [
                scale(world_from_view.x_axis),
                scale(world_from_view.y_axis),
                scale(world_from_view.z_axis),
                0.0,
            ],
            dimensions: [dims.x, dims.y, dims.z, grid.cluster_count()],
            z_factors: [grid.z_factors.x, grid.z_factors.y, grid.near, grid.far],
            viewport: [
                viewport.x,
                viewport.y,
                viewport.x / dims.x as f32,
                viewport.y / dims.y as f32,
            ],
            counts: [lights, 0, 0, 0],
        }
    }

    /// Stamps the capacities the buffers were actually allocated at.
    pub fn with_capacities(mut self, work_list: u32, index_list: u32) -> Self {
        self.counts[1] = work_list;
        self.counts[2] = index_list;
        self
    }
}

/// The draw arguments the rasterizer runs from, and the two numbers the
/// CPU reads back to size the buffers.
///
/// Mirrors `ClusterDraw`. The first four words are
/// `wgpu::util::DrawIndirectArgs` at offset zero, which is what
/// `draw_indirect` requires.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ClusterDraw {
    pub vertex_count: u32,
    pub instance_count: u32,
    pub first_vertex: u32,
    pub first_instance: u32,
    /// Work items the grid found, uncapped.
    pub wanted: u32,
    /// Indices the grid needs, uncapped.
    pub index_size: u32,
    /// Lights in the busiest cell of the grid (#820).
    ///
    /// The number the lights-per-pixel view (#817) can only be bisected
    /// for by eye, which does not separate 32 from 45. It rides home in
    /// the record that already makes the trip, so it costs one
    /// `atomicMax` in a pass that is already visiting every cell.
    pub peak_cell: u32,
    /// Cells holding at least one light, so the mean is over the cells
    /// that exist rather than over the empty half of the grid.
    pub filled_cells: u32,
}

impl ClusterDraw {
    /// A frame's starting state: six vertices, nothing drawn yet.
    pub fn empty() -> Self {
        Self {
            vertex_count: QUAD_VERTICES,
            ..Default::default()
        }
    }
}

/// Bytes one cell's record takes: an offset and five counts, padded to
/// the 16-byte alignment a `vec4`-shaped struct gets.
const CELL_SIZE: u64 = 32;
/// Bytes one work item takes: object, type, slice.
const WORK_ITEM_SIZE: u64 = 12;

/// The buffers, and the sizes they were allocated at.
pub(super) struct ClusterBuffers {
    pub view: wgpu::Buffer,
    pub draw: wgpu::Buffer,
    pub work_list: wgpu::Buffer,
    pub cells: wgpu::Buffer,
    pub scratch: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    /// Cells the per-cell buffers are sized for.
    pub cell_capacity: u32,
    pub work_capacity: u32,
    pub index_capacity: u32,
}

impl ClusterBuffers {
    pub fn new(device: &wgpu::Device) -> Self {
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_view_ubo"),
            size: std::mem::size_of::<ClusterViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draw = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cluster_draw_args"),
            size: std::mem::size_of::<ClusterDraw>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            view,
            draw,
            work_list: work_buffer(device, 1),
            cells: cell_buffer(device, "cluster_cells", 1),
            scratch: cell_buffer(device, "cluster_scratch", 1),
            indices: index_buffer(device, INITIAL_INDEX_CAPACITY),
            cell_capacity: 1,
            work_capacity: 1,
            index_capacity: INITIAL_INDEX_CAPACITY,
        }
    }

    /// Grows whatever no longer fits, and reports whether anything was
    /// replaced — a replaced buffer means the bind groups that named it
    /// are stale.
    ///
    /// Never shrinks. A scene oscillating around a boundary would
    /// otherwise reallocate every frame, and the memory involved is
    /// kilobytes.
    pub fn ensure_capacity(&mut self, device: &wgpu::Device, cells: u32, work: u32) -> bool {
        let mut rebuilt = false;
        if cells > self.cell_capacity {
            self.cells = cell_buffer(device, "cluster_cells", cells);
            self.scratch = cell_buffer(device, "cluster_scratch", cells);
            self.cell_capacity = cells;
            rebuilt = true;
        }
        if work > self.work_capacity {
            let capacity = work.next_power_of_two();
            self.work_list = work_buffer(device, capacity);
            self.work_capacity = capacity;
            rebuilt = true;
        }
        rebuilt
    }

    /// Grows the index list to `needed`, reporting whether it was
    /// replaced.
    ///
    /// Separate from [`Self::ensure_capacity`] because the number comes
    /// from the GPU a frame or two late, not from the scene walk.
    pub fn ensure_indices(&mut self, device: &wgpu::Device, needed: u32) -> bool {
        if needed <= self.index_capacity {
            return false;
        }
        self.index_capacity = needed.next_power_of_two();
        self.indices = index_buffer(device, self.index_capacity);
        true
    }
}

fn work_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_work_list"),
        size: capacity.max(1) as u64 * WORK_ITEM_SIZE,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    })
}

fn cell_buffer(device: &wgpu::Device, label: &str, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: capacity.max(1) as u64 * CELL_SIZE,
        // COPY_DST because the counting pass accumulates into it, so it
        // has to start at zero every frame — which `clear_buffer` does
        // without a shader. COPY_SRC so a test, or anyone diagnosing a
        // scene lit by nothing, can read the grid back.
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn index_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_index_list"),
        size: capacity.max(1) as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

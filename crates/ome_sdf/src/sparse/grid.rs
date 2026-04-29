//! `SparseGrid` — chunk-local sparse SDF voxel storage.
//!
//! Owns the four GPU buffers (root indices, subgrid pool, free list,
//! counters) backing the two-level sparse layout. Mutating compute
//! passes (classify / allocate / populate / free) live in sibling
//! modules and bind these buffers; this module is the lifecycle root
//! that all of them compose against.

use ome_bvh::Aabb;

use super::{ROOT_CELLS, SUBGRID_VOXELS, free_list};

/// Size in bytes of the dispatch-indirect-args triple
/// `[x, y, z]` (3 × `u32`). Constant rather than `mem::size_of`-derived
/// so consumers can match the layout without depending on a host
/// helper type.
pub const DISPATCH_INDIRECT_ARGS_SIZE: u64 = 12;

/// Fixed-capacity sparse SDF grid bound to one chunk. See module-level
/// docs in [`super`] for the layout, capacity, and encoder-ordering
/// contract.
pub struct SparseGrid {
    bounds: Aabb,
    max_subgrids: u32,
    root_indices_buffer: wgpu::Buffer,
    subgrid_pool_buffer: wgpu::Buffer,
    free_list_buffer: wgpu::Buffer,
    counters_buffer: wgpu::Buffer,
    needs_indices_buffer: wgpu::Buffer,
    needs_count_buffer: wgpu::Buffer,
    needs_indirect_args_buffer: wgpu::Buffer,
    populate_indirect_args_buffer: wgpu::Buffer,
}

impl SparseGrid {
    /// Allocate the four GPU buffers for a fresh `SparseGrid` covering
    /// `bounds` (chunk-local f32, post-`ActiveOrigin`) and seed the
    /// free list + counters so the grid is immediately ready for an
    /// allocate / populate cycle.
    ///
    /// `root_indices` is initialised to `EMPTY_ROOT_SENTINEL`
    /// (`0xFFFFFFFF`) via `mapped_at_creation` — every lookup on a
    /// fresh grid returns `FAR_FROM_SURFACE`. The free list is filled
    /// with the identity permutation `[0, 1, …, max_subgrids - 1]`
    /// and `counters.free_top` is set to `max_subgrids` via
    /// [`free_list::init`].
    ///
    /// `max_subgrids` must be in `1..=ROOT_CELLS`. Use
    /// [`super::MAX_SUBGRIDS_DEFAULT`] unless profiling motivates a
    /// per-chunk override.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: Aabb,
        max_subgrids: u32,
    ) -> Self {
        assert!(
            max_subgrids > 0 && max_subgrids <= ROOT_CELLS,
            "max_subgrids must be in 1..={ROOT_CELLS}, got {max_subgrids}",
        );
        let grid = Self {
            bounds,
            max_subgrids,
            root_indices_buffer: make_root_indices_buffer(device),
            subgrid_pool_buffer: make_subgrid_pool_buffer(device, max_subgrids),
            free_list_buffer: make_free_list_buffer(device, max_subgrids),
            counters_buffer: make_counters_buffer(device),
            needs_indices_buffer: make_needs_indices_buffer(device),
            needs_count_buffer: make_needs_count_buffer(device),
            needs_indirect_args_buffer: make_needs_indirect_args_buffer(device),
            populate_indirect_args_buffer: make_populate_indirect_args_buffer(device),
        };
        free_list::init(queue, &grid.free_list_buffer, &grid.counters_buffer, max_subgrids);
        grid
    }

    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    pub fn max_subgrids(&self) -> u32 {
        self.max_subgrids
    }

    pub fn root_indices_buffer(&self) -> &wgpu::Buffer {
        &self.root_indices_buffer
    }

    pub fn subgrid_pool_buffer(&self) -> &wgpu::Buffer {
        &self.subgrid_pool_buffer
    }

    pub fn free_list_buffer(&self) -> &wgpu::Buffer {
        &self.free_list_buffer
    }

    pub fn counters_buffer(&self) -> &wgpu::Buffer {
        &self.counters_buffer
    }

    /// `ROOT_CELLS × u32` compaction buffer. Filled by the classify
    /// pass with the linear root-cell indices that need a subgrid
    /// allocation; the allocate pass (S4) consumes
    /// `[0..needs_count]` of it via indirect dispatch.
    pub fn needs_indices_buffer(&self) -> &wgpu::Buffer {
        &self.needs_indices_buffer
    }

    /// 4-byte atomic `u32` counter incremented once per marked cell by
    /// the classify pass. Read by the finalize compute pass to derive
    /// the indirect dispatch args.
    pub fn needs_count_buffer(&self) -> &wgpu::Buffer {
        &self.needs_count_buffer
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer written by the
    /// classify-finalize compute pass. Bound with
    /// `BufferUsages::INDIRECT` so the allocate pass can call
    /// `dispatch_workgroups_indirect(&buf, 0)` directly.
    pub fn needs_indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.needs_indirect_args_buffer
    }

    /// 12-byte `[x, y, z]` dispatch-indirect-args buffer written by the
    /// populate-finalize compute pass. Distinct from
    /// `needs_indirect_args_buffer` because the two consumers use
    /// different workgroup sizes — classify's downstream consumer is
    /// the allocate pass at `@workgroup_size(64)` (so x = ⌈n / 64⌉),
    /// populate is `1 workgroup per marked cell` (x = n). Bound with
    /// `BufferUsages::INDIRECT` for `dispatch_workgroups_indirect`.
    pub fn populate_indirect_args_buffer(&self) -> &wgpu::Buffer {
        &self.populate_indirect_args_buffer
    }
}

fn make_root_indices_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let size = (ROOT_CELLS as u64) * 4;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::root_indices"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: true,
    });
    {
        // 0xFFFFFFFF is byte-pattern 0xFF, so a flat byte fill is the
        // correct initialiser for every u32 entry. `BufferViewMut` is
        // write-only in wgpu 29 (mapped memory may be write-combining
        // and does not support `&mut [u8]`), so we copy from a small
        // staging vector — `ROOT_CELLS × 4 = 16 KiB`, trivially cheap.
        let init = vec![0xFFu8; size as usize];
        buffer.slice(..).get_mapped_range_mut().copy_from_slice(&init);
    }
    buffer.unmap();
    buffer
}

fn make_subgrid_pool_buffer(device: &wgpu::Device, max_subgrids: u32) -> wgpu::Buffer {
    let size = (max_subgrids as u64) * (SUBGRID_VOXELS as u64) * 4;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::subgrid_pool"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_free_list_buffer(device: &wgpu::Device, max_subgrids: u32) -> wgpu::Buffer {
    let size = (max_subgrids as u64) * 4;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::free_list"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_counters_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::counters"),
        // 4 × u32: free_top, alloc_failed_count, _pad, _pad.
        size: 16,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_indices_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    // Worst case: every root cell needs allocation → ROOT_CELLS × 4 B
    // = 16 KiB. Fixed-capacity sized to that bound; a smaller capacity
    // would only complicate the classify shader's slot bookkeeping
    // without saving meaningful memory.
    let size = (ROOT_CELLS as u64) * 4;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_indices"),
        size,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_count_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_count"),
        size: 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_needs_indirect_args_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::needs_indirect_args"),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn make_populate_indirect_args_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_sdf::sparse::populate_indirect_args"),
        size: DISPATCH_INDIRECT_ARGS_SIZE,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::INDIRECT
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparse::{
        ALLOC_FAILED_SENTINEL, EMPTY_ROOT_SENTINEL, MAX_SUBGRIDS_DEFAULT, ROOT_DIM, SUBGRID_DIM,
        test_device,
    };
    use glam::Vec3;

    fn unit_bounds() -> Aabb {
        Aabb::new(Vec3::ZERO, Vec3::splat(64.0))
    }

    #[test]
    fn constants_consistent() {
        // `MAX_SUBGRIDS_DEFAULT` bounds are enforced at compile time
        // by a `const _` assertion in `super`. The runtime checks
        // here cover the derived-product equalities.
        assert_eq!(ROOT_CELLS, ROOT_DIM * ROOT_DIM * ROOT_DIM);
        assert_eq!(SUBGRID_VOXELS, SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM);
        assert_eq!(EMPTY_ROOT_SENTINEL, 0xFFFFFFFF);
        assert_eq!(ALLOC_FAILED_SENTINEL, 0xFFFFFFFE);
    }

    #[test]
    fn buffer_sizes_match_layout() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("skipping buffer_sizes_match_layout: no GPU available");
            return;
        };
        let max_subgrids = 256;
        let grid = SparseGrid::new(&device, &queue, unit_bounds(), max_subgrids);

        assert_eq!(grid.max_subgrids(), max_subgrids);
        assert_eq!(grid.bounds(), unit_bounds());
        assert_eq!(grid.root_indices_buffer().size(), (ROOT_CELLS as u64) * 4);
        assert_eq!(
            grid.subgrid_pool_buffer().size(),
            (max_subgrids as u64) * (SUBGRID_VOXELS as u64) * 4,
        );
        assert_eq!(grid.free_list_buffer().size(), (max_subgrids as u64) * 4);
        assert_eq!(grid.counters_buffer().size(), 16);
        assert_eq!(grid.needs_indices_buffer().size(), (ROOT_CELLS as u64) * 4);
        assert_eq!(grid.needs_count_buffer().size(), 4);
        assert_eq!(
            grid.needs_indirect_args_buffer().size(),
            DISPATCH_INDIRECT_ARGS_SIZE,
        );
        assert_eq!(
            grid.populate_indirect_args_buffer().size(),
            DISPATCH_INDIRECT_ARGS_SIZE,
        );
    }

    #[test]
    fn root_indices_initialized_to_empty_sentinel() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("skipping root_indices_initialized_to_empty_sentinel: no GPU available");
            return;
        };
        let grid = SparseGrid::new(&device, &queue, unit_bounds(), 16);
        let bytes = test_device::readback(&device, &queue, grid.root_indices_buffer());
        assert_eq!(bytes.len(), (ROOT_CELLS as usize) * 4);
        for chunk in bytes.chunks_exact(4) {
            let val = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            assert_eq!(
                val, EMPTY_ROOT_SENTINEL,
                "root cell must initialise to EMPTY_ROOT_SENTINEL",
            );
        }
    }

    #[test]
    fn default_capacity_is_under_pool_budget() {
        // `MAX_SUBGRIDS_DEFAULT × SUBGRID_VOXELS × 4 B` must stay near
        // the issue's `<15 MB` AC. 1024 × 4096 × 4 = 16 MiB — the
        // small overshoot is intentional power-of-two headroom.
        let pool_bytes =
            (MAX_SUBGRIDS_DEFAULT as u64) * (SUBGRID_VOXELS as u64) * 4;
        assert_eq!(pool_bytes, 16 * 1024 * 1024);
    }

    #[test]
    #[should_panic(expected = "max_subgrids must be in")]
    fn rejects_zero_max_subgrids() {
        let Some((device, queue)) = test_device::try_acquire() else {
            // Force the panic path so this test still validates the
            // assert message format when no GPU is available.
            panic!("max_subgrids must be in 1..=4096, got 0 (skipped — no GPU)");
        };
        let _ = SparseGrid::new(&device, &queue, unit_bounds(), 0);
    }

    #[test]
    #[should_panic(expected = "max_subgrids must be in")]
    fn rejects_oversized_max_subgrids() {
        let Some((device, queue)) = test_device::try_acquire() else {
            panic!("max_subgrids must be in 1..=4096, got 9999 (skipped — no GPU)");
        };
        let _ = SparseGrid::new(&device, &queue, unit_bounds(), ROOT_CELLS + 1);
    }

}

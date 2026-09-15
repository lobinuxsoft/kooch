//! Each material's tiles for the compute shading path (#1157): a material dispatches only over the
//! tiles it covers, the way Nanite bins shading by material. GPU-only, no readback.

use std::sync::Mutex;

use bytemuck::{Pod, Zeroable, bytes_of};

use super::VBUF64_FORMAT;

const TILE_BINS_SHADER: &str = include_str!("../../../shaders/material_tile_bins.wgsl");

/// Slots the bins address, `MAX_SHADING_SLOTS` on the shading side.
const SLOTS: u64 = 256;
/// One bit per slot.
const WORDS: u64 = 8;
/// Ahead of the list: overflow, tiles per row, each slot's first entry and the total.
const HEADER: u64 = 3 + SLOTS;
/// List entries per tile before binning gives up and every material covers the grid again.
const ENTRIES_PER_TILE: u64 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BinParams {
    size: [u32; 2],
    tiles: [u32; 2],
    shading_rate: u32,
    slots: u32,
    capacity: u32,
    _pad: u32,
}

/// The tile-sized buffers, rebuilt when the grid changes.
struct Grid {
    tiles: (u32, u32),
    bits: wgpu::Buffer,
    bins: wgpu::Buffer,
}

/// What a frame's binning hands the shading pass.
pub(super) struct Binned {
    /// Bound by the compute frame at `tile_bins`.
    pub bins: wgpu::Buffer,
    /// Indirect dispatch arguments, `slot * 12` bytes in.
    pub args: wgpu::Buffer,
}

pub(super) struct TileBins {
    classify: wgpu::ComputePipeline,
    offsets: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    counts: wgpu::Buffer,
    cursor: wgpu::Buffer,
    args: wgpu::Buffer,
    grid: Mutex<Option<Grid>>,
}

impl TileBins {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("material_tile_bins_shader"),
            source: wgpu::ShaderSource::Wgsl(TILE_BINS_SHADER.into()),
        });
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material_tile_bins_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadOnly,
                        format: VBUF64_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<BinParams>() as u64,
                        ),
                    },
                    count: None,
                },
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                storage(8, false),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("material_tile_bins_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        use wgpu::BufferUsages as U;
        Self {
            classify: pipeline("cs_classify"),
            offsets: pipeline("cs_offsets"),
            scatter: pipeline("cs_scatter"),
            bgl,
            params: buffer(
                "material_tile_bins_params",
                std::mem::size_of::<BinParams>() as u64,
                U::UNIFORM | U::COPY_DST,
            ),
            counts: buffer("material_tile_counts", SLOTS * 4, U::STORAGE | U::COPY_DST),
            cursor: buffer("material_tile_cursor", SLOTS * 4, U::STORAGE),
            // COPY_SRC so a test can read what the GPU decided.
            args: buffer(
                "material_tile_args",
                SLOTS * 12,
                U::STORAGE | U::INDIRECT | U::COPY_SRC,
            ),
            grid: Mutex::new(None),
        }
    }

    /// Bins this frame's visibility buffer over a `tiles` grid of the shaded target.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn bin(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        visible_meshlets: &wgpu::Buffer,
        instances: &wgpu::Buffer,
        screen_size: (u32, u32),
        tiles: (u32, u32),
        rate: u32,
        slots: u32,
    ) -> Binned {
        let count = tiles.0 * tiles.1;
        let mut grid = self.grid.lock().unwrap_or_else(|e| e.into_inner());
        if grid.as_ref().is_none_or(|g| g.tiles != tiles) {
            let entries = u64::from(count);
            let buffer = |label: &str, size: u64| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: size * 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            };
            *grid = Some(Grid {
                tiles,
                bits: buffer("material_tile_bits", entries * WORDS),
                bins: buffer("material_tile_bins", HEADER + entries * ENTRIES_PER_TILE),
            });
        }
        let grid = grid.as_ref().expect("sized above");

        queue.write_buffer(
            &self.params,
            0,
            bytes_of(&BinParams {
                size: [screen_size.0, screen_size.1],
                tiles: [tiles.0, tiles.1],
                shading_rate: rate,
                slots: slots.min(SLOTS as u32),
                capacity: count * ENTRIES_PER_TILE as u32,
                _pad: 0,
            }),
        );
        queue.write_buffer(&self.counts, 0, &[0; SLOTS as usize * 4]);

        let entries = [
            wgpu::BindingResource::TextureView(vbuf_view),
            visible_meshlets.as_entire_binding(),
            instances.as_entire_binding(),
            self.params.as_entire_binding(),
            grid.bits.as_entire_binding(),
            self.counts.as_entire_binding(),
            self.cursor.as_entire_binding(),
            grid.bins.as_entire_binding(),
            self.args.as_entire_binding(),
        ];
        let entries: Vec<_> = entries
            .into_iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource,
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material_tile_bins_bg"),
            layout: &self.bgl,
            entries: &entries,
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("material_tile_bins"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_pipeline(&self.classify);
        pass.dispatch_workgroups(tiles.0, tiles.1, 1);
        pass.set_pipeline(&self.offsets);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.scatter);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        drop(pass);

        Binned {
            bins: grid.bins.clone(),
            args: self.args.clone(),
        }
    }
}

#[cfg(test)]
mod tests;

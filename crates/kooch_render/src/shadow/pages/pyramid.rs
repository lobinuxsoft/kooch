//! A hierarchical page pyramid over the sun's clipmap (#1022).

use super::PageConfig;
use crate::shadow::pages::ClipmapConfig;

/// The overlap query over this pyramid: no bindings of its own, so every caller passes the texture
/// it already has. The expansion and the tests include the same text, which is the only arrangement
/// in which a test of it says anything about the frame.
pub const OVERLAP: &str = include_str!("../../../shaders/page_overlap.wgsl");

/// The format is the smallest one every backend accepts as a storage
/// texture: `R8Uint` is not guaranteed writable, and a single bit is
/// what is being stored either way.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// Threads per axis in the seed and reduce dispatches.
const GROUP: u32 = 8;

/// The pyramid, its two pipelines, and the per-mip views they write.
pub struct PagePyramid {
    texture: wgpu::Texture,
    /// One view per mip, for the storage-write side. A storage binding
    /// addresses exactly one mip, so the chain cannot be one view.
    mips: Vec<wgpu::TextureView>,
    /// The whole chain, read with an explicit level by the reduce.
    whole: wgpu::TextureView,
    seed_layout: wgpu::BindGroupLayout,
    reduce_layout: wgpu::BindGroupLayout,
    shape_layout: wgpu::BindGroupLayout,
    seed: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
    side: u32,
    levels: u32,
}

impl PagePyramid {
    /// How many mips a `side x side` grid reduces to, counting mip 0.
    pub fn mip_count(side: u32) -> u32 {
        side.max(1).ilog2() + 1
    }

    pub fn new(device: &wgpu::Device, config: PageConfig, clipmap: ClipmapConfig) -> Self {
        let side = config.side(0);
        let levels = clipmap.levels;
        let mip_count = Self::mip_count(side);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("page_pyramid"),
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: levels,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mips = (0..mip_count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("page_pyramid_mip"),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let whole = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("page_pyramid_all"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let shape_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("page_pyramid_shape_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2Array,
            },
            count: None,
        };
        let seed_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("page_pyramid_seed_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_entry(1),
            ],
        });
        let reduce_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("page_pyramid_reduce_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                storage_entry(1),
            ],
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_pyramid"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    kooch_lighting::PAGE_TABLE,
                    include_str!("../../../shaders/page_pyramid.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = |entry: &str, second: &wgpu::BindGroupLayout| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(&shape_layout), Some(second)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let seed = pipeline("seed_pages", &seed_layout);
        let reduce = pipeline("reduce_mip", &reduce_layout);

        Self {
            texture,
            mips,
            whole,
            seed_layout,
            reduce_layout,
            shape_layout,
            seed,
            reduce,
            side,
            levels,
        }
    }

    /// The whole chain, for whoever asks the overlap question.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.whole
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// Records mip 0 from the page table and every reduction above it.
    pub fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        table: &wgpu::Buffer,
        base: u32,
    ) {
        let groups = |extent: u32| extent.div_ceil(GROUP).max(1);

        // 🔴 A uniform buffer PER MIP, and the mip is inside it.
        let shape_for = |mip: u32, side: u32| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_pyramid_shape"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &buffer,
                0,
                bytemuck::cast_slice(&[side, self.levels, base, mip]),
            );
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_pyramid_shape"),
                layout: &self.shape_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            })
        };
        let shape_group = shape_for(0, self.side);
        let seed_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page_pyramid_seed"),
            layout: &self.seed_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.mips[0]),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("page_pyramid_seed"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.seed);
            pass.set_bind_group(0, &shape_group, &[]);
            pass.set_bind_group(1, &seed_group, &[]);
            pass.dispatch_workgroups(groups(self.side), groups(self.side), self.levels);
        }

        // ⚠️ The uniform is ONE buffer and the passes are recorded into one encoder, so a
        // `write_buffer` per mip inside this loop would be overwritten before any of them ran —
        // `queue.write_buffer` is ordered against the queue, not against the encoder.
        for mip in 1..self.mips.len() as u32 {
            let side = (self.side >> mip).max(1);
            let shape_group = shape_for(mip, side);
            let below = self.texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("page_pyramid_below"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                base_mip_level: mip - 1,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("page_pyramid_reduce"),
                layout: &self.reduce_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&below),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.mips[mip as usize]),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("page_pyramid_reduce"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce);
            pass.set_bind_group(0, &shape_group, &[]);
            pass.set_bind_group(1, &group, &[]);
            pass.dispatch_workgroups(groups(side), groups(side), self.levels);
        }
    }
}

#[cfg(test)]
mod tests;

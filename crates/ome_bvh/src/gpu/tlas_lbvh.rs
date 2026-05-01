//! TLAS Karras LBVH GPU rebuild pipeline (epic #370 PR-1).
//!
//! Mirrors the BLAS [`crate::gpu::lbvh`] pipeline at the algorithmic
//! level — same Karras 2012 algorithm, same workgroup size for the
//! tree-construction passes, same `KarrasConfig` uniform layout — but
//! the leaves are *chunk descriptors* (one per resident chunk) instead
//! of per-primitive AABBs. Output: a flat `2N - 1` [`BvhNode`] array
//! in [`AccelBuffers::tlas_nodes`] byte-identical to what the legacy
//! CPU [`crate::accel::tlas::rebuild`] used to produce.
//!
//! This module lands incrementally across PR-1 commits. Current state
//! (commit 3): Morton encode pipeline only — the rest of the passes
//! land in commits 4..=7.

use std::num::NonZeroU64;

use super::karras_common::KarrasConfig;
use super::types::GpuSceneBounds;

/// Compiled TLAS rebuild pipelines + their uniform staging buffers.
/// Shared across every rebuild dispatch on a given device; safe to
/// reuse from frame to frame because every per-rebuild input
/// (chunk count, scene bounds, scratch buffers) is passed by parameter.
pub struct TlasGpuBuilder {
    pub morton_pipeline: wgpu::ComputePipeline,
    pub morton_bgl: wgpu::BindGroupLayout,
    /// Uniform buffer holding [`GpuSceneBounds`] for the current
    /// rebuild. Written via `queue.write_buffer` at dispatch time.
    pub scene_bounds_buffer: wgpu::Buffer,
    /// Uniform buffer holding the Karras `n` (live chunk count). Same
    /// layout as the BLAS [`crate::gpu::lbvh::LbvhConfig`] so the
    /// later TLAS Karras passes (commits 5..=7) can share it.
    pub config_buffer: wgpu::Buffer,
}

impl TlasGpuBuilder {
    pub fn new(
        device: &wgpu::Device,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> Self {
        let morton_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_bvh::tlas_morton"),
            source: wgpu::ShaderSource::Wgsl(super::TLAS_MORTON_WGSL.into()),
        });
        let morton_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_bvh::tlas_morton_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<GpuSceneBounds>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
            ],
        });
        let morton_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_bvh::tlas_morton_pl"),
            bind_group_layouts: &[Some(&morton_bgl)],
            immediate_size: 0,
        });
        let morton_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_bvh::tlas_morton_pipeline"),
            layout: Some(&morton_pl),
            module: &morton_shader,
            entry_point: Some("tlas_morton_main"),
            compilation_options: Default::default(),
            cache: pipeline_cache,
        });

        let scene_bounds_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::tlas_scene_bounds"),
            size: std::mem::size_of::<GpuSceneBounds>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let config_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::tlas_config"),
            size: std::mem::size_of::<KarrasConfig>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            morton_pipeline,
            morton_bgl,
            scene_bounds_buffer,
            config_buffer,
        }
    }

    /// Pass 0 of the TLAS rebuild: write per-chunk Morton codes into
    /// `tlas_mortons` for the subsequent onesweep sort. Safe for any
    /// `n` — `n == 0` is a no-op (early-out from the orchestrator).
    pub fn dispatch_morton(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        chunk_descriptors: &wgpu::Buffer,
        tlas_mortons: &wgpu::Buffer,
        scene: GpuSceneBounds,
        n: u32,
    ) {
        if n == 0 {
            return;
        }

        queue.write_buffer(&self.scene_bounds_buffer, 0, bytemuck::bytes_of(&scene));
        let cfg = KarrasConfig {
            n,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        };
        queue.write_buffer(&self.config_buffer, 0, bytemuck::bytes_of(&cfg));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_bvh::tlas_morton_bg"),
            layout: &self.morton_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: chunk_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.scene_bounds_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tlas_mortons.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.config_buffer.as_entire_binding(),
                },
            ],
        });
        // Workgroup size 256 matches the BLAS morton pass — keeps the
        // encoding byte-identical and avoids per-vendor tuning surprises.
        const MORTON_WORKGROUP_SIZE: u32 = 256;
        let workgroups = n.div_ceil(MORTON_WORKGROUP_SIZE);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_bvh::tlas_morton_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.morton_pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(workgroups.max(1), 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aabb::Aabb;
    use crate::accel::descriptor::ChunkDescriptor;
    use crate::gpu::builder::test_device;
    use crate::morton::MortonCode;
    use glam::Vec3;
    use wgpu::util::DeviceExt;

    fn descriptor_for(centre: Vec3, half: f32) -> ChunkDescriptor {
        let aabb = Aabb::from_centre(centre, Vec3::splat(half));
        ChunkDescriptor {
            aabb_min: aabb.min.into(),
            first_node: 0,
            aabb_max: aabb.max.into(),
            node_count: 0,
            first_leaf: 0,
            leaf_count: 0,
            first_primitive: 0,
            primitive_count: 0,
            max_smoothness_radius: 0.0,
            _pad: [0.0; 3],
        }
    }

    fn readback_u32(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::Buffer,
        n: u32,
    ) -> Vec<u32> {
        let bytes = (n as u64) * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tlas_morton_test_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tlas_morton_test_readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(src, 0, &staging, 0, bytes);
        queue.submit(std::iter::once(encoder.finish()));
        let slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            sender.send(res).ok();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("device poll failed");
        receiver
            .recv()
            .expect("map_async sender dropped")
            .expect("map_async failed");
        let data = slice.get_mapped_range();
        let v: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
        drop(data);
        staging.unmap();
        v
    }

    #[test]
    fn tlas_morton_byte_identical_to_cpu() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
            return;
        };

        // 16 hand-picked chunks distributed over a 10×10×10 box so the
        // Morton encoding exercises every axis. Centres are deliberately
        // not on integer cell boundaries — that's where CPU/GPU rounding
        // would diverge if the encoders weren't byte-identical.
        let centres: [Vec3; 16] = [
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(9.5, 0.5, 0.5),
            Vec3::new(0.5, 9.5, 0.5),
            Vec3::new(0.5, 0.5, 9.5),
            Vec3::new(9.5, 9.5, 9.5),
            Vec3::new(2.7, 3.3, 4.1),
            Vec3::new(7.1, 1.9, 6.4),
            Vec3::new(1.2, 8.8, 2.5),
            Vec3::new(5.5, 5.5, 5.5),
            Vec3::new(3.0, 6.0, 9.0),
            Vec3::new(0.1, 0.2, 0.3),
            Vec3::new(9.9, 9.8, 9.7),
            Vec3::new(4.4, 4.4, 4.4),
            Vec3::new(6.6, 2.2, 8.8),
            Vec3::new(2.5, 7.5, 5.0),
            Vec3::new(8.0, 1.0, 3.0),
        ];
        let descs: Vec<ChunkDescriptor> =
            centres.iter().map(|c| descriptor_for(*c, 0.4)).collect();
        let aabbs: Vec<Aabb> = centres
            .iter()
            .map(|c| Aabb::from_centre(*c, Vec3::splat(0.4)))
            .collect();
        let n = descs.len() as u32;

        let chunk_descs_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tlas_morton_test_chunk_descriptors"),
            contents: bytemuck::cast_slice(&descs),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let mortons_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tlas_morton_test_mortons"),
            size: (n as u64) * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let scene = GpuSceneBounds::from_aabbs(&aabbs);
        let builder = TlasGpuBuilder::new(&device, None);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tlas_morton_test_encoder"),
        });
        builder.dispatch_morton(
            &device,
            &queue,
            &mut encoder,
            &chunk_descs_buf,
            &mortons_buf,
            scene,
            n,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let gpu_mortons = readback_u32(&device, &queue, &mortons_buf, n);

        // CPU reference: replicate the shader's normalize-then-encode.
        // `GpuSceneBounds::from_aabbs` already handled degenerate axes
        // (inv_extent == 0), so the multiply-by-zero contract holds.
        let inv = Vec3::from_array(scene.inv_extent);
        let scene_min = Vec3::from_array(scene.min);
        let cpu_mortons: Vec<u32> = aabbs
            .iter()
            .map(|a| {
                let centre = a.center();
                let normalized = (centre - scene_min) * inv;
                MortonCode::from_normalized(normalized).0
            })
            .collect();

        assert_eq!(gpu_mortons.len(), cpu_mortons.len());
        for (i, (g, c)) in gpu_mortons.iter().zip(cpu_mortons.iter()).enumerate() {
            assert_eq!(
                g, c,
                "GPU/CPU Morton diverge at chunk[{i}]: gpu={g:#010x} cpu={c:#010x}",
            );
        }
    }
}

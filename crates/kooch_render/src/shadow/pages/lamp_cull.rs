//! One hierarchical cull for every lamp (#939).
//!
//! Olsson et al. 2014 (§3.4/§5.2) adapted to the meshlet pool: a
//! light/instance pre-pass walks the hierarchy the scene already has,
//! and the meshlet-domain passes run only over the pairs it emits.
//! Every lamp shares four dispatches — there is no per-lamp cull
//! object, no per-lamp bind group and no CPU loop, which is what
//! retired the `MeshletCull`-per-lamp shape this replaced.
//!
//! The passes are view-independent (the frustum is the light's own
//! range, the LOD is measured from its position), so the whole thing
//! records ONCE per frame and every camera consumes the same
//! survivors. See `lamp_cull.wgsl` for the shape of each pass.

use crate::meshlet::GpuGlobalMeshPool;

use super::raster::{LAMP_CULLS, buffer_entry, entry, uniform_entry};

use kooch_lighting::CLUSTER_COMMON;
use kooch_lighting::PAGE_TABLE as TABLE;
const LAMP: &str = include_str!("../../../shaders/lamp_cull.wgsl");

/// Survivors one lamp may keep — its fixed slice of the shared arena.
/// Fixed rather than prefix-summed so the cull is one pass with no
/// scan; the count climbs past the cap on purpose so an overflowing
/// lamp is a number in the panel, and every reader clamps. Mirrors
/// `LAMP_SURVIVORS` in `page_table.wgsl`.
pub const LAMP_SURVIVORS: u32 = 4096;

/// Light/instance pairs the pre-pass may emit. Past it, pairs are
/// counted into the header's second word rather than silently lost.
const PAIR_CAPACITY: u32 = 16384;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LampUniform {
    scene: [u32; 4],
    arena: [u32; 4],
    lod: [f32; 4],
}

/// What the bind group was built over, compared per frame the way the
/// raster's `BoundKeys` are: a grown or replaced buffer must rebuild
/// the group or the pass reads freed memory.
#[derive(PartialEq)]
struct LampKeys {
    lights: wgpu::Buffer,
    instances: wgpu::Buffer,
    descriptors: wgpu::Buffer,
    meshlets: wgpu::Buffer,
    bounds: wgpu::Buffer,
    counts: wgpu::Buffer,
    group_err: wgpu::Buffer,
}

struct LampBound {
    pairs: wgpu::BindGroup,
    args: wgpu::BindGroup,
    main: wgpu::BindGroup,
    keys: LampKeys,
}

/// The four pipelines and the buffers between them.
pub struct LampCull {
    uniform: wgpu::Buffer,
    /// Header (count, overflow) then two words per pair.
    pairs: wgpu::Buffer,
    args: wgpu::Buffer,
    /// `[slot * group_capacity + group]` — the #465 reduction's arena,
    /// one row per lamp. Grown with the scene; the growth is the
    /// stated ceiling on [`LAMP_CULLS`].
    group_err: wgpu::Buffer,
    group_capacity: u32,
    /// Arena rows — the most active lights any frame has had.
    group_rows: u32,
    /// `[slot * LAMP_SURVIVORS ..]` — every lamp's packed survivors.
    survivors: wgpu::Buffer,

    pairs_bgl: wgpu::BindGroupLayout,
    args_bgl: wgpu::BindGroupLayout,
    main_bgl: wgpu::BindGroupLayout,
    pairs_pass: wgpu::ComputePipeline,
    args_pass: wgpu::ComputePipeline,
    err_pass: wgpu::ComputePipeline,
    cull_pass: wgpu::ComputePipeline,

    bound: Option<LampBound>,
}

impl LampCull {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lamp_cull"),
            source: wgpu::ShaderSource::Wgsl(format!("{CLUSTER_COMMON}\n{TABLE}\n{LAMP}").into()),
        });
        let c = wgpu::ShaderStages::COMPUTE;
        let pairs_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lamp_pairs_bgl"),
            entries: &[
                uniform_entry(0, false, c),
                buffer_entry(1, true, c),
                buffer_entry(2, true, c),
                buffer_entry(3, true, c),
                buffer_entry(4, false, c),
            ],
        });
        let args_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lamp_args_bgl"),
            entries: &[
                uniform_entry(0, false, c),
                buffer_entry(4, false, c),
                buffer_entry(5, false, c),
            ],
        });
        // Shared by the error and cull passes — eight storage buffers,
        // which is the whole downlevel budget for the stage.
        let main_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lamp_main_bgl"),
            entries: &[
                uniform_entry(0, false, c),
                buffer_entry(1, true, c),
                buffer_entry(3, true, c),
                buffer_entry(4, false, c),
                buffer_entry(6, true, c),
                buffer_entry(7, true, c),
                buffer_entry(8, false, c),
                buffer_entry(9, false, c),
                buffer_entry(10, false, c),
            ],
        });
        let pipeline = |label: &str, entry: &str, bgl: &wgpu::BindGroupLayout| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pairs_pass = pipeline("lamp_cull_pairs", "cs_lamp_pairs", &pairs_bgl);
        let args_pass = pipeline("lamp_cull_args", "cs_lamp_args", &args_bgl);
        let err_pass = pipeline("lamp_cull_err", "cs_lamp_err", &main_bgl);
        let cull_pass = pipeline("lamp_cull_cull", "cs_lamp_cull", &main_bgl);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Self {
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lamp_cull_uniform"),
                size: std::mem::size_of::<LampUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pairs: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lamp_cull_pairs"),
                size: (2 + PAIR_CAPACITY as u64 * 2) * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lamp_cull_args"),
                size: 12,
                usage: storage | wgpu::BufferUsages::INDIRECT,
                mapped_at_creation: false,
            }),
            group_err: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lamp_cull_group_err"),
                size: LAMP_CULLS as u64 * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            group_capacity: 1,
            group_rows: 1,
            survivors: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lamp_cull_survivors"),
                size: LAMP_CULLS as u64 * LAMP_SURVIVORS as u64 * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
            pairs_bgl,
            args_bgl,
            main_bgl,
            pairs_pass,
            args_pass,
            err_pass,
            cull_pass,
            bound: None,
        }
    }

    /// The shared survivor arena, for the expansion's bind group.
    pub fn survivors(&self) -> &wgpu::Buffer {
        &self.survivors
    }

    /// Grows the group-error arena to the scene AND the frame's active
    /// lights. Group slots are bounded by the cull thread count, the
    /// same over-approximation `MeshletCull::ensure_group_capacity`
    /// uses; rows are the lights the frame actually has, NOT
    /// [`LAMP_CULLS`] — a 256-slot cap over an empty scene must not
    /// cost 256 rows of arena.
    fn ensure_groups(&mut self, device: &wgpu::Device, groups: u32, slots: u32) {
        let groups = groups.max(1);
        let slots = slots.clamp(1, LAMP_CULLS);
        if groups <= self.group_capacity && slots <= self.group_rows {
            return;
        }
        self.group_capacity = groups.max(self.group_capacity);
        self.group_rows = slots.max(self.group_rows);
        self.group_err = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lamp_cull_group_err"),
            size: self.group_rows as u64 * self.group_capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.bound = None;
    }

    /// Records the four passes. Once per FRAME — nothing here depends
    /// on a camera — with the survivors and `visible_counts` left for
    /// every view's expansion to read.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        mesh_pool: &GpuGlobalMeshPool,
        instances: &wgpu::Buffer,
        lights: &wgpu::Buffer,
        visible_counts: &wgpu::Buffer,
        sun_buckets: u32,
        lamp_slots: u32,
        instance_count: u32,
        meshlets_per_mesh: u32,
        lod_target: f32,
    ) {
        self.ensure_groups(
            device,
            instance_count.saturating_mul(meshlets_per_mesh),
            lamp_slots,
        );
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&LampUniform {
                scene: [
                    instance_count.max(1),
                    meshlets_per_mesh.max(1),
                    lamp_slots.min(LAMP_CULLS),
                    PAIR_CAPACITY,
                ],
                arena: [self.group_capacity, sun_buckets, 0, 0],
                lod: [
                    lod_target.max(0.01),
                    0.5 * super::LOCAL_MAX_TEXELS as f32,
                    0.0,
                    0.0,
                ],
            }),
        );
        // The header, the arena and the lamps' span of the counts —
        // cleared HERE, per frame, so the per-view raster clears (which
        // cover the sun's span only) never wipe what both views share.
        encoder.clear_buffer(&self.pairs, 0, Some(8));
        encoder.clear_buffer(&self.group_err, 0, None);
        encoder.clear_buffer(
            visible_counts,
            sun_buckets as u64 * 4,
            Some(LAMP_CULLS as u64 * 4),
        );

        self.ensure_bound(device, mesh_pool, instances, lights, visible_counts);
        let bound = self.bound.as_ref().expect("just built");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow pages: lamp cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pairs_pass);
        pass.set_bind_group(0, &bound.pairs, &[]);
        pass.dispatch_workgroups(
            (lamp_slots.min(LAMP_CULLS) * instance_count.max(1)).div_ceil(64),
            1,
            1,
        );
        pass.set_pipeline(&self.args_pass);
        pass.set_bind_group(0, &bound.args, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.err_pass);
        pass.set_bind_group(0, &bound.main, &[]);
        pass.dispatch_workgroups_indirect(&self.args, 0);
        pass.set_pipeline(&self.cull_pass);
        pass.dispatch_workgroups_indirect(&self.args, 0);
    }

    fn ensure_bound(
        &mut self,
        device: &wgpu::Device,
        mesh_pool: &GpuGlobalMeshPool,
        instances: &wgpu::Buffer,
        lights: &wgpu::Buffer,
        visible_counts: &wgpu::Buffer,
    ) {
        let keys = LampKeys {
            lights: lights.clone(),
            instances: instances.clone(),
            descriptors: mesh_pool.mesh_descriptors.clone(),
            meshlets: mesh_pool.meshlets.clone(),
            bounds: mesh_pool.mesh_bounds.clone(),
            counts: visible_counts.clone(),
            group_err: self.group_err.clone(),
        };
        if self.bound.as_ref().is_some_and(|b| b.keys == keys) {
            return;
        }
        self.bound = Some(LampBound {
            pairs: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lamp_pairs_bg"),
                layout: &self.pairs_bgl,
                entries: &[
                    entry(0, &self.uniform),
                    entry(1, lights),
                    entry(2, &keys.bounds),
                    entry(3, instances),
                    entry(4, &self.pairs),
                ],
            }),
            args: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lamp_args_bg"),
                layout: &self.args_bgl,
                entries: &[
                    entry(0, &self.uniform),
                    entry(4, &self.pairs),
                    entry(5, &self.args),
                ],
            }),
            main: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lamp_main_bg"),
                layout: &self.main_bgl,
                entries: &[
                    entry(0, &self.uniform),
                    entry(1, lights),
                    entry(3, instances),
                    entry(4, &self.pairs),
                    entry(6, &keys.descriptors),
                    entry(7, &keys.meshlets),
                    entry(8, &self.group_err),
                    entry(9, &self.survivors),
                    entry(10, visible_counts),
                ],
            }),
            keys,
        });
    }
}

//! Integration tests for #493: atomic R64 visibility buffer winner-takes-all.
//!
//! Skips on adapters that lack the `TEXTURE_INT64_ATOMIC | SHADER_INT64 |
//! SHADER_INT64_ATOMIC_MIN_MAX` feature bundle so headless CI without int64
//! atomics still passes. On supported HW (RDNA 2+ via radv on Linux, RDNA 4
//! desktop, RDNA 3 OneXFly handheld) it verifies the two invariants the
//! production raster shader relies on:
//!
//! 1. **Closer-fragment depth wins.** Under reversed-Z the bit pattern of
//!    `f32` depth is monotonically ordered, so packing it in the high 32
//!    bits makes `textureAtomicMax` deterministically pick the closest
//!    fragment per pixel. This is the load-bearing invariant that fixes
//!    the coplanar z-fighting visible since #491's dragon import.
//!
//! 2. **Equal-depth tie-break is larger packed_ids.** Mirrors Bevy: at
//!    identical depth bits the larger `(slot << 7 | tri)` wins under
//!    `atomicMax`. The integration tests assert this direction so the
//!    deferred shader's unpack matches the raster's pack.

use ome_render::vbuf64::{pack_visibility, unpack_visibility};
use std::sync::mpsc;

const SHADER_SOURCE: &str = r#"
@group(0) @binding(0) var vbuf: texture_storage_2d<r64uint, atomic>;

struct Inputs {
    v1: u64,
    v2: u64,
}

@group(0) @binding(1) var<storage, read> inp: Inputs;

@compute @workgroup_size(1, 1, 1)
fn cs_atomic_max_two() {
    textureAtomicMax(vbuf, vec2<u32>(0u, 0u), inp.v1);
    textureAtomicMax(vbuf, vec2<u32>(0u, 0u), inp.v2);
}
"#;

/// Acquires a wgpu device with the int64-atomic feature bundle (#493).
/// Returns `None` when the adapter does not advertise the bundle, so the
/// test skips cleanly on adapters / backends that lack it (Mac/MSL has
/// no `atomic_uint64`, older drivers, etc.).
fn try_acquire_device_vbuf64() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;

    // Atomic storage textures need TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
    // as the gating feature on top of the int64-atomic bundle. The
    // production GpuContext requests it as a hard-required feature, so
    // any device that runs the engine has it.
    // `StorageTextureAccess::Atomic` is gated by Features::TEXTURE_ATOMIC
    // (wgpu 29 names; the validation error message names
    // TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES but the actual gate is
    // TEXTURE_ATOMIC + format-specific features). The R64 atomic format
    // additionally requires TEXTURE_INT64_ATOMIC; the int64 max/min
    // shader op needs SHADER_INT64_ATOMIC_MIN_MAX; the u64 type itself
    // needs SHADER_INT64.
    let needed = wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(needed) {
        return None;
    }

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vbuf64_atomic_winner_test_device"),
        required_features: needed,
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

fn run_atomic_max_two(device: &wgpu::Device, queue: &wgpu::Queue, v1: u64, v2: u64) -> u64 {
    use wgpu::util::DeviceExt;

    // 32x1 R64Uint = 32 * 8 = 256 bytes/row, exactly meeting wgpu's
    // 256-byte bytes-per-row alignment for copy_texture_to_buffer.
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vbuf64_test_texture"),
        size: wgpu::Extent3d {
            width: 32,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R64Uint,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Zero the texture explicitly. We can't rely on `clear_texture`
    // (Features::CLEAR_TEXTURE not always available); instead we upload
    // a zero buffer once at the start, mirroring what the production
    // clear shader does each frame.
    let zero_bytes = vec![0u8; 32 * 8];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &zero_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct Inputs {
        v1: u64,
        v2: u64,
    }

    let inputs = Inputs { v1, v2 };
    let inputs_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vbuf64_test_inputs"),
        contents: bytemuck::bytes_of(&inputs),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("vbuf64_test_shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("vbuf64_test_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::Atomic,
                    format: wgpu::TextureFormat::R64Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(16),
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("vbuf64_test_pipeline_layout"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("vbuf64_test_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("cs_atomic_max_two"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vbuf64_test_bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: inputs_buf.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("vbuf64_test_encoder"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("vbuf64_test_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vbuf64_test_staging"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        wgpu::Extent3d {
            width: 32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    rx.recv().expect("channel closed").expect("map failed");

    let data = slice.get_mapped_range();
    // The first 8 bytes are pixel (0,0). Following 31 pixels = 248 bytes
    // are the still-zero remainder of the row.
    let pixel: [u8; 8] = data[..8].try_into().unwrap();
    drop(data);
    staging.unmap();

    u64::from_le_bytes(pixel)
}

#[test]
fn closer_reversed_z_fragment_wins_atomicmax_on_gpu() {
    let Some((device, queue)) = try_acquire_device_vbuf64() else {
        eprintln!(
            "vbuf64 features unavailable on this adapter — skipping atomic R64 GPU test"
        );
        return;
    };

    // Reversed-Z: 0.95 is closer than 0.20. Larger packed u64 wins
    // atomicMax, so the closer fragment is the deterministic winner.
    let near = pack_visibility(0.95, 10, 0);
    let far = pack_visibility(0.20, 10, 0);
    let winner = run_atomic_max_two(&device, &queue, far, near);
    assert_eq!(winner, near, "closer fragment must win atomicMax");

    // Symmetric: insertion order doesn't matter, atomicMax is
    // commutative.
    let winner_swap = run_atomic_max_two(&device, &queue, near, far);
    assert_eq!(
        winner_swap, near,
        "atomicMax is commutative; closer still wins regardless of order"
    );
}

#[test]
fn equal_depth_higher_cluster_id_wins_atomicmax_on_gpu() {
    let Some((device, queue)) = try_acquire_device_vbuf64() else {
        eprintln!(
            "vbuf64 features unavailable on this adapter — skipping atomic R64 GPU test"
        );
        return;
    };

    // Coplanar fragments at identical depth: the larger packed_ids slot
    // wins the tie under atomicMax. This is what Bevy does and what the
    // deferred shader's unpack assumes.
    let lhs = pack_visibility(0.5, 100, 0);
    let rhs = pack_visibility(0.5, 99, 0);
    let winner = run_atomic_max_two(&device, &queue, rhs, lhs);
    assert_eq!(winner, lhs, "tie-break: larger cluster_id wins atomicMax");

    let (depth, slot, tri) = unpack_visibility(winner);
    assert_eq!(slot, 100);
    assert_eq!(tri, 0);
    assert_eq!(depth.to_bits(), 0.5_f32.to_bits());
}

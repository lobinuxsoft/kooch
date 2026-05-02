//! Shared GPU integration test harness.
//!
//! Cargo treats `tests/common/mod.rs` as a non-test sibling: each
//! integration test that pulls it in via `mod common;` recompiles the
//! helpers without spawning extra runners. Owns:
//!
//! - [`try_acquire_device`] — best-effort wgpu device, returns `None`
//!   when no adapter is available so CI without a GPU skips cleanly.
//! - [`build_cube_mesh`] — small canonical mesh used by both the cull
//!   integration and the render integration tests.
//! - [`read_buffer_to_vec`] — generic readback helper.
//!
//! Run integration tests with `--test-threads=1` (Mesa radv parallel
//! workers SIGSEGV inside Vulkan when several adapters init
//! concurrently — documented in `project_phase1_progress.md`).

#![allow(dead_code)] // each test binary touches a different subset

use bytemuck::Pod;
use ome_render::mesh::{Mesh, MeshVertex};

/// Acquires a wgpu device with no special features, suitable for any
/// meshlet test. Returns `None` if the adapter request fails — the
/// caller is expected to early-return so headless CI still passes.
pub fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
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

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ome_render_test_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// Builds a UV-sphere with `lat_segments` × `lon_segments` quads. Used
/// by the end-to-end bench to give meshopt enough geometry to produce
/// 4+ meshlets — the cube produces ~2 meshlets, which is too small to
/// stress-test the cull pipeline.
pub fn build_sphere_mesh(lat_segments: u32, lon_segments: u32) -> Mesh {
    use std::f32::consts::PI;

    let lat = lat_segments.max(2);
    let lon = lon_segments.max(3);

    let mut vertices = Vec::with_capacity(((lat + 1) * (lon + 1)) as usize);
    for j in 0..=lat {
        let theta = j as f32 / lat as f32 * PI; // [0, π]
        let sin_t = theta.sin();
        let cos_t = theta.cos();
        for i in 0..=lon {
            let phi = i as f32 / lon as f32 * 2.0 * PI; // [0, 2π]
            let x = sin_t * phi.cos();
            let y = cos_t;
            let z = sin_t * phi.sin();
            vertices.push(MeshVertex {
                position: [x, y, z],
                normal: [x, y, z], // unit sphere → position == normal
                uv: [i as f32 / lon as f32, j as f32 / lat as f32],
            });
        }
    }

    let mut indices = Vec::with_capacity((lat * lon * 6) as usize);
    let stride = lon + 1;
    for j in 0..lat {
        for i in 0..lon {
            let a = j * stride + i;
            let b = a + 1;
            let c = (j + 1) * stride + i;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    Mesh::from_arrays(vertices, indices)
}

/// Builds a 12-triangle cube mesh centred at the origin, edge length 1.
/// `meshopt::build_meshlets` clusters this into a handful of meshlets
/// — small enough to keep tests fast, large enough that frustum culling
/// has something to flip.
pub fn build_cube_mesh() -> Mesh {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let face_normals = [
        [0.0, 0.0, -1.0], // -Z
        [0.0, 0.0, 1.0],  // +Z
        [0.0, -1.0, 0.0], // -Y
        [0.0, 1.0, 0.0],  // +Y
        [-1.0, 0.0, 0.0], // -X
        [1.0, 0.0, 0.0],  // +X
    ];

    // Six faces, four unique vertices each — duplicated so per-face
    // normals are not blended. 24 vertices total.
    let face_indices: [[usize; 4]; 6] = [
        [0, 1, 2, 3], // -Z
        [4, 5, 6, 7], // +Z (note: needs CCW reversal below)
        [0, 1, 5, 4], // -Y
        [3, 2, 6, 7], // +Y
        [0, 3, 7, 4], // -X
        [1, 2, 6, 5], // +X
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, corners) in face_indices.iter().enumerate() {
        let normal = face_normals[face_idx];
        let base = vertices.len() as u32;
        for &c in corners {
            vertices.push(MeshVertex {
                position: positions[c],
                normal,
                uv: [0.0, 0.0],
            });
        }
        // Quad → 2 triangles. Order chosen for outward-facing CCW.
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    Mesh::from_arrays(vertices, indices)
}

/// Reads `buffer` back into a `Vec<T>`. `T` must be Pod and `buffer`'s
/// usage must include `COPY_SRC`. Blocks the device until the readback
/// is fully resolved.
pub fn read_buffer_to_vec<T: Pod + Default + Clone>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    count: u64,
) -> Vec<T> {
    let elem_size = std::mem::size_of::<T>() as u64;
    let byte_count = elem_size * count;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback_staging"),
        size: byte_count,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, byte_count);
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range();
    let mut out = vec![T::default(); count as usize];
    out.as_mut_slice().clone_from_slice(bytemuck::cast_slice::<u8, T>(&bytes));
    out
}

/// Reads a single 4-byte u32 at `offset` from `buffer`.
pub fn read_u32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
) -> u32 {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("u32_readback_staging"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("u32_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, 4);
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range();
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes);
    u32::from_le_bytes(buf)
}

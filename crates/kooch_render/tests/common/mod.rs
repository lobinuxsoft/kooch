//! Shared GPU integration test harness.
//!
//! Cargo treats `tests/common/mod.rs` as a non-test sibling: each
//! integration test that pulls it in via `mod common;` recompiles the
//! helpers without spawning extra runners. Owns:
//!
//! - [`try_acquire_device`] — best-effort wgpu device, returns `None`
//!   when no adapter is available so CI without a GPU skips cleanly.
//!   Adapter is acquired once per test binary via `OnceLock` to dodge
//!   the Mesa radv `request_adapter` race documented in issue #334.
//! - [`build_cube_mesh`] — small canonical mesh used by both the cull
//!   integration and the render integration tests.
//! - [`read_buffer_to_vec`] — generic readback helper.

#![allow(dead_code)] // each test binary touches a different subset

/// The lit scene the two shading-path binaries share (#824, #825).
pub mod lit_scene;

use std::sync::OnceLock;

use bytemuck::Pod;
use kooch_render::mesh::{Mesh, MeshVertex};

static SHARED_DEVICE: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

/// Acquires a wgpu device with no special features, suitable for any
/// meshlet test. Returns `None` if the adapter request fails — the
/// caller is expected to early-return so headless CI still passes.
pub fn try_acquire_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    SHARED_DEVICE
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });

            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .ok()?;

            // Hi-Z SPD pyramid build (#486) needs 12 storage texture
            // slots in one bind group; default wgpu limit is 4. Raise
            // it here mirroring the production GpuContext setup.
            let mut limits = wgpu::Limits::default();
            limits.max_storage_textures_per_shader_stage =
                16.min(adapter.limits().max_storage_textures_per_shader_stage);
            // #454.4 cull pipeline binds 5 groups (cull, pool, scene,
            // group_err, debug); the production GpuContext clamps to
            // 6 (TARGET_MAX_BIND_GROUPS in kooch_core). Mirror it here
            // so the cull shader compiles against the test device.
            limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
            // #454.6 cull pipeline binds 9 storage buffers
            // (params + visible IDs + count + 2 pool descriptors +
            // instances + group_max_err + reject_reasons +
            // stage_counters). wgpu's default 8 fails shader-module
            // creation. Mirror TARGET_MAX_STORAGE_BUFFERS_PER_STAGE.
            limits.max_storage_buffers_per_shader_stage =
                16.min(adapter.limits().max_storage_buffers_per_shader_stage);
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("kooch_render_test_device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            }))
            .ok()
        })
        .clone()
}

/// Acquires a wgpu device carrying the 64-bit texture-atomic bundle, so
/// the render stage builds its `Vbuf64Stage` and a frame takes the
/// **R64 path** — the one the OneXFly runs.
///
/// 🔴 [`try_acquire_device`] requests `Features::empty()`, which means a
/// test written against it silently exercises the R32 / Hi-Z fallback
/// instead. That is not hypothetical: #795's first GPU-scope test passed
/// with the R64 scopes deleted for exactly this reason, and #824's
/// parity tests passed with the tile-light loop deliberately broken
/// until this helper existed. Any test whose subject is the R64 path has
/// to come through here.
///
/// Returns `None` when the adapter does not advertise the bundle (Metal
/// has no `atomic_uint64`, older drivers lack it), so headless CI skips
/// cleanly. Its own device rather than the shared one: the features
/// differ, and a test that needs them must not be handed one without.
pub fn try_acquire_device_r64() -> Option<(wgpu::Device, wgpu::Queue)> {
    // 🔴 A fourth copy of the engine's feature list, and the reason it
    // is spelled out again is that the rig asks the adapter directly.
    // `SHADER_F16` and `FLOAT32_FILTERABLE` are here because the shaders
    // under test use them, not because the vbuf64 path needs them —
    // leaving either out makes every GPU test skip with "no adapter",
    // which reads as missing hardware rather than a stale list.
    let required = wgpu::Features::TEXTURE_ATOMIC
        | wgpu::Features::TEXTURE_INT64_ATOMIC
        | wgpu::Features::SHADER_INT64
        | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX
        | wgpu::Features::SHADER_F16
        | wgpu::Features::FLOAT32_FILTERABLE;

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
    if !adapter.features().contains(required) {
        return None;
    }

    // The same limits `try_acquire_device` raises, for the same
    // pipelines — the cull, vbuf and Hi-Z shaders do not compile
    // against wgpu's defaults.
    let mut limits = wgpu::Limits::default();
    limits.max_storage_textures_per_shader_stage =
        16.min(adapter.limits().max_storage_textures_per_shader_stage);
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
    limits.max_storage_buffers_per_shader_stage =
        16.min(adapter.limits().max_storage_buffers_per_shader_stage);

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kooch_render_test_device_r64"),
        required_features: required,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}

/// Acquires a wgpu device with `Features::TIMESTAMP_QUERY` +
/// `TIMESTAMP_QUERY_INSIDE_ENCODERS` so the mesh-frame bench (#335)
/// can drive `MeshletGpuTimers` for per-pass timing. Returns `None`
/// when either no adapter is available OR the adapter doesn't expose
/// timestamp queries (Mesa llvmpipe, MoltenVK on some macOS releases,
/// software fallbacks under WSL2). The bench is `#[ignore]`d by
/// default so skipping cleanly here keeps CI green on those backends.
///
/// Does NOT share state with [`try_acquire_device`]; the bench is a
/// dedicated test binary so the extra adapter request per run is
/// fine. Mirrors that helper's limits (storage textures / bind
/// groups / storage buffers) so the cull / vbuf / Hi-Z pipelines
/// compile against this device.
pub fn try_acquire_device_with_timer() -> Option<(wgpu::Device, wgpu::Queue, wgpu::Adapter)> {
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

    let features = adapter.features();
    let needed = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if !features.contains(needed) {
        return None;
    }

    let mut limits = wgpu::Limits::default();
    limits.max_storage_textures_per_shader_stage =
        16.min(adapter.limits().max_storage_textures_per_shader_stage);
    limits.max_bind_groups = 6.min(adapter.limits().max_bind_groups);
    limits.max_storage_buffers_per_shader_stage =
        16.min(adapter.limits().max_storage_buffers_per_shader_stage);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kooch_render_bench_device"),
        required_features: needed,
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue, adapter))
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
    //
    // 🔴 Every corner order below is counter-clockwise **seen from
    // outside**, which is what `front_face: Ccw` plus back-face culling
    // needs. Three of these used to wind the other way (-Z, +Y and -X):
    // the cull tests that were the only callers count meshlets and never
    // noticed, and the first test to look at the pixels saw a cube with
    // three missing faces and a floor whose top surface did not exist.
    let face_indices: [[usize; 4]; 6] = [
        [3, 2, 1, 0], // -Z
        [4, 5, 6, 7], // +Z
        [0, 1, 5, 4], // -Y
        [7, 6, 2, 3], // +Y
        [4, 7, 3, 0], // -X
        [1, 2, 6, 5], // +X
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, corners) in face_indices.iter().enumerate() {
        let normal = face_normals[face_idx];
        let base = vertices.len() as u32;
        // 🔴 A uv per corner, and it used to be [0, 0] on all 24.
        //
        // Nothing rendered differently for it — every material in these
        // tests sampled the 1x1 white fallback, where any coordinate
        // reads the same texel. What it broke was the ability to MEASURE
        // texture sampling at all: a mesh whose uvs are constant has uv
        // derivatives of exactly zero, so mip selection reports level 0
        // from every distance and a test looking at it concludes the
        // selection is broken. That is a wrong conclusion this file
        // handed out once already.
        for (corner, &c) in corners.iter().enumerate() {
            vertices.push(MeshVertex {
                position: positions[c],
                normal,
                uv: match corner {
                    0 => [0.0, 0.0],
                    1 => [1.0, 0.0],
                    2 => [1.0, 1.0],
                    _ => [0.0, 1.0],
                },
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
    out.as_mut_slice()
        .clone_from_slice(bytemuck::cast_slice::<u8, T>(&bytes));
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

/// Reads a 2-D `Rgba8Unorm` texture back into a tightly packed buffer.
pub fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let (w, h) = (size.width, size.height);
    let unpadded = w * 4;
    let padded = unpadded.div_ceil(256) * 256;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rgba8_readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rgba8_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((unpadded * h) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    staging.unmap();
    out
}

/// sRGB electrical value → linear.
///
/// Comparisons belong in linear because that is where the shading
/// happened. In 8-bit sRGB the transfer function plus ACES compress a
/// genuine 2× difference in irradiance down to about 1.1× in the byte,
/// which makes a working BRDF look like a broken one.
pub fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Mean linear luminance over a `(2·half + 1)²` box centred on
/// `(cx, cy)`, so one stray pixel on an edge cannot decide a test.
///
/// The box is clamped to the image, which matters at a silhouette: a
/// sample that runs off the edge would otherwise index another row.
pub fn luminance_at(pixels: &[u8], width: u32, cx: u32, cy: u32, half: u32) -> f32 {
    let height = pixels.len() as u32 / (width * 4);
    let mut total = 0.0;
    let mut count = 0.0;
    for y in cy.saturating_sub(half)..=(cy + half).min(height - 1) {
        for x in cx.saturating_sub(half)..=(cx + half).min(width - 1) {
            let idx = ((y * width + x) * 4) as usize;
            total += 0.2126 * srgb_to_linear(pixels[idx])
                + 0.7152 * srgb_to_linear(pixels[idx + 1])
                + 0.0722 * srgb_to_linear(pixels[idx + 2]);
            count += 1.0;
        }
    }
    total / count
}

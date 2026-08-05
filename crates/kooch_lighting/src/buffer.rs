//! GPU residency for Inti: the per-frame constants UBO and the light
//! storage buffer, plus the bind group both shading paths bind.

use bytemuck::cast_slice;
use glam::Vec3;
use kooch_core::resource::Resources;
use wgpu::util::DeviceExt;

use crate::extract::extract_lights;
use crate::frame::{AmbientLight, Exposure, IntiFrame};
use crate::gpu_light::GpuLight;

/// Lights a fresh buffer holds before it has to grow. Sixteen covers an
/// authored room; growth is geometric from there.
const INITIAL_CAPACITY: u32 = 16;

/// Owns the two bindings `inti_pbr.wgsl` declares and keeps them sized
/// to the scene.
///
/// # One buffer, several views
///
/// The light set does not depend on where the camera is, so it is
/// shared across every view in a frame. `camera_position` does depend
/// on it and rides in the frame UBO anyway — which is safe only
/// because each view records **and submits** its own encoder:
/// `write(A) → submit(A) → write(B) → submit(B)` is ordered on the
/// queue, so B's camera cannot reach A's pass. The existing
/// `MaterialTwoPass::camera_buffer` is shared on exactly this
/// reasoning. If a future path ever records two views into one encoder,
/// this buffer needs a dynamic offset per view, the same way the
/// per-material screen UBO already does.
pub struct GpuLights {
    frame_buffer: wgpu::Buffer,
    light_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    capacity: u32,
    light_count: u32,
}

impl GpuLights {
    /// The layout both shading pipelines declare. An associated
    /// function because a pipeline layout has to be built before any
    /// `GpuLights` exists — the same shape `MaterialPool` uses.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inti_lights_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Visible to both stages because the R64 path shades
                    // in a fragment shader and the R32 fallback in a
                    // compute one, off the same layout.
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<IntiFrame>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let layout = Self::bind_group_layout(device);
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("inti_frame_ubo"),
            contents: bytemuck::bytes_of(&IntiFrame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let light_buffer = create_light_buffer(device, INITIAL_CAPACITY);
        let bind_group = create_bind_group(device, &layout, &frame_buffer, &light_buffer);
        Self {
            frame_buffer,
            light_buffer,
            bind_group,
            layout,
            capacity: INITIAL_CAPACITY,
            light_count: 0,
        }
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Lights uploaded by the last [`Self::update`]. Reported by the
    /// editor stats overlay so "I placed a light and nothing changed"
    /// has an answer that is not a guess.
    pub fn light_count(&self) -> u32 {
        self.light_count
    }

    /// Walks the world, uploads the lights, and writes the per-frame
    /// constants.
    ///
    /// Call **before** creating the frame's encoder: growing the
    /// storage buffer replaces it, and a replaced buffer must not be
    /// one a recorded pass already references.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        camera_position: Vec3,
    ) {
        let lights = extract_lights(resources);
        self.light_count = lights.len() as u32;
        self.ensure_capacity(device, self.light_count);
        if !lights.is_empty() {
            queue.write_buffer(&self.light_buffer, 0, cast_slice(&lights));
        }

        let ambient = resources.get::<AmbientLight>().copied().unwrap_or_default();
        let exposure = resources.get::<Exposure>().copied().unwrap_or_default();
        queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&IntiFrame::new(
                &ambient,
                &exposure,
                camera_position,
                self.light_count,
            )),
        );
    }

    /// Grows geometrically to fit `needed`. Never shrinks: a scene that
    /// oscillates around a capacity boundary would otherwise reallocate
    /// every frame.
    fn ensure_capacity(&mut self, device: &wgpu::Device, needed: u32) {
        if needed <= self.capacity {
            return;
        }
        let capacity = needed.next_power_of_two().max(INITIAL_CAPACITY);
        self.light_buffer = create_light_buffer(device, capacity);
        self.bind_group =
            create_bind_group(device, &self.layout, &self.frame_buffer, &self.light_buffer);
        self.capacity = capacity;
        tracing::debug!(
            target: "kooch_lighting::buffer",
            capacity,
            "grew the Inti light buffer",
        );
    }
}

/// Capacity is floored at one element even for an unlit scene: wgpu
/// rejects a zero-sized storage binding, and a second pipeline for
/// "no lights" would be a whole code path that only runs in the case
/// nobody looks at.
fn create_light_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("inti_lights_storage"),
        size: (capacity.max(1) as u64) * std::mem::size_of::<GpuLight>() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    frame: &wgpu::Buffer,
    lights: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("inti_lights_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: lights.as_entire_binding(),
            },
        ],
    })
}

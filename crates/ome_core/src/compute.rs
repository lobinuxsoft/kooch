//! Compute shader utilities.
//!
//! [`VectorAddCompute`] demonstrates a minimal compute pipeline that adds
//! two `f32` arrays element-wise on the GPU.
//!
//! # Example
//! ```ignore
//! let compute = VectorAddCompute::new(&device);
//! compute.dispatch(&device, &queue, &buf_a, &buf_b, &buf_out, 1024);
//! ```

use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, Buffer, CommandEncoderDescriptor, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, Device, PipelineLayoutDescriptor, Queue,
    ShaderModuleDescriptor, ShaderStages,
};

const VECTOR_ADD_SHADER: &str = r"
@group(0) @binding(0) var<storage, read> input_a: array<f32>;
@group(0) @binding(1) var<storage, read> input_b: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if idx < arrayLength(&output) {
        output[idx] = input_a[idx] + input_b[idx];
    }
}
";

const WORKGROUP_SIZE: u32 = 64;

/// Compute pipeline that adds two `f32` arrays element-wise.
///
/// Accepts raw `&Device` / `&Queue` references so it can be used both inside
/// the engine (via [`GpuContext`](crate::gpu::GpuContext)) and in standalone
/// headless examples.
pub struct VectorAddCompute {
    pipeline: ComputePipeline,
    bind_group_layout: BindGroupLayout,
}

impl VectorAddCompute {
    /// Creates the shader module, bind group layout, and compute pipeline.
    pub fn new(device: &Device) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("vector_add_shader"),
            source: wgpu::ShaderSource::Wgsl(VECTOR_ADD_SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("vector_add_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("vector_add_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("vector_add_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    /// Dispatches the compute shader over `count` elements.
    ///
    /// `input_a`, `input_b` and `output` must be GPU buffers of at least
    /// `count * 4` bytes each. The caller is responsible for creating a
    /// staging buffer and copying the result back to CPU if needed.
    pub fn dispatch(
        &self,
        device: &Device,
        queue: &Queue,
        input_a: &Buffer,
        input_b: &Buffer,
        output: &Buffer,
        count: u32,
    ) {
        let bind_group = self.create_bind_group(device, input_a, input_b, output);

        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("vector_add_encoder"),
        });

        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("vector_add_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(count.div_ceil(WORKGROUP_SIZE), 1, 1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    fn create_bind_group(
        &self,
        device: &Device,
        input_a: &Buffer,
        input_b: &Buffer,
        output: &Buffer,
    ) -> BindGroup {
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("vector_add_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: input_a.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: input_b.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use wgpu::{DeviceDescriptor, Instance, InstanceDescriptor, RequestAdapterOptions};

    use super::*;

    #[test]
    #[ignore] // Requires GPU hardware.
    fn vector_add_pipeline_creation() {
        let instance = Instance::new(InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
            ..Default::default()
        });

        let adapter =
            pollster::block_on(instance.request_adapter(&RequestAdapterOptions::default()))
                .expect("no GPU adapter found");

        let (device, _queue) = pollster::block_on(adapter.request_device(
            &DeviceDescriptor {
                label: Some("test_device"),
                ..Default::default()
            },
            None,
        ))
        .expect("failed to create device");

        // Should not panic.
        let _compute = VectorAddCompute::new(&device);
    }
}

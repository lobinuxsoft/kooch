//! Building the six passes: layouts, pipelines, samplers and targets.

use super::*;

impl Fsr3 {
    pub(in crate::meshlet::vbuf64_stage) fn new(
        device: &wgpu::Device,
        render: (u32, u32),
        output: (u32, u32),
    ) -> Self {
        let module = |label: &str, pass: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source(pass).into()),
            })
        };

        let inputs_module = module("fsr3_prepare_inputs", PREPARE_INPUTS_SOURCE);
        let reduce_module = module("fsr3_reduce", REDUCE_SOURCE);
        let reactivity_module = module("fsr3_prepare_reactivity", REACTIVITY_SOURCE);
        let instability_module = module("fsr3_luma_instability", INSTABILITY_SOURCE);
        let accumulate_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fsr3_accumulate"),
            source: wgpu::ShaderSource::Wgsl(source_f16(ACCUMULATE_SOURCE).into()),
        });

        let prepare_inputs_bgl = layout(
            device,
            "fsr3_prepare_inputs_bgl",
            &[
                uniform(0),
                sampled(1, wgpu::TextureSampleType::Depth),
                filterable(2),
                filterable(3),
                storage(
                    4,
                    wgpu::TextureFormat::R32Uint,
                    wgpu::StorageTextureAccess::Atomic,
                ),
                write_hdr(5),
                storage(
                    6,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::WriteOnly,
                ),
                write_r32(7),
            ],
        );
        let reduce_bgl = layout(
            device,
            "fsr3_reduce_bgl",
            &[uniform(0), filterable(1), write_r32(2), write_r32(3)],
        );
        let reactivity_bgl = layout(
            device,
            "fsr3_reactivity_bgl",
            &[
                uniform(0),
                filterable(1),
                unfilterable(2),
                sampled(3, wgpu::TextureSampleType::Uint),
                filterable(4),
                filterable(5),
                sampler_entry(6),
                write_hdr(7),
                write_r32(8),
                write_r32(9),
            ],
        );
        let instability_bgl = layout(
            device,
            "fsr3_instability_bgl",
            &[
                uniform(0),
                filterable(1),
                filterable(2),
                filterable(3),
                filterable(4),
                sampler_entry(5),
                write_hdr(6),
                write_r32(7),
            ],
        );
        let accumulate_bgl = layout(
            device,
            "fsr3_accumulate_bgl",
            &[
                uniform(0),
                filterable(1),
                filterable(2),
                filterable(3),
                filterable(4),
                filterable(5),
                filterable(6),
                sampler_entry(7),
                storage(
                    8,
                    wgpu::TextureFormat::R32Float,
                    wgpu::StorageTextureAccess::ReadWrite,
                ),
                write_hdr(9),
                write_hdr(10),
            ],
        );

        Self {
            prepare_inputs: pipeline(
                device,
                "fsr3_prepare_inputs",
                &inputs_module,
                "prepare_inputs",
                &prepare_inputs_bgl,
            ),
            clear_reconstructed: pipeline(
                device,
                "fsr3_clear_reconstructed_depth",
                &inputs_module,
                "clear_reconstructed_depth",
                &prepare_inputs_bgl,
            ),
            prepare_inputs_bgl,
            farthest_mip1: pipeline(
                device,
                "fsr3_farthest_depth_mip1",
                &reduce_module,
                "farthest_depth_mip1",
                &reduce_bgl,
            ),
            clear_new_locks: pipeline(
                device,
                "fsr3_clear_new_locks",
                &reduce_module,
                "clear_new_locks",
                &reduce_bgl,
            ),
            reduce_bgl,
            reactivity: pipeline(
                device,
                "fsr3_prepare_reactivity",
                &reactivity_module,
                "prepare_reactivity",
                &reactivity_bgl,
            ),
            reactivity_bgl,
            instability: pipeline(
                device,
                "fsr3_luma_instability",
                &instability_module,
                "luma_instability",
                &instability_bgl,
            ),
            instability_bgl,
            accumulate: pipeline(
                device,
                "fsr3_accumulate",
                &accumulate_module,
                "accumulate",
                &accumulate_bgl,
            ),
            accumulate_bgl,
            ubo: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fsr3_ubo"),
                size: std::mem::size_of::<Fsr3Ubo>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            linear: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("fsr3_linear"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            }),
            targets: Targets::new(device, render, output),
            render_size: render,
            output_size: output,
            state: std::sync::Mutex::new(History {
                index: 0,
                reset: true,
                frame_index: 0,
                prev_jitter: glam::Vec2::ZERO,
                prev_exposure: 1.0,
                logged_stage: None,
            }),
        }
    }
}

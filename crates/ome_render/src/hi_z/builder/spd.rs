use std::num::NonZeroU64;

use super::super::{HI_Z_FORMAT, SPD_PYRAMID_SLOT_COUNT};
use super::types::{HiZ, SpdConstants};

impl HiZ {
    /// Records the SPD pyramid build into `encoder`. `depth_view`
    /// must reference a Depth32Float texture matching the dimensions
    /// passed to [`Self::new`].
    ///
    /// The `arena` is required: the caller MUST keep the per-call
    /// SPD bind group alive past `queue.submit`. wgpu does not
    /// internally Arc-clone bind groups on `set_bind_group`, and
    /// Mesa radv invalidates them if the local goes out of scope
    /// before the GPU reaches the dispatch. See PR #479 for the
    /// arena pattern this matches.
    pub fn build_from_depth(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        depth_view: &wgpu::TextureView,
        arena: &mut Vec<wgpu::BindGroup>,
    ) {
        let bg = build_spd_bind_group(
            device,
            &self.spd_bgl,
            depth_view,
            &self.mip_views,
            self.mip_count,
            &self.spd_dummy_view,
            &self.spd_sampler,
            &self.spd_constants_buffer,
        );

        // First dispatch: one workgroup per 64×64 virtual-source
        // tile. Each workgroup writes 32×32 pixels to mip_1
        // (= pyramid mip 0). `virtual_w/h` = source rounded up to
        // next pow2 (with +1 to force strict round); pyramid mip 0
        // size = virtual / 2, so workgroups = virtual / 64.
        let wg_x = (self.virtual_w / 64).max(1);
        let wg_y = (self.virtual_h / 64).max(1);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_spd_first_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spd_first_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // Second dispatch: one workgroup over mip 6 → mips 7..=12.
        // Skipped when the pyramid doesn't reach mip 7.
        if self.mip_count > 6 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_spd_second_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.spd_second_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        arena.push(bg);
    }
}

pub(super) fn build_spd_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let mut entries = Vec::with_capacity(15);
    // mip_0: source depth attachment.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    });
    // mip_1..=mip_5: write-only storage.
    for binding in 1..=5 {
        entries.push(storage_slot(binding, wgpu::StorageTextureAccess::WriteOnly));
    }
    // mip_6: read_write (cross-workgroup sync between the two
    // dispatches). Requires `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`,
    // which the engine already enables in `gpu.rs::required_engine_features`.
    entries.push(storage_slot(6, wgpu::StorageTextureAccess::ReadWrite));
    // mip_7..=mip_12: write-only storage.
    for binding in 7..=12 {
        entries.push(storage_slot(binding, wgpu::StorageTextureAccess::WriteOnly));
    }
    // sampler for textureGather on mip_0.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 13,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
    });
    // Constants UBO.
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 14,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(std::mem::size_of::<SpdConstants>() as u64),
        },
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_spd_bgl"),
        entries: &entries,
    })
}

fn storage_slot(binding: u32, access: wgpu::StorageTextureAccess) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access,
            format: HI_Z_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_spd_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    depth_view: &wgpu::TextureView,
    mip_views: &[wgpu::TextureView],
    mip_count: u32,
    dummy_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    constants: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let mut entries = Vec::with_capacity(15);
    entries.push(wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(depth_view),
    });
    for slot in 0..SPD_PYRAMID_SLOT_COUNT as u32 {
        let view = if slot < mip_count {
            &mip_views[slot as usize]
        } else {
            dummy_view
        };
        entries.push(wgpu::BindGroupEntry {
            binding: slot + 1,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: 13,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 14,
        resource: constants.as_entire_binding(),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hi_z_spd_bg"),
        layout,
        entries: &entries,
    })
}

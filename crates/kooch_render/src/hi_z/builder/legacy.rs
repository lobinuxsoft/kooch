use super::super::{HI_Z_FORMAT, WORKGROUP_SIZE, mip_size};
use super::types::HiZ;

impl HiZ {
    /// Test-only path: per-mip downsample from an R32Float source.
    /// Production callers should use [`Self::build_from_depth`].
    pub fn build_from_r32(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        r32_view: &wgpu::TextureView,
    ) {
        let copy_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hi_z_copy_r32_bg"),
            layout: &self.copy_r32_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(r32_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.mip_views[0]),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_copy_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.copy_r32_pipeline);
            pass.set_bind_group(2, &copy_bg, &[]);
            pass.dispatch_workgroups(
                self.width.div_ceil(WORKGROUP_SIZE),
                self.height.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
        for mip in 1..self.mip_count {
            let (dst_w, dst_h) = mip_size(self.width, self.height, mip);
            let bg = &self.reduce_bgs[(mip - 1) as usize];
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("hi_z_reduce_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce_pipeline);
            pass.set_bind_group(1, bg, &[]);
            pass.dispatch_workgroups(
                dst_w.div_ceil(WORKGROUP_SIZE),
                dst_h.div_ceil(WORKGROUP_SIZE),
                1,
            );
        }
    }
}

pub(super) fn bgl_copy_r32(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_copy_r32_bgl"),
        entries: &[float_src_entry(0), storage_dst_entry(1)],
    })
}

pub(super) fn bgl_reduce(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("hi_z_reduce_bgl"),
        entries: &[float_src_entry(0), storage_dst_entry(1)],
    })
}

fn float_src_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_dst_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: HI_Z_FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

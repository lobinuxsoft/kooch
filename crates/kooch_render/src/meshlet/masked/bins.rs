//! A view's masked bins: its cull's survivors grouped per masked material, and the draws over them.

use super::{MASKED_BINS, MaskedRaster};
use crate::material::MaterialPipeline;
use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;

const ARGS_BYTES: u64 = 16;

/// Per view, beside its cull: what [`MaskedRaster::bin`] writes and [`MaskedRaster::draw`] reads.
pub struct MaskedBins {
    counts: wgpu::Buffer,
    bases: wgpu::Buffer,
    args: wgpu::Buffer,
    dispatch: wgpu::Buffer,
    /// Slots into the cull's visible list, as long as it.
    slots: wgpu::Buffer,
}

impl MaskedBins {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |label, size, extra| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | extra,
                mapped_at_creation: false,
            })
        };
        let bins = MASKED_BINS as u64;
        let none = wgpu::BufferUsages::empty();
        let indirect = wgpu::BufferUsages::INDIRECT;
        Self {
            counts: storage("masked_counts", bins * 4, none),
            bases: storage("masked_bases", bins * 4, none),
            args: storage("masked_args", bins * ARGS_BYTES, indirect),
            dispatch: storage("masked_dispatch", 12, indirect),
            slots: storage("masked_slots", 4, none),
        }
    }

    fn fit(&mut self, device: &wgpu::Device, visible: &wgpu::Buffer) {
        if self.slots.size() < visible.size() {
            self.slots = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("masked_slots"),
                size: visible.size(),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
        }
    }
}

impl MaskedRaster {
    /// Groups `cull`'s survivors per masked material. After the cull, before the raster.
    pub fn bin(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        bins: &mut MaskedBins,
        cull: &MeshletCull,
        scene: &MeshletScene,
    ) {
        if !self.any() {
            return;
        }
        bins.fit(device, cull.visible_meshlets_buffer());
        let resources = [
            cull.visible_meshlets_buffer(),
            cull.indirect_args_buffer(),
            scene.instance_buffer(),
            &self.table,
            &bins.counts,
            &bins.bases,
            &bins.args,
            &bins.slots,
        ];
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masked_bins_bg"),
            layout: &self.layouts.bins,
            entries: &entries,
        });
        let dispatch = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masked_dispatch_bg"),
            layout: &self.layouts.dispatch,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: bins.dispatch.as_entire_binding(),
            }],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("masked_bins_prepare"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &dispatch, &[]);
            pass.set_pipeline(&self.layouts.prepare);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("masked_bins"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &group, &[]);
        pass.set_pipeline(&self.layouts.count);
        pass.dispatch_workgroups_indirect(&bins.dispatch, 0);
        pass.set_pipeline(&self.layouts.offsets);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.layouts.scatter);
        pass.dispatch_workgroups_indirect(&bins.dispatch, 0);
    }

    /// One indirect draw per bin, into the pass the opaque raster just drew in. `vbuf64` is the
    /// R64 target, and `None` on the R32 path, which writes its colour attachment.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::RenderPass<'_>,
        bins: &MaskedBins,
        materials: &MaterialPipeline,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        vbuf64: Option<&wgpu::TextureView>,
    ) {
        if !self.any() {
            return;
        }
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let mut frame = vec![
            entry(0, self.camera.as_entire_binding()),
            entry(
                1,
                wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.screen,
                    offset: 0,
                    size: std::num::NonZeroU64::new(std::mem::size_of::<super::Screen>() as u64),
                }),
            ),
            entry(2, self.inti.as_entire_binding()),
            entry(3, bins.slots.as_entire_binding()),
            entry(4, bins.bases.as_entire_binding()),
        ];
        if let Some(view) = vbuf64 {
            frame.push(entry(5, wgpu::BindingResource::TextureView(view)));
        }
        let frame_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masked_frame_bg"),
            layout: &self.layouts.frame,
            entries: &frame,
        });
        let materials_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masked_materials_bg"),
            layout: &self.layouts.materials,
            entries: &[
                entry(0, materials.pool().buffer().as_entire_binding()),
                entry(1, materials.pool().values().as_entire_binding()),
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("masked_scene_bg"),
            layout: &self.layouts.scene,
            entries: &[
                entry(0, cull.visible_meshlets_buffer().as_entire_binding()),
                entry(1, scene.instance_buffer().as_entire_binding()),
            ],
        });
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, &materials_bg, &[]);
        pass.set_bind_group(3, &scene_bg, &[]);
        let textures = materials.texture_pool();
        for (bin, (slot, pipeline)) in self.bins.iter().enumerate() {
            let offset = (bin as u64 * self.screen_stride) as u32;
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &frame_bg, &[offset]);
            let group = textures.material_bind_group(device, &materials.slot_texture_refs(*slot));
            pass.set_bind_group(4, &group, &[]);
            pass.draw_indirect(&bins.args, bin as u64 * ARGS_BYTES);
        }
    }
}

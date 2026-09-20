//! GPU acceptance for camera stacking (#1221): a base camera and an overlay with a different
//! culling mask compose into one image — the overlay's layers over the base's, and the base
//! wherever the overlay drew nothing.
//!
//! Run with:
//!   cargo test -p kooch_render --test camera_stack

mod common;

use common::lit_scene::{Rig, SIZE, rig};
use glam::{Mat4, Vec3};
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_render::VIEWPORT_DEPTH_FORMAT;
use kooch_render::meshlet::{MeshletBlit, ViewId};
use kooch_render::quality::{ShadingSettings, TemporalSettings, UpscaleTechnique};

/// A block of the rig's own material between the camera and the floor, on `layers`: big enough that
/// an image missing it is not a matter of a few pixels.
fn block(r: &mut Rig, layers: u32) {
    let mut commands = Commands::new();
    commands
        .spawn(&mut r.resources)
        .insert(MeshRenderer {
            mesh: Some(r.mesh),
            material: Some(r.material),
            visible: true,
            layers,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.0, 1.5, 4.0))
                * Mat4::from_scale(Vec3::splat(2.0)),
        });
    commands.apply(&mut r.resources);
}

/// One composed image, colour and depth: the depth is where a wrong composite shows up, because a
/// blit that writes its empty pixels leaves the right colour and no geometry under it.
struct Frame {
    color: Vec<u8>,
    depth: Vec<f32>,
}

/// What the stack composes into, and what the base alone composes into: the same rig, the same
/// frame, read twice so the difference is the overlay and nothing else.
struct Composed {
    base: Frame,
    stacked: Frame,
}

/// Renders `base_mask` into the primary view and `overlay_mask` into a second one, then composes
/// base-then-overlay the way the frame does.
fn compose(base_mask: u32, overlay_mask: u32) -> Option<Composed> {
    composed(base_mask, overlay_mask, None)
}

/// The same, through `upscale` — which is how a project ships, and where the coverage the composite
/// reads is easiest to lose.
fn composed(
    base_mask: u32,
    overlay_mask: u32,
    upscale: Option<TemporalSettings>,
) -> Option<Composed> {
    let mut r: Rig = rig(2, true)?;
    if let Some(temporal) = upscale {
        r.resources.insert(temporal);
        // What a project publishes, and what a view created mid-session reads: the per-view compute
        // flag is set from this every frame, and a rig that never published it leaves a late view on
        // the fragment path.
        r.resources.insert(ShadingSettings {
            compute: true,
            ..Default::default()
        });
        // The first frame records the scale; the resize is what turns it into a smaller buffer.
        common::lit_scene::render(&mut r, true);
        r.stage.resize(&r.device, (SIZE, SIZE));
    }
    // The block is on layer 1; the floor, wall and lights are on layer 0.
    block(&mut r, 0b10);
    r.camera.culling_mask = base_mask;

    let overlay_view = r.stack_view();
    let mut overlay_camera = r.camera;
    overlay_camera.culling_mask = overlay_mask;

    // Settled, as the shadow tests do: the pages fill over a few frames.
    for _ in 0..4 {
        r.stage
            .render_with_assets_primary(&r.device, &r.queue, &r.resources, &r.camera, 1.0);
        r.stage.render_with_assets(
            overlay_view,
            &r.device,
            &r.queue,
            &r.resources,
            &overlay_camera,
            1.0,
        );
    }

    Some(Composed {
        base: r.composite(&[]),
        stacked: r.composite(&[overlay_view]),
    })
}

impl Rig {
    /// A second view at the rig's size, as an overlay camera gets one.
    fn stack_view(&mut self) -> ViewId {
        self.stage.create_view(&self.device, (SIZE, SIZE))
    }

    /// The primary view blitted onto a fresh target, then `overlays` over it, read back.
    fn composite(&self, overlays: &[ViewId]) -> Frame {
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("camera_stack_target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("camera_stack_depth"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: VIEWPORT_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        let blit = MeshletBlit::new(
            &self.device,
            wgpu::TextureFormat::Rgba8Unorm,
            VIEWPORT_DEPTH_FORMAT,
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("camera_stack_encoder"),
            });
        // The clear the base camera owns: an overlay brings none of this.
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("camera_stack_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        blit.blit(
            &self.device,
            &mut encoder,
            self.stage.color_view(),
            self.stage.depth_sample_view(),
            &view,
            &depth_view,
        );
        for overlay in overlays {
            blit.blit(
                &self.device,
                &mut encoder,
                self.stage
                    .view_color_view(*overlay)
                    .expect("the overlay view is live"),
                self.stage
                    .view_depth_sample(*overlay)
                    .expect("the overlay view is live"),
                &view,
                &depth_view,
            );
        }
        self.queue.submit(Some(encoder.finish()));
        Frame {
            color: common::read_rgba8(&self.device, &self.queue, &target),
            depth: self.read_depth(&depth),
        }
    }

    /// A `Depth32Float` attachment, tightly packed.
    fn read_depth(&self, texture: &wgpu::Texture) -> Vec<f32> {
        let padded = (SIZE * 4).div_ceil(256) * 256;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera_stack_depth_readback"),
            size: u64::from(padded * SIZE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("camera_stack_depth_copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).expect("the send lands")
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
        rx.recv()
            .expect("the map answers")
            .expect("the map succeeds");
        let bytes = slice.get_mapped_range().to_vec();
        let mut depth = Vec::with_capacity((SIZE * SIZE) as usize);
        for row in 0..SIZE {
            let at = (row * padded) as usize;
            for x in 0..SIZE as usize {
                let word = &bytes[at + x * 4..at + x * 4 + 4];
                depth.push(f32::from_le_bytes(word.try_into().expect("four bytes")));
            }
        }
        depth
    }
}

/// Pixels where the two images differ.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(a, b)| a[..3] != b[..3])
        .count()
}

/// 🔴 The acceptance: what the base excludes and the overlay keeps is in the composed image, and it
/// was not in the base's own.
#[test]
fn an_overlay_adds_its_layer() {
    let Some(composed) = compose(0b01, 0b10) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let added = differing(&composed.base.color, &composed.stacked.color);
    let pixels = composed.base.color.len() / 4;
    assert!(
        added * 50 > pixels,
        "the overlay changed {added} pixels of {pixels}: it drew nothing over the base",
    );
}

/// 🔴 What makes it an overlay rather than a second frame: everywhere it drew nothing, the base is
/// untouched. A composite that wrote its empty pixels would blank the scene under it.
#[test]
fn an_overlay_keeps_the_base() {
    let Some(composed) = compose(0b01, 0b10) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let lit = composed
        .base
        .color
        .chunks_exact(4)
        .zip(composed.stacked.color.chunks_exact(4))
        .filter(|(base, _)| base[0] as u32 + base[1] as u32 + base[2] as u32 > 30);
    let (kept, shown) = lit.fold((0usize, 0usize), |(kept, shown), (base, stacked)| {
        (kept + usize::from(base[..3] == stacked[..3]), shown + 1)
    });
    assert!(shown > 0, "the base drew nothing to keep");
    // The block the overlay adds covers part of the scene on purpose; most of the base survives it.
    assert!(
        kept * 2 > shown,
        "only {kept} of the base's {shown} lit pixels survived the overlay",
    );
}

/// An overlay that keeps no layer composes nothing: the stack is the base, pixel for pixel.
#[test]
fn an_empty_overlay_changes_nothing() {
    let Some(composed) = compose(0b01, 0) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    assert_eq!(
        differing(&composed.base.color, &composed.stacked.color),
        0,
        "an overlay drawing nothing still changed the image",
    );
    // 🔴 And the geometry under it survives: the colour blend hides a composite that writes its
    // empty pixels, the depth does not — and what comes after the stack tests against it.
    let moved = composed
        .base
        .depth
        .iter()
        .zip(&composed.stacked.depth)
        .filter(|(base, stacked)| base != stacked)
        .count();
    assert_eq!(
        moved, 0,
        "the overlay wiped the depth of {moved} pixels where it drew nothing",
    );
}

/// 🔴 The configuration a project actually ships: SGSR2 at half scale. The upscaler used to write
/// alpha 1 over the whole image, and an overlay composed from that is an opaque black plate with
/// its own objects on it — the base gone. Coverage has to survive every pass that rewrites colour.
#[test]
fn an_upscaled_overlay_keeps_the_base() {
    let upscale = TemporalSettings {
        technique: UpscaleTechnique::Sgsr2,
        render_scale: 50,
        sharpening: 50,
    };
    let Some(composed) = composed(0b01, 0b10, Some(upscale)) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let lit = composed
        .base
        .color
        .chunks_exact(4)
        .zip(composed.stacked.color.chunks_exact(4))
        .filter(|(base, _)| base[0] as u32 + base[1] as u32 + base[2] as u32 > 30);
    let (kept, shown) = lit.fold((0usize, 0usize), |(kept, shown), (base, stacked)| {
        (kept + usize::from(base[..3] == stacked[..3]), shown + 1)
    });
    assert!(shown > 0, "the base drew nothing to keep");
    // 🔴 Both halves, or the test is vacuous: an overlay that composed nothing at all would keep
    // every pixel of the base and prove nothing about coverage.
    let added = differing(&composed.base.color, &composed.stacked.color);
    let pixels = composed.base.color.len() / 4;
    assert!(
        added * 50 > pixels,
        "the upscaled overlay changed {added} pixels of {pixels}: it composed nothing",
    );
    assert!(
        kept * 2 > shown,
        "only {kept} of the base's {shown} lit pixels survived an upscaled overlay",
    );
}

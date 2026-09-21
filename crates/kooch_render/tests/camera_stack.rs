//! GPU acceptance for camera stacking (#1221): a base camera and an overlay with a different
//! culling mask compose into one image — the overlay's layers over the base's, and the base
//! wherever the overlay drew nothing.
//!
//! Run with:
//!   cargo test -p kooch_render --test camera_stack

mod common;

use common::composite::Frame;
use common::lit_scene::{Rig, SIZE, rig};
use glam::{Mat4, Vec3};
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_render::meshlet::ViewId;
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

    /// The primary view composed over black, then `overlays` over it.
    fn composite(&self, overlays: &[ViewId]) -> Frame {
        common::composite::composite(self, overlays, wgpu::Color::BLACK)
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

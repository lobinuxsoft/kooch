//! GPU acceptance for light and shadow layers (#1220): a light lights the layers it names, and
//! casts only what its shadow mask names — two masks, because a lamp that lights the character
//! without lighting the floor is not the same wish as one the character casts no shadow into.
//!
//! Run with:
//!   cargo test -p kooch_render --test light_layers

mod common;

use common::lit_scene::{Rig, rig};
use glam::{Mat4, Vec3};
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::point_light::PointLight;
use kooch_render::shadow::ShadowSettings;

/// A blue lamp over the scene, lighting `layers` and shadowing `shadow_layers`.
fn lamp(r: &mut Rig, layers: u32, shadow_layers: u32, cast_shadows: bool) {
    let mut commands = Commands::new();
    commands
        .spawn(&mut r.resources)
        .insert(PointLight {
            // Cold, so its contribution reads against the warm scene rather than adding to it.
            color: Vec3::new(0.05, 0.2, 1.0),
            intensity: 60_000.0,
            range: 14.0,
            radius: 0.1,
            cast_shadows,
            layers,
            shadow_layers,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_translation(Vec3::new(0.0, 2.0, 1.0)),
        });
    commands.apply(&mut r.resources);
}

/// A pane of the rig's own material under the lamp, on `layers`.
fn pane(r: &mut Rig, layers: u32) {
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
            matrix: Mat4::from_translation(Vec3::new(0.0, 1.2, 1.0))
                * Mat4::from_scale(Vec3::new(2.5, 0.02, 2.5)),
        });
    commands.apply(&mut r.resources);
}

/// How much blue the image holds, which is how much of that lamp reached it.
fn blue(r: &mut Rig) -> u64 {
    // Settled, as the shadow tests do: the pages fill over a few frames.
    let mut pixels = Vec::new();
    for _ in 0..4 {
        pixels = common::lit_scene::render(r, true);
    }
    pixels.chunks_exact(4).map(|p| p[2] as u64).sum()
}

/// 🔴 The first half: a light reaches the layers it names and nothing else, tested per pixel in the
/// shade rather than by keeping a list per light.
#[test]
fn a_light_reaches_only_its_layers() {
    let Some(mut every) = rig(0, true) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    lamp(&mut every, u32::MAX, u32::MAX, false);
    let lit = blue(&mut every);

    let mut none = rig(0, true).unwrap();
    // A layer the scene is not in: everything it stands on is in Default, bit 0.
    lamp(&mut none, 0b10, u32::MAX, false);
    let dark = blue(&mut none);

    // Only the ambient is left on the other layer: the lamp is the scene's one light.
    assert!(
        lit > dark + dark / 4,
        "the lamp lit {lit} against {dark} on a layer nothing is in",
    );
}

/// Shadows on, on the classic maps or in the virtual pages, which are two different machines and
/// two different culls.
fn shadowed_rig(pages: bool) -> Option<Rig> {
    let mut r = rig(0, true)?;
    if let Some(settings) = r.resources.get_mut::<ShadowSettings>() {
        settings.enabled = true;
        settings.virtual_pages = pages;
    }
    Some(r)
}

/// 🔴 The second half: what casts into a light is its own mask, and it is rejected in that light's
/// cull — a pane the lamp does not shadow takes none of its light off the floor. Both shadow
/// machines: the cube maps and the virtual pages cull their casters in different code.
#[test]
fn a_shadow_keeps_only_its_layers() {
    for pages in [false, true] {
        let Some(mut open) = shadowed_rig(pages) else {
            eprintln!("no R64-capable adapter; skipping");
            return;
        };
        lamp(&mut open, u32::MAX, u32::MAX, true);
        let nothing_between = blue(&mut open);

        let casting = |shadow_layers: u32| {
            let mut r = shadowed_rig(pages).unwrap();
            lamp(&mut r, u32::MAX, shadow_layers, true);
            // The pane is on layer 1; the floor and wall are on Default.
            pane(&mut r, 0b10);
            blue(&mut r)
        };
        let shadowed = casting(u32::MAX);
        let ignored = casting(0b01);

        assert!(
            shadowed < nothing_between,
            "pages {pages}: the pane cast nothing at all: {shadowed} against {nothing_between}",
        );
        let taken = |after: u64| nothing_between as i64 - after as i64;
        assert!(
            taken(ignored) * 4 < taken(shadowed),
            "pages {pages}: the pane still shadowed a layer the lamp does not: it took {} of the \
             {} it took while shadowing",
            taken(ignored),
            taken(shadowed),
        );
    }
}

/// 🔴 The author unticks a layer and the shadow has to go with it. Both shadow machines CACHE what
/// they drew — a cube while its key holds, a page while its generation does — and nothing else
/// about the frame moved, so a mask changed in place was the one case that kept the old shadow on
/// screen. Two renders of ONE scene, which is what the Inspector does.
#[test]
fn a_mask_changed_in_place_redraws() {
    for pages in [false, true] {
        let Some(mut r) = shadowed_rig(pages) else {
            eprintln!("no R64-capable adapter; skipping");
            return;
        };
        lamp(&mut r, u32::MAX, u32::MAX, true);
        pane(&mut r, 0b10);
        let shadowed = blue(&mut r);

        // The same scene, the same rig, one box unticked.
        {
            let query = kooch_ecs::query::Query::<&mut PointLight>::new(&r.resources);
            query.for_each(|light| light.shadow_layers = 0b01);
        }
        let ignored = blue(&mut r);

        assert!(
            ignored > shadowed,
            "pages {pages}: the shadow survived the mask being unticked — {ignored} against the \
             {shadowed} it was while shadowing",
        );
    }
}

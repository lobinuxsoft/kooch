//! GPU acceptance for the forward pass (#452): a transparent surface blends over the opaque scene
//! instead of replacing it or vanishing.
//!
//! Run with:
//!   cargo test -p kooch_render --test transparent_forward

mod common;

use common::lit_scene::{SIZE, rig};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_render::material::{Material, MaterialPipeline, Shader};

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * SIZE + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// Light of its own in one colour, half covering: what is behind it has to show through.
fn glass(colour: &str, alpha: f32) -> String {
    format!(
        "// kind: transparent
fn surface(input: SurfaceInput) -> SurfaceOutput {{
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.emissive = vec3<f32>({colour});
    out.alpha = {alpha:?};
    return out;
}}
"
    )
}

/// Adds a pane of `colour` glass where `matrix` puts the rig's cube.
fn pane(r: &mut common::lit_scene::Rig, colour: &str, alpha: f32, matrix: Mat4) {
    let shader = Guid::new_v4();
    let material = Guid::new_v4();
    let materials = r.resources.get_mut::<MaterialPipeline>().unwrap();
    materials.add_shader(shader, &Shader::parse(&glass(colour, alpha)).unwrap());
    let mut look = Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 1.0, 0.0);
    look.shader = Some(shader);
    materials.register(&r.queue, material, &look);
    let mut commands = Commands::new();
    commands
        .spawn(&mut r.resources)
        .insert(MeshRenderer {
            mesh: Some(r.mesh),
            material: Some(material),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform { matrix });
    commands.apply(&mut r.resources);
}

fn centre(pixels: &[u8]) -> [u8; 3] {
    pixel(pixels, SIZE / 2, SIZE / 2)
}

/// The scene's centre pixel, with or without a pane of red glass in front of the wall.
fn render(glass: bool) -> Option<[u8; 3]> {
    let mut r = rig(2, true)?;
    if glass {
        pane(
            &mut r,
            "1.0, 0.0, 0.0",
            0.5,
            Mat4::from_translation(Vec3::new(0.0, 1.2, 4.0))
                * Mat4::from_scale(Vec3::new(3.0, 3.0, 0.1)),
        );
    }
    Some(centre(&common::lit_scene::render(&mut r, true)))
}

#[test]
fn glass_blends_over_the_scene() {
    let Some(behind) = render(false) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let through = render(true).unwrap();
    assert!(
        through[0] > behind[0].saturating_add(20),
        "no red over {behind:?}: {through:?} — the forward pass drew nothing",
    );
    assert!(
        through[1] > 20 && through[2] > 20,
        "{through:?} over {behind:?} is solid red — the glass replaced the scene",
    );
}

/// 🔴 Two panes crossing in an X: left of the crossing the red one is nearer, right of it the blue
/// one. An order per object gets one side wrong whichever it picks; per pixel, each side shows its
/// own front pane over the other.
#[test]
fn crossing_panes_sort_per_pixel() {
    let Some(mut r) = rig(2, true) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let at = Mat4::from_translation(Vec3::new(0.0, 1.2, 4.0));
    let size = Mat4::from_scale(Vec3::new(3.0, 3.0, 0.02));
    pane(
        &mut r,
        "1.0, 0.0, 0.0",
        0.5,
        at * Mat4::from_rotation_y(0.6) * size,
    );
    pane(
        &mut r,
        "0.0, 0.0, 1.0",
        0.5,
        at * Mat4::from_rotation_y(-0.6) * size,
    );
    let pixels = common::lit_scene::render(&mut r, true);
    let left = pixel(&pixels, SIZE / 2 - 25, SIZE / 2);
    let right = pixel(&pixels, SIZE / 2 + 25, SIZE / 2);
    assert!(
        left[0] > left[2],
        "left of the crossing the red pane is in front: {left:?}"
    );
    assert!(
        right[2] > right[0],
        "right of the crossing the blue pane is in front: {right:?}"
    );
}

/// Past four layers the tail carries the rest. A pane is a thin box, two faces deep: two panes fill
/// the four layers and a third lands in the tail, where it still hides some of the wall. Dropped
/// instead of blended, it would change nothing.
#[test]
fn the_tail_keeps_deep_layers() {
    let wall_through = |panes: u32| {
        let mut r = rig(2, true)?;
        for i in 0..panes {
            pane(
                &mut r,
                "0.0, 0.0, 0.0",
                0.15,
                Mat4::from_translation(Vec3::new(0.0, 1.2, 4.0 - i as f32 * 0.3))
                    * Mat4::from_scale(Vec3::new(3.0, 3.0, 0.02)),
            );
        }
        // Dark panes add nothing: what light there is came through them from the wall.
        Some(centre(&common::lit_scene::render(&mut r, true))[1])
    };
    let Some(two) = wall_through(2) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let three = wall_through(3).unwrap();
    assert!(three < two, "a third pane hides nothing: {three} vs {two}");
}

/// A transparent object casts a solid shadow (#452; by alpha is #1224): a nearly invisible pane
/// under the blue light takes more blue off the image than its own 5% cover could. Measured: 4.9%
/// with the shadow, 0.09% without.
#[test]
fn glass_casts_a_shadow() {
    let blue = |with_pane: bool| {
        let mut r = common::lit_scene::rig_with_caster(2)?;
        if with_pane {
            pane(
                &mut r,
                "0.0, 0.0, 0.0",
                0.05,
                Mat4::from_translation(Vec3::new(0.0, 1.2, 1.0))
                    * Mat4::from_scale(Vec3::new(2.5, 0.02, 2.5)),
            );
        }
        // Settled, as the shadow tests do: the pages fill over a few frames.
        let mut pixels = Vec::new();
        for _ in 0..4 {
            pixels = common::lit_scene::render(&mut r, true);
        }
        Some(pixels.chunks_exact(4).map(|p| p[2] as u64).sum::<u64>())
    };
    let Some(open) = blue(false) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let shaded = blue(true).unwrap();
    assert!(
        shaded * 100 < open * 97,
        "the pane took {open} → {shaded} of the blue light: no shadow",
    );
}

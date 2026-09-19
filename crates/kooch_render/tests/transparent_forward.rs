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
    pane_casting(r, colour, alpha, matrix, true);
}

fn pane_casting(
    r: &mut common::lit_scene::Rig,
    colour: &str,
    alpha: f32,
    matrix: Mat4,
    cast_shadows: bool,
) {
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
            cast_shadows,
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

/// The blue light the image receives with a black pane under the caster: `None` no pane,
/// `Some((alpha, cast))` one of that coverage whose renderer casts shadows or not. `pages` puts
/// the lights' shadows in the virtual pages instead of the classic maps.
fn blue_under_pane(pane: Option<(f32, bool)>, pages: bool) -> Option<u64> {
    let mut r = common::lit_scene::rig_with_caster(2)?;
    if pages {
        r.resources
            .get_mut::<kooch_render::shadow::ShadowSettings>()
            .unwrap()
            .virtual_pages = true;
    }
    if let Some((alpha, cast)) = pane {
        pane_casting(
            &mut r,
            "0.0, 0.0, 0.0",
            alpha,
            Mat4::from_translation(Vec3::new(0.0, 1.2, 1.0))
                * Mat4::from_scale(Vec3::new(2.5, 0.02, 2.5)),
            cast,
        );
    }
    // Settled, as the shadow tests do: the pages fill over a few frames.
    let mut pixels = Vec::new();
    for _ in 0..4 {
        pixels = common::lit_scene::render(&mut r, true);
    }
    Some(pixels.chunks_exact(4).map(|p| p[2] as u64).sum::<u64>())
}

/// How much blue a pane takes off the image, against no pane.
fn taken(pane: (f32, bool), pages: bool) -> Option<i64> {
    let open = blue_under_pane(None, pages)? as i64;
    Some(open - blue_under_pane(Some(pane), pages)? as i64)
}

/// 🔴 The shadow follows the coverage (#1224): a 30% pane takes a fraction of what a solid one
/// does, where a solid shadow would take the same. Both kinds of shadow map.
#[test]
fn the_shadow_follows_alpha() {
    for pages in [false, true] {
        let Some(solid) = taken((1.0, true), pages) else {
            eprintln!("no R64-capable adapter; skipping");
            return;
        };
        let thin = taken((0.3, true), pages).unwrap();
        assert!(
            solid > 0,
            "pages {pages}: a solid pane casts nothing ({solid})"
        );
        assert!(
            thin * 10 < solid * 6 && thin * 10 > solid,
            "pages {pages}: a 30% pane took {thin} of the light a solid one took {solid}",
        );
    }
}

/// 🔴 `cast_shadows` off takes the renderer out of every shadow view. It was read by nothing, so
/// unticking it changed nothing, on opaque and transparent renderers alike. What is left is the
/// pane seen directly.
#[test]
fn cast_shadows_off_casts_none() {
    let Some(casting) = taken((1.0, true), false) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let quiet = taken((1.0, false), false).unwrap();
    assert!(
        quiet * 2 < casting,
        "a pane that casts no shadow took {quiet} of the light, a casting one {casting}",
    );
}

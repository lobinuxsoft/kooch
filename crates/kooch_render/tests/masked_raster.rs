//! GPU acceptance for masked materials (#452): a surface that assigns `alpha_clip` is cut where its
//! alpha falls below it, and what is cut shows what lies behind.
//!
//! Run with:
//!   cargo test -p kooch_render --test masked_raster

mod common;

use common::lit_scene::{Rig, SIZE, rig, rig_r32};
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

/// Red light of its own, its alpha running across the uv's x.
fn cut(clip: f32) -> String {
    format!(
        "fn surface(input: SurfaceInput) -> SurfaceOutput {{
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.emissive = vec3<f32>(8.0, 0.0, 0.0);
    out.alpha = input.uv.x;
    out.alpha_clip = {clip:?};
    return out;
}}
"
    )
}

/// The two sides of the scene's centre, with a pane clipped at `clip` in front of the wall, or none,
/// on the R64 visibility buffer or the R32 one.
fn sides(clip: Option<f32>, r32: bool) -> Option<([u8; 3], [u8; 3])> {
    sides_of(clip.map(cut), r32)
}

/// The two sides of the scene's centre with a pane of `source` in front of the wall, or none.
fn sides_of(source: Option<String>, r32: bool) -> Option<([u8; 3], [u8; 3])> {
    let mut r: Rig = match r32 {
        true => rig_r32(2, true)?,
        false => rig(2, true)?,
    };
    if let Some(source) = source {
        let shader = Guid::new_v4();
        let material = Guid::new_v4();
        let materials = r.resources.get_mut::<MaterialPipeline>().unwrap();
        let parsed = Shader::parse(&source).unwrap();
        assert!(parsed.masked());
        materials.add_shader(shader, &parsed);
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
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(0.0, 1.2, 4.0))
                    * Mat4::from_scale(Vec3::new(3.0, 3.0, 0.1)),
            });
        commands.apply(&mut r.resources);
    }
    let pixels = match r32 {
        true => common::lit_scene::render_any(&mut r),
        false => common::lit_scene::render(&mut r, true),
    };
    Some((
        pixel(&pixels, SIZE / 2 - 25, SIZE / 2),
        pixel(&pixels, SIZE / 2 + 25, SIZE / 2),
    ))
}

/// 🔴 Cut at a half, one side of the pane is gone and shows the scene behind; the other stays. By
/// the scene rather than by colour: the R32 shading paints the material, not the surface.
fn cuts_half_away(r32: bool) {
    let Some(scene) = sides(None, r32) else {
        eprintln!("no capable adapter; skipping");
        return;
    };
    let cut = sides(Some(0.5), r32).unwrap();
    let shown = [cut.0 == scene.0, cut.1 == scene.1];
    assert!(
        shown[0] != shown[1],
        "exactly one side should show the scene: {cut:?} over {scene:?}"
    );
}

#[test]
fn a_clip_cuts_half_away() {
    cuts_half_away(false);
}

/// The same cut on the R32 buffer, which draws its bins twice: once per Hi-Z pass.
#[test]
fn r32_clip_cuts_half_away() {
    cuts_half_away(true);
}

/// A clip of 0 keeps every fragment: masked, and solid.
#[test]
fn a_zero_clip_keeps_everything() {
    let Some(scene) = sides(None, false) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let kept = sides(Some(0.0), false).unwrap();
    assert!(
        kept.0 != scene.0 && kept.1 != scene.1,
        "{kept:?} over {scene:?}"
    );
}

/// 🔴 A transparent surface that clips: below the clip nothing is left, not even a faint layer; above
/// it, it still blends.
#[test]
fn a_transparent_clip_cuts() {
    let Some(scene) = sides(None, false) else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    let glass = format!("// kind: transparent\n{}", cut(0.5));
    let (left, right) = sides_of(Some(glass), false).unwrap();
    let shown = [left == scene.0, right == scene.1];
    assert!(
        shown[0] != shown[1],
        "exactly one side should show the scene: {left:?} {right:?} over {scene:?}"
    );
}

/// The blue light the image receives with a black masked pane clipped at `clip` under the caster,
/// or no pane. `pages` puts the lights' shadows in the virtual pages instead of the classic maps.
fn blue_under(clip: Option<f32>, pages: bool) -> Option<u64> {
    let mut r = common::lit_scene::rig_with_caster(2)?;
    if pages {
        r.resources
            .get_mut::<kooch_render::shadow::ShadowSettings>()
            .unwrap()
            .virtual_pages = true;
    }
    if let Some(clip) = clip {
        let shader = Guid::new_v4();
        let material = Guid::new_v4();
        let materials = r.resources.get_mut::<MaterialPipeline>().unwrap();
        let source = cut(clip).replace("vec3<f32>(8.0, 0.0, 0.0)", "vec3<f32>(0.0)");
        materials.add_shader(shader, &Shader::parse(&source).unwrap());
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
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(0.0, 1.2, 1.0))
                    * Mat4::from_scale(Vec3::new(2.5, 0.02, 2.5)),
            });
        commands.apply(&mut r.resources);
    }
    // Settled, as the shadow tests do: the pages fill over a few frames.
    let mut pixels = Vec::new();
    for _ in 0..4 {
        pixels = common::lit_scene::render(&mut r, true);
    }
    Some(pixels.chunks_exact(4).map(|p| p[2] as u64).sum::<u64>())
}

/// 🔴 A pane cut in half casts half a shadow, on both kinds of shadow map: solid, it would take as
/// much light as the whole pane.
#[test]
fn the_shadow_is_cut_too() {
    for pages in [false, true] {
        let Some(open) = blue_under(None, pages) else {
            eprintln!("no R64-capable adapter; skipping");
            return;
        };
        let taken = |clip| open as i64 - blue_under(Some(clip), pages).unwrap() as i64;
        let whole = taken(0.0);
        let half = taken(0.5);
        assert!(whole > 0, "pages {pages}: the whole pane casts nothing");
        assert!(
            half * 4 > whole && half * 4 < whole * 3,
            "pages {pages}: half a pane took {half} of the light the whole one took {whole}",
        );
    }
}

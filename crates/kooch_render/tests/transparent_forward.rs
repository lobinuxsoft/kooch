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

/// Red light of its own, half covering: what is behind it has to show through.
const GLASS: &str = "// kind: transparent
fn surface(input: SurfaceInput) -> SurfaceOutput {
    var out: SurfaceOutput;
    out.normal = normalize(input.world_normal);
    out.roughness = 1.0;
    out.emissive = vec3<f32>(1.0, 0.0, 0.0);
    out.alpha = 0.5;
    return out;
}
";

fn centre(pixels: &[u8]) -> [u8; 3] {
    let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2]]
}

/// The scene's centre pixel, with or without a pane of red glass in front of the wall.
fn render(glass: bool) -> Option<[u8; 3]> {
    let mut r = rig(2, true)?;
    if glass {
        let shader = Guid::new_v4();
        let material = Guid::new_v4();
        let materials = r.resources.get_mut::<MaterialPipeline>().unwrap();
        materials.add_shader(shader, &Shader::parse(GLASS).unwrap());
        let mut pane = Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 1.0, 0.0);
        pane.shader = Some(shader);
        materials.register(&r.queue, material, &pane);
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

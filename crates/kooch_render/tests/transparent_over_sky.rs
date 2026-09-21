//! GPU acceptance: a transparent surface with nothing opaque behind it keeps its colour when the
//! frame composes the stage over the sky.
//!
//! Run with:
//!   cargo test -p kooch_render --test transparent_over_sky

mod common;

use common::lit_scene::{SIZE, rig};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_ecs::commands::Commands;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline, Shader};

/// Red glass no light touches, so its colour is known without a lighting model in the assertion.
const RED_GLASS: &str = "// kind: transparent_unlit
fn unlit(input: SurfaceInput) -> UnlitOutput {
    var out: UnlitOutput;
    out.color = vec3<f32>(1.0, 0.0, 0.0);
    out.alpha = 0.5;
    return out;
}
";

/// The centre pixel of the stage's own colour, and of the stage composed over a black sky, with a
/// slab of red glass overhead and nothing behind it.
fn centre_pixels() -> Option<([u8; 4], [u8; 4])> {
    let mut r = rig(0, false)?;
    // Looking straight up: the whole view is sky, and the slab is the only thing in it.
    r.camera = ViewCamera::looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 10.0, 0.0));

    let shader = Guid::new_v4();
    let material = Guid::new_v4();
    {
        let materials = r.resources.get_mut::<MaterialPipeline>().unwrap();
        materials.add_shader(shader, &Shader::parse(RED_GLASS).unwrap());
        let mut look = Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 1.0, 0.0);
        look.shader = Some(shader);
        materials.register(&r.queue, material, &look);
    }
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
            matrix: Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0))
                * Mat4::from_scale(Vec3::new(8.0, 0.1, 8.0)),
        });
    commands.apply(&mut r.resources);

    let stage = common::lit_scene::render(&mut r, true);
    let composed = common::composite::composite(&r, &[], wgpu::Color::BLACK).color;
    let at = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    let pick =
        |pixels: &[u8]| -> [u8; 4] { [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]] };
    Some((pick(&stage), pick(&composed)))
}

/// 🔴 The stage's colour is **premultiplied** — the transparent composite blends that way, so glass
/// over nothing is its colour already scaled by its alpha. Composing it with `SrcAlpha` scaled it a
/// second time: glass at half alpha over the sky came out at a quarter, and a darker glass
/// disappeared against a dark sky altogether.
#[test]
fn glass_over_the_sky_keeps_its_colour() {
    let Some((stage, composed)) = centre_pixels() else {
        eprintln!("no R64-capable adapter; skipping");
        return;
    };
    // Not vacuous: the glass is there, and half-covering.
    assert!(stage[0] > 60, "no glass in the stage at all: {stage:?}");
    assert!(
        (40..=220).contains(&stage[3]),
        "the glass's coverage is not partial: {stage:?}"
    );
    // Over black, a premultiplied colour composes to itself.
    assert!(
        (composed[0] as i32 - stage[0] as i32).abs() <= 2,
        "the stage's red {} came out as {} over a black sky",
        stage[0],
        composed[0],
    );
}

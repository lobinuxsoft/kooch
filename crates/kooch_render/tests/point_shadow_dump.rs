//! Not a test — a **look**. The owner's scene, rendered to PNGs (#851).
//!
//! Thirteen assertions in `point_shadows.rs` pass while the owner sees
//! shadows that die, a square drawn outward, and a different picture in
//! each camera. When that happens the rig is wrong, not the eye. This
//! reproduces the reported scene by its numbers — `intensity 320000`,
//! `range 10.0`, `ambient 0.0`, compute shading — and writes pictures
//! instead of asserting, because every assertion so far has agreed with
//! the code and disagreed with the screen.
//!
//! Run with:
//!   cargo test -p kooch_render --test point_shadow_dump -- --ignored --nocapture

mod common;

use common::{build_cube_mesh, read_rgba8, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::AccessTracker;
use kooch_render::ViewCamera;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets};
use kooch_render::shadow::ShadowSettings;

// 🔴 1536, not 512. The cube view tiles six faces into a 3x2 grid, so
// at 512 each face gets 170x256 px and a ball 27 px across — small
// enough that its own edge reads as a crescent. Two sessions were spent
// interpreting that crescent.
const SIZE: u32 = 1536;

/// The lamp the inspector showed, overhead.
const OVERHEAD: Vec3 = Vec3::new(0.0, 3.477, 0.0);
/// The reported values, not the suite's 4 000 000 over 40 m.
const INTENSITY: f32 = 320_000.0;
const RANGE: f32 = 10.0;

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
}

/// `occluder` places a cube standing on the floor. Without one the floor
/// is empty, and then **any** darkening in the picture is a defect:
/// nothing in the scene can shadow anything.
fn build(lights: &[(Vec3, bool)], occluder: bool, compute: bool) -> Option<Rig> {
    build_with(lights, occluder, compute, false)
}

/// `contact` turns the screen-space march on, which is what the owner's
/// lamps now have. It is the one shadow in the engine that reads the
/// CAMERA's depth buffer, so an occluder that leaves the screen stops
/// occluding — and #845 marches only the brightest light per pixel, so
/// two lamps never both get one.
fn build_with(
    lights: &[(Vec3, bool)],
    occluder: bool,
    compute: bool,
    contact: bool,
) -> Option<Rig> {
    let (device, queue) = try_acquire_device()?;
    let meshlet_mesh = build_default_meshlets(&build_cube_mesh()).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    resources.insert(ShadowSettings {
        cascade_texels: 1024,
        max_distance: 60.0,
        enabled: true,
        ..Default::default()
    });
    resources.insert(kooch_lighting::AmbientLight {
        intensity: 0.0,
        ..Default::default()
    });
    // What `roll-a-ball` ships and what every capture on record used.
    resources.insert(kooch_render::quality::ShadingSettings {
        compute,
        ..Default::default()
    });

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material = Guid::new_v4();
    materials.register(
        &queue,
        material,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.9, 0.0),
    );
    resources.insert(materials);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (SIZE, SIZE),
            instance_capacity: 8,
            meshlet_capacity: 1024,
            ..Default::default()
        },
    );
    let mesh = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh, &meshlet_mesh);
    // A dense sphere for the caster, because the owner's is a ball and
    // a cube is six quads: a defect that drops SOME meshlets of an
    // object cannot show on geometry that has almost none.
    //
    // 🔴 And with a real LOD **chain**, not `build_default_meshlets` —
    // that one lives in `builder/single_lod.rs` and produces one level.
    // The LOD selector is the only thing in the cull that can drop
    // meshlets from the MIDDLE of an object, and against a single-level
    // mesh it cannot run at all. Five reproductions came out clean for
    // that reason and for no other.
    let ball_mesh = kooch_render::meshlet::build_meshlets_lod_chain(
        &common::build_sphere_mesh(32, 48),
        kooch_render::meshlet::DEFAULT_MAX_VERTICES,
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES,
        0.5,
        Default::default(),
    )
    .expect("build sphere");
    let ball = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, ball, &ball_mesh);

    let mut commands = Commands::new();
    let mut spawn = |matrix: Mat4, which: Guid| {
        commands
            .spawn(&mut resources)
            .insert(MeshRenderer {
                mesh: Some(which),
                material: Some(material),
                visible: true,
                ..Default::default()
            })
            .insert(GlobalTransform { matrix });
    };
    spawn(
        Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_scale(Vec3::new(20.0, 0.5, 20.0)),
        mesh,
    );
    if occluder {
        spawn(
            Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0)) * Mat4::from_scale(Vec3::splat(0.5)),
            ball,
        );
    }

    for (position, cast_shadows) in lights {
        commands
            .spawn(&mut resources)
            .insert(PointLight {
                active: true,
                color: Vec3::ONE,
                intensity: INTENSITY,
                range: RANGE,
                radius: 0.0,
                cast_shadows: *cast_shadows,
                contact_shadows: contact,
            })
            .insert(GlobalTransform {
                matrix: Mat4::from_translation(*position),
            });
    }
    commands.apply(&mut resources);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
    })
}

fn shoot(rig: &mut Rig, name: &str, eye: Vec3) {
    let camera = ViewCamera::looking_at(eye, Vec3::new(0.0, 0.5, 0.0));
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let pixels = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    let path = std::path::Path::new("/tmp/kooch_point_shadows").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(&path, &pixels, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
    eprintln!("wrote {}", path.display());
}

/// Moves the cube that stands ON the floor, leaving the floor alone.
fn move_occluder(resources: &Resources, to: Vec3) {
    kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(resources).for_each(
        |(_, transform)| {
            if transform.matrix.w_axis.y > 0.0 {
                transform.matrix = Mat4::from_translation(to);
            }
        },
    );
}

/// Observation 2 — "las point lights dibujan un cuadrado de sombra hacia
/// afuera".
///
/// An EMPTY floor under one lamp. There is nothing to cast, so the
/// picture must be a smooth pool of light. A square, a diamond or a
/// diagonal seam is the cube's own face boundary printed onto the world.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn an_empty_floor_must_be_smooth() {
    let top = Vec3::new(0.0, 14.0, 0.1);
    for compute in [false, true] {
        let Some(mut rig) = build(&[(OVERHEAD, true)], false, compute) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if compute { "compute" } else { "fragment" };
        // From high up, so the whole footprint of the -Y face is in
        // frame: the lamp is 3.477 m up and a 90 deg face covers
        // +/- 3.477 m of floor, which is where a seam would fall.
        shoot(&mut rig, &format!("seam_{tag}_top.png"), top);
        shoot(
            &mut rig,
            &format!("seam_{tag}_angle.png"),
            Vec3::new(7.0, 6.0, 7.0),
        );
    }

    // The control. Same lamp, same floor, `cast_shadows` OFF — no cube
    // is sampled at all. If the square survives this it is not the cube
    // map and every word above is wrong.
    let Some(mut off) = build(&[(OVERHEAD, false)], false, true) else {
        return;
    };
    shoot(&mut off, "seam_control_no_cube.png", top);

    // And the same lamp at half the height. A 90 deg face covers
    // +/- h of floor, so if the square is the face footprint its side
    // must halve with it. Nothing else in the scene scales that way.
    let Some(mut low) = build(&[(Vec3::new(0.0, 1.74, 0.0), true)], false, true) else {
        return;
    };
    shoot(&mut low, "seam_half_height.png", top);

    // The number the eye cannot give. Same empty floor twice, the only
    // difference being whether a cube is sampled at all, so every
    // non-zero pixel of the difference is the floor shadowing itself.
    let with = grab(&[(OVERHEAD, true)], top);
    let without = grab(&[(OVERHEAD, false)], top);
    let (worst, mean) = compare(&with, &without);
    eprintln!("empty floor self-shadowing: worst {worst:.4}, mean {mean:.5} (0 is correct)");
}

/// Renders the empty floor once and returns the pixels.
fn grab(lights: &[(Vec3, bool)], eye: Vec3) -> Vec<u8> {
    let mut rig = build(lights, false, true).expect("device");
    let camera = ViewCamera::looking_at(eye, Vec3::new(0.0, 0.5, 0.0));
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

/// Worst and mean per-pixel darkening of `a` against `b`, 0 to 1.
fn compare(a: &[u8], b: &[u8]) -> (f32, f32) {
    let mut worst = 0.0f32;
    let mut total = 0.0f64;
    let count = a.len() / 4;
    for i in 0..count {
        let d = (b[i * 4] as f32 - a[i * 4] as f32).max(0.0) / 255.0;
        worst = worst.max(d);
        total += d as f64;
    }
    (worst, (total / count as f64) as f32)
}

/// Observation 1 — "si muevo la luz se actualizan las sombras, si no la
/// muevo mueren".
///
/// The lamp never moves. The caster does, three times, through ONE
/// stage so the cube cache is live. Frame 1 is always drawn; what a
/// cache breaks only shows on frame 2 and after.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn a_still_lamp_over_a_moving_caster() {
    for compute in [false, true] {
        let Some(mut rig) = build(&[(OVERHEAD, true)], true, compute) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if compute { "compute" } else { "fragment" };
        let eye = Vec3::new(6.0, 5.0, 6.0);
        for (n, x) in [0.0f32, 1.2, 2.4, 3.6].iter().enumerate() {
            move_occluder(&rig.resources, Vec3::new(*x, 0.5, 0.0));
            shoot(&mut rig, &format!("move_{tag}_{n}.png"), eye);
        }
    }
}

/// Observation 3 — "en cada cámara se ve diferente".
///
/// Two cameras, one stage, one frame's worth of state, nothing moved
/// between them. The cube maps are rendered from the LIGHT, so the two
/// pictures must agree about where the shadow is. If they do not, the
/// cube's contents depend on who is looking — which is the one thing a
/// shadow map must never do.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_cameras_must_agree() {
    let Some(mut rig) = build(&[(OVERHEAD, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Warm the cache the way a running editor does.
    shoot(&mut rig, "agree_0_warm.png", Vec3::new(6.0, 5.0, 6.0));
    shoot(&mut rig, "agree_1_far.png", Vec3::new(14.0, 11.0, 14.0));
    shoot(&mut rig, "agree_2_near.png", Vec3::new(3.0, 2.2, 3.0));
    // Back to the first camera. Same eye as `agree_0_warm`, so the two
    // must be the same picture — anything else is the cube remembering
    // who looked at it last.
    shoot(&mut rig, "agree_3_back.png", Vec3::new(6.0, 5.0, 6.0));
}

/// The remaining report — "dependiendo de en qué posición esté la cámara
/// la point light genera sombras cortadas o no las genera".
///
/// A dense ball, the lamp at the position the inspector showed, and NINE
/// cameras around it through ONE stage, so the cube cache lives across
/// all of them exactly as it does in a running editor. The cube maps are
/// rendered from the LIGHT: the shadow must land on the same patch of
/// floor in every one of these, whole, with no straight edge cutting it.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_shadow_must_not_depend_on_the_camera() {
    // Straight off the inspector.
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    // Around the ball at a constant height and radius, so the only
    // thing that changes between frames is where the lens is.
    for i in 0..9 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let eye = Vec3::new(a.cos() * 6.0, 4.0, a.sin() * 6.0);
        shoot(&mut rig, &format!("orbit_{i}.png"), eye);
    }
}

/// The editor's arrangement, which no test has ever had: **two views on
/// one stage**.
///
/// `render_with_assets(view_id, ..)` runs the whole frame per view, and
/// `prepare_shadows` takes the CAMERA. So `select_point_casters` culls
/// lamps against whichever camera is rendering, while
/// `point_cube_cache` and `point_shadow_holders` belong to the stage and
/// are shared. Every previous picture in this file came from one view,
/// which is why every previous picture agreed with itself.
///
/// The Game camera here is deliberately pointed away from the lamp, the
/// way a gameplay camera is while the author looks at the lamp in the
/// View panel.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_views_on_one_stage() {
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));

    // The reference: the View camera alone, cold, drawn twice so the
    // second one is a cache hit with nothing else interleaved.
    let view_eye = Vec3::new(6.0, 4.0, 0.0);
    shoot(&mut rig, "views_0_alone.png", view_eye);
    shoot(&mut rig, "views_1_alone_again.png", view_eye);

    // Now alternate, the way the editor does every frame: Game looking
    // away from the lamp, then View from the same eye as above. If the
    // last picture differs from the first two, the shadow depends on
    // who else looked this frame.
    for round in 0..3 {
        let game = ViewCamera::looking_at(Vec3::new(30.0, 2.0, 30.0), Vec3::new(40.0, 0.0, 40.0));
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(
            &mut rig,
            &format!("views_2_after_game_{round}.png"),
            view_eye,
        );
    }
}

/// Which half of the two-view frame does it: the camera-frustum cull, or
/// the shared cube cache?
///
/// Same alternation as `two_views_on_one_stage`, except the Game camera
/// is pointed AT the lamp instead of away from it. Everything else is
/// identical — same stage, same alternation, same cache traffic. If the
/// shadow survives this and dies in the other, the deciding input is
/// whether the lamp fell inside the rendering camera's frustum, and
/// `select_point_casters` culling against `camera` is the whole bug.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_views_where_game_also_sees_the_lamp() {
    const LAMP: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(LAMP, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(6.0, 4.0, 0.0);
    shoot(&mut rig, "sees_0_alone.png", view_eye);

    for round in 0..3 {
        // Looking at the origin, so the lamp's sphere is well inside.
        let game = ViewCamera::looking_at(Vec3::new(0.0, 5.0, 8.0), Vec3::new(0.0, 0.5, 0.0));
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(
            &mut rig,
            &format!("sees_1_after_game_{round}.png"),
            view_eye,
        );
    }
}

/// Two lamps, two views, and the Game camera turning.
///
/// The frustum cull is gone, but `shadow_casting_points` still ranks by
/// distance to `camera.position()`, so the ORDER of the chosen lamps —
/// and therefore which cube slot each one occupies — is still a property
/// of whoever is rendering. The cube array and `point_cube_cache` are
/// still the stage's.
///
/// The View camera never moves in any of these. Every picture must show
/// both shadows, in the same two places.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_lamps_while_the_game_camera_turns() {
    // Both lamps, off the inspector.
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build(&[(A, true), (B, true)], true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(5.0, 3.5, 5.0);
    shoot(&mut rig, "turn_0_alone.png", view_eye);

    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        let game = ViewCamera::looking_at(
            Vec3::new(a.cos() * 7.0, 3.0, a.sin() * 7.0),
            Vec3::new(0.0, 0.5, 0.0),
        );
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(&mut rig, &format!("turn_1_game_at_{i}.png"), view_eye);
    }
}

/// The same turning Game camera, with `contact_shadows` ON — which is
/// what the owner's lamps carry now, and what every picture in this file
/// so far was taken without.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn two_lamps_with_contact_shadows_on() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let second = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let view_eye = Vec3::new(5.0, 3.5, 5.0);
    shoot(&mut rig, "contact_0_alone.png", view_eye);

    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        let game = ViewCamera::looking_at(
            Vec3::new(a.cos() * 7.0, 3.0, a.sin() * 7.0),
            Vec3::new(0.0, 0.5, 0.0),
        );
        rig.stage
            .render_with_assets(second, &rig.device, &rig.queue, &rig.resources, &game, 1.0);
        shoot(&mut rig, &format!("contact_1_game_at_{i}.png"), view_eye);
    }
}

/// The owner's configuration exactly: two lamps, both casting, both with
/// `contact_shadows` on, and the camera that MOVES is the one being
/// looked at.
///
/// Every earlier orbit ran with the march off. A contact shadow is a
/// march through this view's own depth buffer, so it is the one shadow
/// that is allowed to change when the camera turns — and #845 marches
/// only the brightest light per pixel, so of two lamps only ever one
/// gets one.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn orbiting_with_two_lamps_and_contact_on() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    for contact in [false, true] {
        let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let tag = if contact { "on" } else { "off" };
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let eye = Vec3::new(a.cos() * 6.0, 3.5, a.sin() * 6.0);
            shoot(&mut rig, &format!("selforbit_{tag}_{i}.png"), eye);
        }
    }
}

/// The same orbit, measured instead of looked at.
///
/// Each lamp's shadow lands on a FIXED patch of floor — the ball's centre
/// traced away from that lamp down to y = 0 — so the camera moving
/// changes where that patch appears on screen and nothing else. Probing
/// the world point and reporting it against open floor is the only way
/// to tell "the shadow moved off screen" from "the shadow stopped being
/// computed".
#[test]
#[ignore = "prints a table; not an assertion"]
fn measure_the_orbit() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    // Far from the ball and inside both lamps' reach: what "lit" means.
    const OPEN: Vec3 = Vec3::new(-3.0, 0.0, 2.5);

    let centre = |lamp: Vec3| {
        let d = (BALL - lamp).normalize();
        BALL + d * (BALL.y / d.y.abs())
    };

    for contact in [false, true] {
        let Some(mut rig) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        eprintln!("\ncontact_shadows = {contact}");
        eprintln!("  cam   shadow(A)  shadow(B)   open    A/open  B/open");
        for i in 0..8 {
            let ang = i as f32 * std::f32::consts::TAU / 8.0;
            let eye = Vec3::new(ang.cos() * 6.0, 3.5, ang.sin() * 6.0);
            let camera = ViewCamera::looking_at(eye, BALL);
            rig.stage.render_with_assets_primary(
                &rig.device,
                &rig.queue,
                &rig.resources,
                &camera,
                1.0,
            );
            let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
            let probe = |w: Vec3| {
                let clip = camera.view_proj(1.0) * w.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32) as u32;
                let y = ((0.5 - ndc.y * 0.5) * SIZE as f32).clamp(0.0, (SIZE - 1) as f32) as u32;
                common::luminance_at(&px, SIZE, x, y, 2)
            };
            let (sa, sb, open) = (probe(centre(A)), probe(centre(B)), probe(OPEN));
            eprintln!(
                "  {i}     {sa:.4}     {sb:.4}    {open:.4}   {:.2}    {:.2}",
                sa / open.max(1e-4),
                sb / open.max(1e-4),
            );
        }
    }
}

/// The orbit measured so that framing cancels exactly.
///
/// A probe at a fixed world point is not enough: at some angles the ball
/// stands between the lens and that point, and the sample then reads the
/// BALL — 1.42x "brighter than open floor", which looks like a missing
/// shadow and is a missing floor. The rig's own comment warns about it
/// and I walked into it anyway.
///
/// So: the same camera renders twice, once with both lamps casting and
/// once with neither, and the two frames are differenced. Occlusion,
/// perspective, penumbra size and tonemap are identical between them;
/// the only thing that differs is the shadow. `dark` is the fraction of
/// the frame the shadows remove, and it must not depend on the camera.
#[test]
#[ignore = "prints a table; not an assertion"]
fn the_orbit_differenced() {
    const A: Vec3 = Vec3::new(-5.151, 3.477, 0.0);
    const B: Vec3 = Vec3::new(-2.651, 3.477, -1.813);
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);

    for contact in [false, true] {
        let Some(mut on) = build_with(&[(A, true), (B, true)], true, true, contact) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let Some(mut off) = build_with(&[(A, false), (B, false)], true, true, contact) else {
            return;
        };
        eprintln!("\ncontact_shadows = {contact}");
        eprintln!("  cam    darkened%   worst pixel");
        for i in 0..8 {
            let ang = i as f32 * std::f32::consts::TAU / 8.0;
            let eye = Vec3::new(ang.cos() * 6.0, 3.5, ang.sin() * 6.0);
            let camera = ViewCamera::looking_at(eye, BALL);
            let shot = |rig: &mut Rig| {
                rig.stage.render_with_assets_primary(
                    &rig.device,
                    &rig.queue,
                    &rig.resources,
                    &camera,
                    1.0,
                );
                read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
            };
            let (a, b) = (shot(&mut on), shot(&mut off));
            let (worst, mean) = compare(&a, &b);
            eprintln!("  {i}      {:.3}       {worst:.3}", mean * 100.0);
        }
    }
}

/// The owner's `project.rendersettings`, verbatim, which no picture in
/// this file has used.
///
/// Every rig above ran with ambient 0, full-rate shading and no temporal
/// pass. `roll-a-ball` ships `ambient_intensity: 300`, `shading_rate: 2`,
/// `temporal_aa: true`, `point_shadows: 32` and
/// `contact_shadow_dominant: true` — and two of those make a frame
/// depend on where the camera WAS, which is the one property none of the
/// earlier reproductions had.
fn owners_rig(casting: bool) -> Option<Rig> {
    let mut rig = build_with(
        &[
            (Vec3::new(-5.151, 3.477, 0.0), casting),
            (Vec3::new(-2.651, 3.477, -1.813), casting),
        ],
        true,
        true,
        true,
    )?;
    rig.resources.insert(kooch_lighting::AmbientLight {
        intensity: 300.0,
        ..Default::default()
    });
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            rate: kooch_render::meshlet::ShadingRate::Half,
            anisotropy: 1,
        });
    rig.resources
        .insert(kooch_render::quality::TemporalSettings::new(
            kooch_render::quality::UpscaleTechnique::Taa,
            100,
            0,
            true,
        ));
    rig.resources.insert(ShadowSettings {
        cascade_texels: 512,
        max_distance: 30.0,
        enabled: true,
        point_shadows: 32,
        ..Default::default()
    });
    Some(rig)
}

/// A slow orbit under those settings, differenced against the same orbit
/// with neither lamp casting. Framing, occlusion, half-rate upsampling
/// and the temporal resolve are identical between the two runs; the only
/// difference is the shadow.
#[test]
#[ignore = "prints a table; not an assertion"]
fn the_owners_settings_orbit() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let (Some(mut on), Some(mut off)) = (owners_rig(true), owners_rig(false)) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    eprintln!("\n  step   darkened%   worst pixel");
    for i in 0..24 {
        let ang = i as f32 * std::f32::consts::TAU / 24.0;
        let eye = Vec3::new(ang.cos() * 6.0, 3.5, ang.sin() * 6.0);
        let camera = ViewCamera::looking_at(eye, BALL);
        let shot = |rig: &mut Rig| {
            rig.stage.render_with_assets_primary(
                &rig.device,
                &rig.queue,
                &rig.resources,
                &camera,
                1.0,
            );
            read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
        };
        let (a, b) = (shot(&mut on), shot(&mut off));
        let (worst, mean) = compare(&a, &b);
        eprintln!("  {i:2}      {:.3}       {worst:.3}", mean * 100.0);
        if i % 8 == 0 {
            shoot(&mut on, &format!("owner_{i}.png"), eye);
        }
    }
}

/// The Game panel's own image, which is what the owner is looking at.
///
/// Every picture above is the primary view. The Game panel is a SECOND
/// `ViewId` on the same stage, and until `view_color_texture` existed no
/// test could read it back. The View panel renders first each frame,
/// exactly as the editor does it, and then this turns the Game camera.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_game_panel_while_its_camera_turns() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let game = rig.stage.create_view(&rig.device, (SIZE, SIZE));
    let editor = ViewCamera::looking_at(Vec3::new(5.0, 3.5, 5.0), BALL);

    for i in 0..8 {
        // The View panel goes first every frame, like the editor's loop.
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &editor, 1.0);
        let ang = i as f32 * std::f32::consts::TAU / 8.0;
        let camera = ViewCamera::looking_at(Vec3::new(ang.cos() * 6.0, 3.0, ang.sin() * 6.0), BALL);
        rig.stage
            .render_with_assets(game, &rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let tex = rig.stage.view_color_texture(game).expect("live view");
        let px = read_rgba8(&rig.device, &rig.queue, tex);
        let path = format!("/tmp/kooch_point_shadows/game_{i}.png");
        image::save_buffer(&path, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {path}");
    }
}

/// "es como si la geometría se estuviera quedando sin mesh… será que se
/// ocluden las meshlets?"
///
/// Each of the six cube faces IS a camera — a 90° perspective view from
/// the lamp — and each runs its own meshlet cull with its own LOD
/// selection against `cubes.size()` texels. So the shadow can lose
/// meshlets the main view kept, and a sphere whose middle meshlets went
/// missing casts a horseshoe: dark at the rim, lit in the centre.
///
/// If the shadow closes up as `target_error_pixels` goes to zero, the
/// LOD selector in the shadow cull is choosing the cut and the holes are
/// its doing. If it does not, the geometry is whole and the shape is
/// something else.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn does_the_shadow_close_up_at_a_finer_lod() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    for target in [1.0f32, 0.25, 0.01] {
        let Some(mut rig) = owners_rig(true) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        rig.resources
            .insert(kooch_render::meshlet::MeshletLodSettings {
                target_error_pixels: target,
            });
        // Straight down on the lamp's own axis, so the whole silhouette
        // of the shadow is in frame and a hole in it cannot hide behind
        // the ball.
        let eye = Vec3::new(2.6, 7.0, 4.0);
        let camera = ViewCamera::looking_at(eye, BALL);
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
        let name = format!("/tmp/kooch_point_shadows/lod_{target}.png");
        image::save_buffer(&name, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {name}");
    }
}

/// The new view (#852), on the owner's scene.
///
/// Nothing on top of the cube's answer, so what comes out is readable as
/// a fault and not as a shade: magenta no caster, blue past range, grey
/// the factor itself.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_point_shadow_factor_view() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::meshlet::MeshletDebugMode::PointShadowFactor);
    for (name, eye) in [
        ("factor_high.png", Vec3::new(2.6, 9.0, 4.0)),
        ("factor_low.png", Vec3::new(5.0, 3.0, 5.0)),
    ] {
        let camera = ViewCamera::looking_at(eye, BALL);
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
        let path = format!("/tmp/kooch_point_shadows/{name}");
        image::save_buffer(&path, &px, SIZE, SIZE, image::ColorType::Rgba8).unwrap();
        eprintln!("wrote {path}");
    }
}

/// The cube view (#852), on the owner's scene.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_point_cube_view() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    rig.resources
        .insert(kooch_render::meshlet::MeshletDebugMode::PointCubeFaces);
    // Straight down, so the floor fills the frame: the view is a
    // SURFACE shader, so it paints only where there is geometry and the
    // sky leaves its cells blank.
    let camera = ViewCamera::looking_at(Vec3::new(0.0, 9.0, 0.2), BALL);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    image::save_buffer(
        "/tmp/kooch_point_shadows/cube_faces.png",
        &px,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();
    eprintln!("wrote cube_faces.png");
}

/// The cube's six faces read **out of the depth texture**, with the
/// contrast stretched to whatever is actually in them.
///
/// 🔴 Every other picture in this file goes through the sampling path,
/// the filter, the bias and a surface shader before it reaches a PNG.
/// Four places a picture can lie, and one of them already did: the 3×2
/// grid normalises by the light's `range`, so a ball at 5.4 m against a
/// floor at 5.9 m is two greys a hundredth apart and reads as noise.
///
/// This one copies the layer and stretches min..max. If the occluder is
/// in the map it is unmissable here, and if it is not, that is not an
/// interpretation either.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_cube_faces_raw() {
    const BALL: Vec3 = Vec3::new(0.0, 0.5, 0.0);
    let Some(mut rig) = owners_rig(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let camera = ViewCamera::looking_at(Vec3::new(0.0, 9.0, 0.2), BALL);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);

    let cubes = rig.stage.shadow_cubes_texture().expect("cubes").clone();
    let size = cubes.size().width;
    const NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    for face in 0..6u32 {
        let depth = read_face_depth(&rig.device, &rig.queue, &cubes, face);
        let recorded: Vec<f32> = depth.iter().copied().filter(|d| *d > 0.0).collect();
        if recorded.is_empty() {
            eprintln!("face {} — empty", NAMES[face as usize]);
            continue;
        }
        let lo = recorded.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = recorded.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // Reversed-Z: the stored value is `near / distance`, so BIGGER
        // is CLOSER to the lamp. Painted so that closer is darker, the
        // way an occluder reads to a human.
        let px: Vec<u8> = depth
            .iter()
            .map(|d| {
                if *d <= 0.0 {
                    return 255;
                }
                let t = ((d - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0);
                (255.0 * (1.0 - t)) as u8
            })
            .collect();
        let path = format!("/tmp/kooch_point_shadows/raw_face_{face}.png");
        image::save_buffer(&path, &px, size, size, image::ColorType::L8).unwrap();
        eprintln!(
            "face {} — {} texels recorded, near/dist in {lo:.5}..{hi:.5} \
             ({:.2}..{:.2} m)",
            NAMES[face as usize],
            recorded.len(),
            0.1 / hi,
            0.1 / lo,
        );
    }
}

/// One layer of the cube array, as f32 depth.
fn read_face_depth(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
) -> Vec<f32> {
    let size = texture.size().width;
    let padded = (size * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cube_face_readback"),
        size: (padded * size) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size),
            },
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        let start = (row * padded) as usize;
        let bytes = &data[start..start + (size * 4) as usize];
        out.extend_from_slice(bytemuck::cast_slice::<u8, f32>(bytes));
    }
    drop(data);
    staging.unmap();
    out
}

/// The reported scene, by its own numbers, not by my memory of them.
///
/// 🔴 `owners_rig` above had the lamps in the wrong place. It put both
/// off to one side, so the ball always landed on the **+X** face. The
/// scene the owner is looking at — `point_light_test.scene` — has a lamp
/// at the ORIGIN, `(0, 3.4766, 0)`, directly over a ball at `(0, 1, 0)`,
/// which puts the occluder on **−Y**: the one face where a flat floor
/// records a single constant depth, because the stored value is
/// `near / major-axis magnitude` and the major axis there IS the height.
/// Five reproductions never put anything on that face.
///
/// The other two corrections: the ball floats a metre up, not half a
/// metre, and both lamps have `contact_shadows: false`.
fn reported_scene(casting: bool) -> Option<Rig> {
    let mut rig = build_with(
        &[
            (Vec3::new(0.0, 3.4765813, 0.0), casting),
            (Vec3::new(-5.1512194, 3.4765813, 0.0), casting),
        ],
        true,
        true,
        false,
    )?;
    // The prefab's override: `position Vec3((0.0, 1.0, 0.0))`. Scale
    // kept — `move_occluder` drops it, which quietly doubles the ball.
    kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(&rig.resources).for_each(
        |(_, transform)| {
            if transform.matrix.w_axis.y > 0.0 {
                transform.matrix = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::splat(0.5));
            }
        },
    );
    rig.resources.insert(kooch_lighting::AmbientLight {
        intensity: 300.0,
        ..Default::default()
    });
    rig.resources
        .insert(kooch_render::quality::ShadingSettings {
            compute: true,
            rate: kooch_render::meshlet::ShadingRate::Half,
            anisotropy: 1,
        });
    rig.resources
        .insert(kooch_render::quality::TemporalSettings::new(
            kooch_render::quality::UpscaleTechnique::Taa,
            100,
            0,
            true,
        ));
    rig.resources.insert(ShadowSettings {
        cascade_texels: 512,
        max_distance: 30.0,
        enabled: true,
        point_shadows: 32,
        ..Default::default()
    });
    Some(rig)
}

/// That scene from its own camera: shaded, then the factor, then the six
/// faces raw.
#[test]
#[ignore = "writes PNGs to look at; not an assertion"]
fn the_reported_scene() {
    const BALL: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    const EYE: Vec3 = Vec3::new(0.0, 6.0342627, 9.728698);
    let Some(mut rig) = reported_scene(true) else {
        eprintln!("no GPU adapter, skipping");
        return;
    };
    let camera = ViewCamera::looking_at(EYE, BALL);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    image::save_buffer(
        "/tmp/kooch_point_shadows/scene_shaded.png",
        &px,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();

    rig.resources
        .insert(kooch_render::meshlet::MeshletDebugMode::PointShadowFactor);
    rig.stage
        .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
    let px = read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture());
    image::save_buffer(
        "/tmp/kooch_point_shadows/scene_factor.png",
        &px,
        SIZE,
        SIZE,
        image::ColorType::Rgba8,
    )
    .unwrap();

    let cubes = rig.stage.shadow_cubes_texture().expect("cubes").clone();
    const NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    for face in 0..6u32 {
        let depth = read_face_depth(&rig.device, &rig.queue, &cubes, face);
        let recorded: Vec<f32> = depth.iter().copied().filter(|d| *d > 0.0).collect();
        if recorded.is_empty() {
            eprintln!("face {} — empty", NAMES[face as usize]);
            continue;
        }
        let lo = recorded.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = recorded.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let px: Vec<u8> = depth
            .iter()
            .map(|d| {
                if *d <= 0.0 {
                    return 255;
                }
                let t = ((d - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0);
                (255.0 * (1.0 - t)) as u8
            })
            .collect();
        image::save_buffer(
            format!("/tmp/kooch_point_shadows/scene_face_{face}.png"),
            &px,
            cubes.size().width,
            cubes.size().width,
            image::ColorType::L8,
        )
        .unwrap();
        eprintln!(
            "face {} — {} texels, {:.2}..{:.2} m",
            NAMES[face as usize],
            recorded.len(),
            0.1 / hi,
            0.1 / lo,
        );
    }
    eprintln!("wrote scene_shaded.png, scene_factor.png, scene_face_*.png");
}

/// One lamp, then two. The scene is otherwise identical.
#[test]
#[ignore = "prints a table; not an assertion"]
fn one_lamp_then_two() {
    for (label, lamps) in [
        ("one lamp  ", &[(Vec3::new(0.0, 3.4765813, 0.0), true)][..]),
        (
            "two lamps ",
            &[
                (Vec3::new(0.0, 3.4765813, 0.0), true),
                (Vec3::new(-5.1512194, 3.4765813, 0.0), true),
            ][..],
        ),
    ] {
        let Some(mut rig) = build_with(lamps, true, true, false) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(&rig.resources)
            .for_each(|(_, transform)| {
                if transform.matrix.w_axis.y > 0.0 {
                    transform.matrix = Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                        * Mat4::from_scale(Vec3::splat(0.5));
                }
            });
        let camera = ViewCamera::looking_at(Vec3::new(0.0, 6.03, 9.73), Vec3::new(0.0, 1.0, 0.0));
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let cubes = rig.stage.shadow_cubes_texture().expect("cubes").clone();
        let mut closest = 0.0f32;
        for face in 0..6u32 {
            for d in read_face_depth(&rig.device, &rig.queue, &cubes, face) {
                closest = closest.max(d);
            }
        }
        let metres = 0.1 / closest;
        let verdict = if metres < 3.4 {
            "ball IS there"
        } else {
            "floor only"
        };
        eprintln!("  {label} slot 0's closest = {metres:>6.2} m   {verdict}");
    }
}

/// Where does the ball have to stand before the cube records it?
///
/// The cube holds the floor at 3.48 m on every face. A ball at `y = 1`
/// puts its top 1.98 m under the lamp, so "the closest thing in the
/// whole cube" is 3.48 m when the ball is missing and 1.98 m when it is
/// there. One number, no picture to read.
#[test]
#[ignore = "prints a table; not an assertion"]
fn where_the_ball_enters_the_cube() {
    eprintln!("\n    ball position        closest thing in the cube");
    for (x, y) in [
        (0.0f32, 1.0f32),
        (0.25, 1.0),
        (0.5, 1.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (4.0, 1.0),
        (0.0, 0.5),
        (0.0, 1.5),
        (0.0, 2.0),
        (0.0, 3.0),
    ] {
        let Some(mut rig) = reported_scene(true) else {
            eprintln!("no GPU adapter, skipping");
            return;
        };
        let at = Vec3::new(x, y, 0.0);
        kooch_ecs::query::Query::<(&MeshRenderer, &mut GlobalTransform)>::new(&rig.resources)
            .for_each(|(_, transform)| {
                if transform.matrix.w_axis.y > 0.0 {
                    transform.matrix =
                        Mat4::from_translation(at) * Mat4::from_scale(Vec3::splat(0.5));
                }
            });
        let camera = ViewCamera::looking_at(Vec3::new(0.0, 6.03, 9.73), at);
        rig.stage
            .render_with_assets_primary(&rig.device, &rig.queue, &rig.resources, &camera, 1.0);
        let cubes = rig.stage.shadow_cubes_texture().expect("cubes").clone();
        let mut closest = 0.0f32;
        for face in 0..6u32 {
            for d in read_face_depth(&rig.device, &rig.queue, &cubes, face) {
                closest = closest.max(d);
            }
        }
        let metres = if closest > 0.0 { 0.1 / closest } else { 0.0 };
        let verdict = if metres < 3.4 {
            "ball IS there"
        } else {
            "floor only"
        };
        eprintln!("    ({x:>4.2}, {y:>4.2}, 0.00)     {metres:>6.2} m   {verdict}");
    }
}

/// The AABB-vs-frustum test from `meshlet_cull/atomic.wgsl`, on the CPU,
/// so a rejection can name the plane that made it.
///
/// 🔴 It says every meshlet is KEPT on the -Y face, and the GPU dropped
/// them all — which is what pointed at #853. The two disagreeing was the
/// finding: the maths was right and the matrix the GPU read was somebody
/// else's.
#[test]
#[ignore = "prints a table; not an assertion"]
fn which_plane_rejects_the_ball() {
    use glam::Vec4;
    fn outside(clip_from_local: Mat4, lo: Vec3, hi: Vec3) -> Option<(usize, f32)> {
        let center = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        let r = clip_from_local.transpose();
        let rows = [r.x_axis, r.y_axis, r.z_axis, r.w_axis];
        let planes: [Vec4; 5] = [
            rows[3] + rows[0],
            rows[3] - rows[0],
            rows[3] + rows[1],
            rows[3] - rows[1],
            rows[2],
        ];
        for (i, p) in planes.iter().enumerate() {
            let n = Vec3::new(p.x, p.y, p.z);
            let len = n.length();
            let plane = *p / len;
            let n = Vec3::new(plane.x, plane.y, plane.z);
            // WGSL `sign`, which is 0 at 0 — not Rust's `signum`, which
            // is 1. Same answer here (a zero normal component cancels
            // the term either way), spelled the way the shader spells it.
            let wsign = |v: f32| {
                if v > 0.0 {
                    1.0
                } else if v < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            };
            let flipped = half * Vec3::new(wsign(n.x), wsign(n.y), wsign(n.z));
            let d = n.dot(center + flipped);
            if d <= -plane.w {
                return Some((i, len));
            }
        }
        None
    }

    let lamp = Vec3::new(0.0, 3.4765813, 0.0);
    const NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
    const PLANES: [&str; 5] = ["left", "right", "bottom", "top", "near"];
    for ball_y in [0.5f32, 1.0, 2.0] {
        let model = Mat4::from_translation(Vec3::new(0.0, ball_y, 0.0))
            * Mat4::from_scale(Vec3::splat(0.5));
        for face in 0..6usize {
            let vp = kooch_render::shadow::face_view_proj(lamp, face, 0.1);
            let verdict = match outside(vp * model, Vec3::splat(-1.0), Vec3::splat(1.0)) {
                Some((i, len)) => format!("REJECTED by {} (|n| = {len:.6})", PLANES[i]),
                None => "kept".to_string(),
            };
            eprintln!("  ball y={ball_y:>4.1}  face {}  {verdict}", NAMES[face]);
        }
    }

    // Per-meshlet now: the whole-mesh box passing says nothing about the
    // sub-boxes the cull actually tests.
    let ball = kooch_render::meshlet::build_meshlets_lod_chain(
        &common::build_sphere_mesh(32, 48),
        kooch_render::meshlet::DEFAULT_MAX_VERTICES,
        kooch_render::meshlet::DEFAULT_MAX_TRIANGLES,
        0.5,
        Default::default(),
    )
    .unwrap();
    let model =
        Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)) * Mat4::from_scale(Vec3::splat(0.5));
    eprintln!("\n  {} meshlets in the ball", ball.meshlets.len());
    for face in 0..6usize {
        let vp = kooch_render::shadow::face_view_proj(lamp, face, 0.1);
        let clip = vp * model;
        let mut kept = 0usize;
        let mut by_plane = [0usize; 5];
        for m in &ball.meshlets {
            match outside(clip, Vec3::from(m.aabb_min), Vec3::from(m.aabb_max)) {
                Some((i, _)) => by_plane[i] += 1,
                None => kept += 1,
            }
        }
        eprintln!(
            "  face {}  kept {kept:>3}  rejected L{} R{} B{} T{} N{}",
            NAMES[face], by_plane[0], by_plane[1], by_plane[2], by_plane[3], by_plane[4],
        );
    }
    let degenerate = ball
        .meshlets
        .iter()
        .filter(|m| m.aabb_min == m.aabb_max)
        .count();
    eprintln!("  {degenerate} meshlets have a degenerate (zero-volume) AABB");
}

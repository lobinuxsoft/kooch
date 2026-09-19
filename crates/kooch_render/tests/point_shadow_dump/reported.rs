//! A reported scene, rebuilt: one lamp then two, and where a ball meets the cube.

use super::*;

/// One layer of the cube array, as f32 depth.
pub(super) fn read_face_depth(
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

/// That scene from its own camera: shaded, then the factor, then the six faces raw.
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

/// The AABB-vs-frustum test from `meshlet_cull/atomic.wgsl`, on the CPU, so a rejection can name
/// the plane that made it.
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

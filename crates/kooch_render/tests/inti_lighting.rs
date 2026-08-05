//! #441's acceptance: lights that light.
//!
//! The assertion this whole issue exists to make true is the first test
//! below — **a scene with a light and a scene without one cannot render
//! the same**. Before Inti they rendered identically, because the
//! shading model was `normal × 0.5 + 0.5` and no render crate read a
//! light at all.
//!
//! The rest pin the shape of the result rather than exact values: a
//! sphere lit from the camera has to be bright where it faces the light
//! and dark where it faces away. Pinning pixel values would break on
//! every legitimate BRDF change; pinning the contrast breaks only when
//! shading stops responding to lights, which is the failure that
//! shipped for as long as this engine has existed.
//!
//! Run with:
//!   cargo test -p kooch_render --test inti_lighting

mod common;

use common::{build_sphere_mesh, try_acquire_device};
use glam::{Mat4, Quat, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_render::material::{Material, MaterialPipeline};
use kooch_render::meshlet::{
    MeshletDebugMode, MeshletRenderStage, MeshletRenderStageConfig, build_default_meshlets,
};

const SIZE: u32 = 128;

/// A sphere at the origin, a camera looking at it from +Z, and no
/// lights. Everything else in this file starts here.
struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: Resources,
    stage: MeshletRenderStage,
    view_proj: Mat4,
    cam_pos: Vec3,
}

fn rig() -> Option<Rig> {
    let (device, queue) = try_acquire_device()?;

    let mesh = build_sphere_mesh(24, 24);
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (SIZE, SIZE),
            instance_capacity: 8,
            meshlet_capacity: 1024,
            ..Default::default()
        },
    );

    // Mid-roughness dielectric white: responds to both the diffuse and
    // the specular lobe, so a change in either shows up.
    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    let material_guid = Guid::new_v4();
    materials.register(
        &queue,
        material_guid,
        &Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.5, 0.0),
    );
    resources.insert(materials);

    let mesh_guid = Guid::new_v4();
    stage.ensure_gpu_mesh(&device, mesh_guid, &meshlet_mesh);

    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(mesh_guid),
            material: Some(material_guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut resources);

    let cam_pos = Vec3::new(0.0, 0.0, 3.0);
    let view = Mat4::look_at_rh(cam_pos, Vec3::ZERO, Vec3::Y);
    let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);

    Some(Rig {
        device,
        queue,
        resources,
        stage,
        view_proj: proj * view,
        cam_pos,
    })
}

/// Adds a directional light pointing along `direction`.
///
/// The direction comes from the transform's -Z, never from a field —
/// that is the scope correction #441 was rewritten around, so the test
/// exercises it the same way an author would: by rotating the entity.
fn add_directional(resources: &mut Resources, direction: Vec3, intensity: f32) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize());
    let mut commands = Commands::new();
    commands
        .spawn(resources)
        .insert(DirectionalLight {
            active: true,
            color: Vec3::ONE,
            intensity,
            cast_shadows: false,
        })
        .insert(GlobalTransform {
            matrix: Mat4::from_quat(rotation),
        });
    commands.apply(resources);
}

fn render(rig: &mut Rig) -> Vec<u8> {
    rig.stage.render_with_assets_primary(
        &rig.device,
        &rig.queue,
        &rig.resources,
        rig.view_proj,
        rig.cam_pos,
    );
    read_rgba8(&rig.device, &rig.queue, rig.stage.color_texture())
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * SIZE + x) * 4) as usize;
    [
        pixels[idx],
        pixels[idx + 1],
        pixels[idx + 2],
        pixels[idx + 3],
    ]
}

/// sRGB electrical value → linear.
///
/// Comparisons happen in linear because that is where the shading
/// happened. In 8-bit sRGB the transfer function plus ACES compress a
/// genuine 2× difference in irradiance down to about 1.1× in the byte,
/// which makes a working BRDF look like a broken one.
fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Mean linear luminance over a small box, so one stray pixel on a
/// silhouette cannot decide a test.
fn brightness(pixels: &[u8], cx: u32, cy: u32) -> f32 {
    let half = 3;
    let mut total = 0.0;
    let mut count = 0.0;
    for y in (cy - half)..=(cy + half) {
        for x in (cx - half)..=(cx + half) {
            let p = pixel(pixels, x, y);
            total += 0.2126 * srgb_to_linear(p[0])
                + 0.7152 * srgb_to_linear(p[1])
                + 0.0722 * srgb_to_linear(p[2]);
            count += 1.0;
        }
    }
    total / count
}

/// Distance from the centre to the sphere's silhouette, measured on the
/// centre row rather than derived from the projection. Hard-coding a
/// pixel offset makes a test that samples the background the day
/// someone changes the camera, and a background sample passes every
/// contrast assertion for the wrong reason.
fn silhouette_radius(pixels: &[u8]) -> u32 {
    let cy = SIZE / 2;
    let mut radius = 0;
    for x in (0..SIZE / 2).rev() {
        // Alpha is the coverage sentinel: the deferred paths write 1
        // for any pixel a meshlet covered and leave 0 for background.
        if pixel(pixels, x, cy)[3] == 0 {
            break;
        }
        radius = SIZE / 2 - x;
    }
    radius
}

/// 🔴 The reason this issue exists.
#[test]
fn a_scene_with_a_light_and_one_without_cannot_render_the_same() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let unlit = render(&mut rig);
    add_directional(&mut rig.resources, Vec3::NEG_Z, 20_000.0);
    let lit = render(&mut rig);

    assert_ne!(
        unlit, lit,
        "adding a DirectionalLight changed nothing on screen — which is \
         exactly the state #441 was opened to end",
    );
    assert!(
        brightness(&lit, SIZE / 2, SIZE / 2) > brightness(&unlit, SIZE / 2, SIZE / 2),
        "the lit render should be brighter where the light faces the surface",
    );
}

/// #441's other acceptance criterion, near-verbatim: a sphere lit by
/// one `DirectionalLight` has to be bright where it faces the light and
/// dark where it faces away.
///
/// Measured across the light's axis rather than from the centre
/// outwards. A radial profile looks like the obvious test and is a poor
/// one: at the silhouette the surface still catches a strong grazing
/// specular, and ACES compresses what is a 1.6× difference in radiance
/// into 1.3× on screen. Pole against pole, the two samples differ by
/// `N·L = +0.7` against `N·L = -0.7`, and the second one receives
/// nothing at all.
#[test]
fn a_lit_sphere_is_bright_facing_the_light_and_dark_facing_away() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Pointing along -Z: away from the camera, into the sphere's face.
    //
    // Ambient off. It arrives from every direction by construction, so
    // it contributes almost equally at the pole and at the edge — which
    // is exactly the difference this test measures, and at the default
    // intensity it is the same order of magnitude as the key light.
    // Leaving it on would test the ambient term while claiming to test
    // the directional one.
    rig.resources.insert(kooch_lighting::AmbientLight {
        intensity: 0.0,
        ..Default::default()
    });
    // Straight down, so the sphere's top half faces it and the bottom
    // half faces away.
    add_directional(&mut rig.resources, Vec3::NEG_Y, 2_000.0);
    let pixels = render(&mut rig);

    let radius = silhouette_radius(&pixels);
    assert!(
        radius > 20,
        "the sphere covers only {radius} px of the centre row; the rig \
         is not rendering what the rest of this test assumes",
    );

    // Row 0 is the top of the image (`frag_coord_to_ndc` flips y), so
    // the smaller row index is the lit pole.
    let offset = radius * 7 / 10;
    let lit_pole = brightness(&pixels, SIZE / 2, SIZE / 2 - offset);
    let dark_pole = brightness(&pixels, SIZE / 2, SIZE / 2 + offset);

    assert!(
        lit_pole > 0.1,
        "the pole facing a 2 000 lux light is near-black ({lit_pole:.4} \
         linear) — check the exposure before the BRDF",
    );
    assert!(
        dark_pole < 0.02,
        "the pole facing away from the only light in the scene should \
         receive nothing, got {dark_pole:.4} linear",
    );
    assert!(
        lit_pole > dark_pole * 5.0,
        "lit {lit_pole:.4} vs unlit {dark_pole:.4} (linear) — the \
         shading is barely responding to N·L",
    );
}

/// The ambient term is what keeps the unlit side of the sphere above
/// being a silhouette, so it has to be measurable on its own.
#[test]
fn ambient_alone_lifts_the_side_no_light_reaches() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_directional(&mut rig.resources, Vec3::NEG_Y, 2_000.0);

    rig.resources.insert(kooch_lighting::AmbientLight {
        intensity: 0.0,
        ..Default::default()
    });
    let without = render(&mut rig);

    rig.resources
        .insert(kooch_lighting::AmbientLight::default());
    let with = render(&mut rig);

    let radius = silhouette_radius(&without);
    let offset = radius * 7 / 10;
    let dark_without = brightness(&without, SIZE / 2, SIZE / 2 + offset);
    let dark_with = brightness(&with, SIZE / 2, SIZE / 2 + offset);

    assert!(
        dark_with > dark_without,
        "with no environment map, ambient is the only thing between a \
         metal in shadow and a black hole: {dark_with:.4} vs \
         {dark_without:.4} (linear)",
    );
}

/// Same sphere, light pointing at the camera instead: the face we can
/// see is now the far side, lit only by ambient.
#[test]
fn a_sphere_lit_from_behind_is_darker_than_one_lit_from_the_front() {
    let Some(mut rig_front) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_directional(&mut rig_front.resources, Vec3::NEG_Z, 20_000.0);
    let front = render(&mut rig_front);

    let mut rig_back = rig().expect("device acquired once already");
    add_directional(&mut rig_back.resources, Vec3::Z, 20_000.0);
    let back = render(&mut rig_back);

    assert!(
        brightness(&front, SIZE / 2, SIZE / 2) > brightness(&back, SIZE / 2, SIZE / 2) * 2.0,
        "turning the light around barely changed the image, so the \
         shading is not reading the light's direction",
    );
}

/// The old shading model is still reachable — it just stopped being
/// what you get by default.
#[test]
fn the_normals_debug_view_differs_from_lit_shading() {
    let Some(mut rig) = rig() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    add_directional(&mut rig.resources, Vec3::NEG_Z, 20_000.0);

    let lit = render(&mut rig);
    rig.resources.insert(MeshletDebugMode::Normals);
    let normals = render(&mut rig);

    assert_ne!(
        lit, normals,
        "MeshletDebugMode::Normals rendered the same as Off, so either \
         the dropdown does nothing or Off is still the debug view",
    );
}

/// Reads a 2-D Rgba8Unorm texture back into a tightly packed buffer.
fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let (w, h) = (size.width, size.height);
    let unpadded = w * 4;
    let padded = unpadded.div_ceil(256) * 256;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("inti_readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("inti_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
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
    let mut out = Vec::with_capacity((unpadded * h) as usize);
    for row in 0..h {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    staging.unmap();
    out
}

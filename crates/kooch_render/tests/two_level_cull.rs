//! GPU integration test: the two-level cull (#1002) draws the SAME
//! meshlets the one-level rectangle did.
//!
//! 🔴 This exists because naga is not enough. The first version of the
//! chunked path validated as a shader module and every unit test
//! passed, and it still died on the first frame: the chunk list was
//! bound as storage AND used as the source of a
//! `dispatch_workgroups_indirect` in one compute pass, which wgpu
//! refuses. Nothing short of running it on a device could see that.
//!
//! Asserts:
//!   - the chunked path submits without a validation error
//!   - its survivor set is IDENTICAL to the rectangle's, not merely
//!     similar — a reshape that changes what is drawn is a bug even
//!     when it is faster
//!   - a screen-size threshold rejects a distant instance and leaves a
//!     near one alone

mod common;

use common::{build_cube_mesh, build_sphere_mesh, try_acquire_device};
use glam::{Mat4, Vec3};
use kooch_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, GlobalMeshPool, LodConfig, MeshInstance, MeshletCull,
    MeshletCullPipelines, MeshletScene, SceneCullParams, build_default_meshlets,
    build_meshlets_lod_chain, chunks_for,
};

struct Rig {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pool: kooch_render::meshlet::GpuGlobalMeshPool,
    scene: MeshletScene,
    pipelines: MeshletCullPipelines,
    cull: MeshletCull,
    instances: u32,
    per_mesh: u32,
}

/// A pool holding one cube and one sphere, so `meshlets_per_mesh` is
/// bigger than what most instances need — which is the whole condition
/// #1002 is about.
fn rig(device: wgpu::Device, queue: wgpu::Queue, places: &[Vec3]) -> Rig {
    let cube = build_default_meshlets(&build_cube_mesh()).expect("cube meshlets");
    let sphere = build_default_meshlets(&build_sphere_mesh(16, 24)).expect("sphere meshlets");

    let mut pool = GlobalMeshPool::new();
    let cube_handle = pool.register(&cube);
    let sphere_handle = pool.register(&sphere);
    let gpu_pool = pool.upload(&device);
    let per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);

    // Every instance but the last is a cube; the sphere is what makes
    // the stride wide.
    let instances: Vec<MeshInstance> = places
        .iter()
        .enumerate()
        .map(|(i, at)| {
            let mesh = if i + 1 == places.len() {
                sphere_handle.mesh_id
            } else {
                cube_handle.mesh_id
            };
            MeshInstance::new(Mat4::from_translation(*at), mesh, 0)
        })
        .collect();

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let count = instances.len() as u32;
    let threads = count * per_mesh;
    let mut cull = MeshletCull::new(&device, threads.max(1) * 2, DEFAULT_MAX_TRIANGLES as u32);
    cull.ensure_capacity(&device, threads);
    cull.ensure_group_capacity(&device, threads);
    cull.ensure_chunk_capacity(&device, chunks_for(count, per_mesh));

    Rig {
        pipelines: MeshletCullPipelines::new(&device),
        device,
        queue,
        pool: gpu_pool,
        scene,
        cull,
        instances: count,
        per_mesh,
    }
}

impl Rig {
    fn params(&self, min_pixels: f32) -> (CullParams, SceneCullParams) {
        let cam = Vec3::new(0.0, 0.5, 6.0);
        let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
        let proj = kooch_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 500.0);
        let view_proj = proj * view;
        let scale = kooch_render::meshlet::projection_scale_y(view_proj);
        (
            CullParams::new(view_proj, cam, self.per_mesh)
                .with_lod(720.0, scale, 1.0)
                .with_min_screen_pixels(min_pixels),
            SceneCullParams::new(self.instances, self.per_mesh)
                .with_groups(self.instances * self.per_mesh)
                .with_chunks(chunks_for(self.instances, self.per_mesh)),
        )
    }

    /// Runs one cull and reads back the packed survivors, sorted so two
    /// runs are comparable — the atomic append order is not stable and
    /// is not supposed to be.
    fn survivors(&self, two_level: bool, min_pixels: f32) -> Vec<u32> {
        let (cull_params, scene_params) = self.params(min_pixels);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("two_level_cull_encoder"),
            });
        if two_level {
            self.cull.dispatch_scene_pool_atomic_chunked(
                &self.pipelines,
                &self.device,
                &self.queue,
                &mut encoder,
                &self.pool,
                &self.scene,
                &cull_params,
                &scene_params,
            );
        } else {
            self.cull.dispatch_scene_pool_atomic(
                &self.pipelines,
                &self.device,
                &self.queue,
                &mut encoder,
                &self.pool,
                &self.scene,
                &cull_params,
                &scene_params,
            );
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        let count = common::read_u32(
            &self.device,
            &self.queue,
            self.cull.visible_count_buffer(),
            0,
        );
        let mut ids: Vec<u32> = common::read_buffer_to_vec(
            &self.device,
            &self.queue,
            self.cull.visible_meshlets_buffer(),
            count as u64,
        );
        ids.sort_unstable();
        ids
    }
}

/// The reshape must be invisible. Same scene, same camera, same
/// survivors — the only difference is how many threads were asked to
/// find them.
#[test]
fn both_culls_agree_on_survivors() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let rig = rig(
        device,
        queue,
        &[
            Vec3::new(-1.5, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
            // Behind the camera — both paths must drop it.
            Vec3::new(0.0, 0.0, 200.0),
            Vec3::new(0.0, 0.0, -2.0),
        ],
    );

    let rectangle = rig.survivors(false, 0.0);
    let chunked = rig.survivors(true, 0.0);

    assert!(!rectangle.is_empty(), "the one-level cull found nothing");
    assert_eq!(
        rectangle, chunked,
        "the two-level cull changed what is drawn",
    );
}

/// A reach of zero draws everything the frustum holds; a large one
/// starts dropping the far instance first.
#[test]
fn the_reach_drops_the_far_instance() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };
    let rig = rig(
        device,
        queue,
        &[Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -300.0)],
    );

    let everything = rig.survivors(true, 0.0);
    let near_only = rig.survivors(true, 64.0);

    assert!(
        !everything.is_empty(),
        "nothing survived with the reach off"
    );
    assert!(
        near_only.len() < everything.len(),
        "a 64-pixel reach dropped nothing: {} vs {}",
        near_only.len(),
        everything.len(),
    );
    assert!(
        !near_only.is_empty(),
        "the reach also ate the instance in front of the camera",
    );
}

/// 🔴 The test above builds its meshes with `build_default_meshlets`,
/// which is SINGLE LOD: every meshlet is a root, and a root always
/// passes the selector. So it never ran the LOD descent at all — the
/// part of the cull the two-level split had to reproduce exactly, and
/// the part that decides which of a mesh's several versions is drawn.
///
/// A chain, many instances, and distances spread far enough that the
/// selector lands on different levels for different copies. If the two
/// paths disagree here, the picture disagrees: a meshlet drawn at the
/// wrong level overlaps the one that should have replaced it.
#[test]
fn both_culls_agree_down_the_lod_chain() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let chain = build_meshlets_lod_chain(
        &build_sphere_mesh(24, 32),
        64,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("lod chain");
    assert!(
        chain
            .meshlets
            .iter()
            .any(|m| m.parent_meshlet_index != u32::MAX),
        "the fixture has no chain, so this test would prove nothing",
    );

    // A cube beside it, so `meshlets_per_mesh` is the sphere's and the
    // cubes pay a stride they do not need — the condition #1002 is
    // about, and the one that makes the two paths differ in shape.
    let cube = build_default_meshlets(&build_cube_mesh()).expect("cube meshlets");

    let mut pool = GlobalMeshPool::new();
    let chain_handle = pool.register(&chain);
    let cube_handle = pool.register(&cube);
    let gpu_pool = pool.upload(&device);
    let per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);

    // Down the z axis, so consecutive copies sit at different projected
    // sizes and the selector descends at different points along the row.
    let mut instances = Vec::new();
    for i in 0..64u32 {
        let z = -(i as f32) * 4.0;
        let mesh = if i % 3 == 0 {
            cube_handle.mesh_id
        } else {
            chain_handle.mesh_id
        };
        instances.push(MeshInstance::new(
            Mat4::from_translation(Vec3::new(((i % 5) as f32 - 2.0) * 3.0, 0.0, z)),
            mesh,
            0,
        ));
    }

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let count = instances.len() as u32;
    let threads = count * per_mesh;
    let mut cull = MeshletCull::new(&device, threads.max(1) * 2, DEFAULT_MAX_TRIANGLES as u32);
    cull.ensure_capacity(&device, threads);
    cull.ensure_group_capacity(&device, threads);
    cull.ensure_chunk_capacity(&device, chunks_for(count, per_mesh));

    let rig = Rig {
        pipelines: MeshletCullPipelines::new(&device),
        device,
        queue,
        pool: gpu_pool,
        scene,
        cull,
        instances: count,
        per_mesh,
    };

    let rectangle = rig.survivors(false, 0.0);
    let chunked = rig.survivors(true, 0.0);

    assert!(
        rectangle.len() > 64,
        "only {} survivors — the fixture is not exercising the chain",
        rectangle.len(),
    );
    assert_eq!(
        rectangle,
        chunked,
        "the two culls picked different LOD levels: {} vs {} survivors",
        rectangle.len(),
        chunked.len(),
    );
}

/// 🔴 The instance-level frustum test is a rejection the one-level cull
/// never made, and the tests above put everything comfortably in the
/// middle of the screen — where a bounding sphere that is too small, or
/// centred wrong, or unscaled, looks exactly like a correct one.
///
/// This is the case that tells them apart: instances straddling the
/// frustum planes, where part of the mesh is on screen and its centre is
/// not. Rejecting one of those loses a whole model, which is what
/// "several models have no mesh" looks like from the outside.
#[test]
fn nothing_is_lost_at_the_frustum_edge() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // A ring of instances at the edges and corners of a 60° view, at
    // several depths, plus a few big ones the camera sits inside.
    let mut places = Vec::new();
    for depth in [3.0f32, 8.0, 20.0] {
        let half = depth * (30.0f32.to_radians()).tan();
        for (x, y) in [
            (half, 0.0),
            (-half, 0.0),
            (0.0, half),
            (0.0, -half),
            (half, half),
            (-half, -half),
            // Just past the edge: some meshlets still cross into view.
            (half * 1.15, 0.0),
            (-half * 1.15, half * 1.15),
        ] {
            places.push(Vec3::new(x, y, -depth));
        }
    }
    // Around and behind the eye, which sits at (0, 0.5, 6).
    places.push(Vec3::new(0.0, 0.5, 6.0));
    places.push(Vec3::new(0.0, 0.5, 5.0));

    let rig = rig(device, queue, &places);

    let rectangle = rig.survivors(false, 0.0);
    let chunked = rig.survivors(true, 0.0);

    assert!(!rectangle.is_empty(), "the fixture put nothing on screen");
    assert_eq!(
        rectangle,
        chunked,
        "the instance cull dropped what the meshlet cull kept: {} vs {}",
        rectangle.len(),
        chunked.len(),
    );
}

/// A scaled instance's bounding sphere has to be scaled with it.
///
/// An unscaled radius is invisible at scale 1 — which is what every
/// other fixture here uses — and drops the object the moment anyone
/// enlarges it in the editor.
#[test]
fn a_scaled_instance_keeps_its_bounds() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    let cube = build_default_meshlets(&build_cube_mesh()).expect("cube meshlets");
    let sphere = build_default_meshlets(&build_sphere_mesh(16, 24)).expect("sphere meshlets");
    let mut pool = GlobalMeshPool::new();
    let cube_handle = pool.register(&cube);
    let sphere_handle = pool.register(&sphere);
    let gpu_pool = pool.upload(&device);
    let per_mesh = gpu_pool.max_meshlets_per_mesh.max(1);

    // Big and off to the side: at scale 1 its centre is outside the
    // frustum, and only the scaled radius brings it back in.
    let instances = vec![
        MeshInstance::new(
            Mat4::from_scale_rotation_translation(
                Vec3::splat(20.0),
                glam::Quat::IDENTITY,
                Vec3::new(-14.0, 0.0, -6.0),
            ),
            cube_handle.mesh_id,
            0,
        ),
        MeshInstance::new(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)),
            sphere_handle.mesh_id,
            0,
        ),
    ];

    let scene = MeshletScene::new(&device, instances.len() as u32);
    scene.upload_instances(&queue, &instances);

    let count = instances.len() as u32;
    let threads = count * per_mesh;
    let mut cull = MeshletCull::new(&device, threads.max(1) * 2, DEFAULT_MAX_TRIANGLES as u32);
    cull.ensure_capacity(&device, threads);
    cull.ensure_group_capacity(&device, threads);
    cull.ensure_chunk_capacity(&device, chunks_for(count, per_mesh));

    let rig = Rig {
        pipelines: MeshletCullPipelines::new(&device),
        device,
        queue,
        pool: gpu_pool,
        scene,
        cull,
        instances: count,
        per_mesh,
    };

    assert_eq!(
        rig.survivors(false, 0.0),
        rig.survivors(true, 0.0),
        "the scaled instance's sphere did not follow its transform",
    );
}

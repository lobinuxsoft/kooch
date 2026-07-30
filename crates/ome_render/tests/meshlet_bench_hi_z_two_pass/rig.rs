use glam::{Mat4, Vec3};
use ome_core::Guid;
use ome_render::hi_z::HiZ;
use ome_render::material::{Material, MaterialPipeline};
use ome_render::mesh::Mesh;
use ome_render::meshlet::{
    CullParams, DEFAULT_MAX_TRIANGLES, DEFERRED_COLOR_FORMAT, GlobalMeshPool, MeshInstance,
    MeshletCull, MeshletDeferredShader, MeshletScene, MeshletVisRasterizer, SceneCullParams,
    VISIBILITY_BUFFER_FORMAT, build_default_meshlets, meshlet_bind_group_layout,
    pool_meshlet_bind_group,
};

use crate::common::{build_sphere_mesh, try_acquire_device};
use crate::{DEPTH_FORMAT, RT_SIZE};

pub(crate) struct BenchRig {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) cull: MeshletCull,
    pub(crate) vbuf_raster: MeshletVisRasterizer,
    pub(crate) deferred: MeshletDeferredShader,
    pub(crate) meshlet_bg: wgpu::BindGroup,
    pub(crate) material_bg: wgpu::BindGroup,
    pub(crate) vbuf_view: wgpu::TextureView,
    pub(crate) depth_view: wgpu::TextureView,
    pub(crate) depth_sample_view: wgpu::TextureView,
    pub(crate) color_view: wgpu::TextureView,
    pub(crate) gpu_pool: ome_render::meshlet::GpuGlobalMeshPool,
    pub(crate) scene: MeshletScene,
    pub(crate) hiz_prev: HiZ,
    pub(crate) hiz_curr: HiZ,
    pub(crate) cull_params: CullParams,
    pub(crate) scene_params: SceneCullParams,
    pub(crate) view_proj: Mat4,
}

pub(crate) fn build_rig() -> Option<BenchRig> {
    let (device, queue) = try_acquire_device()?;

    let mesh: Mesh = build_sphere_mesh(32, 32);
    let meshlet_mesh = build_default_meshlets(&mesh).expect("build meshlets");
    let mut pool = GlobalMeshPool::new();
    let handle = pool.register(&meshlet_mesh);
    let max_meshlets_per_mesh = pool.max_meshlets_per_mesh().max(1);
    let gpu_pool = pool.upload(&device);

    let scene = MeshletScene::new(&device, 4);
    let instance = MeshInstance::new(Mat4::IDENTITY, handle.mesh_id, 0);
    scene.upload_instances(&queue, &[instance]);

    let mut cull = MeshletCull::new(&device, 4096, DEFAULT_MAX_TRIANGLES as u32);
    cull.ensure_group_capacity(&device, pool.group_capacity.max(1));

    let vbuf_raster = MeshletVisRasterizer::new(
        &device,
        Some(DEPTH_FORMAT),
        cull.meshlet_bind_group_layout(),
        None,
    );
    let deferred = MeshletDeferredShader::new(&device, cull.meshlet_bind_group_layout());

    let meshlet_bgl = meshlet_bind_group_layout(&device);
    let meshlet_bg = pool_meshlet_bind_group(&device, &meshlet_bgl, &gpu_pool);

    let mut materials = MaterialPipeline::with_capacity(&device, &queue, 4);
    materials.register(
        &queue,
        Guid::new_v4(),
        &Material::new([0.8, 0.6, 0.4, 1.0], 0.0, 0.4, 0.0),
    );
    let material_bg = materials.pool().bind_group(&device);

    let vbuf_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_two_pass_vbuf"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VISIBILITY_BUFFER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let vbuf_view = vbuf_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_two_pass_color"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEFERRED_COLOR_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench_two_pass_depth"),
        size: wgpu::Extent3d {
            width: RT_SIZE,
            height: RT_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_sample_view = depth_tex.create_view(&wgpu::TextureViewDescriptor {
        label: Some("bench_two_pass_depth_sample"),
        format: Some(DEPTH_FORMAT),
        dimension: Some(wgpu::TextureViewDimension::D2),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(1),
    });

    let hiz_prev = HiZ::new(&device, RT_SIZE, RT_SIZE);
    let hiz_curr = HiZ::new(&device, RT_SIZE, RT_SIZE);
    // Seed hiz_prev to "far" via the same init path render_with_assets
    // uses: clear depth + run the pyramid build over the cleared
    // contents.
    {
        let mut init_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench_hi_z_init"),
        });
        {
            let _clear = init_enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bench_hi_z_init_depth_clear"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let mut init_arena: Vec<wgpu::BindGroup> = Vec::new();
        hiz_prev.init_to_far(&device, &mut init_enc, &depth_sample_view, &mut init_arena);
        queue.submit(std::iter::once(init_enc.finish()));
        // Keep init_arena alive across submit by leaking it for the
        // bench's lifetime. The bench drops at end-of-test anyway.
        std::mem::forget(init_arena);
    }

    let cam = Vec3::new(0.0, 0.0, 3.0);
    let view = Mat4::look_at_rh(cam, Vec3::ZERO, Vec3::Y);
    let proj = ome_render::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view_proj = proj * view;
    let cull_params = CullParams::new(view_proj, cam, max_meshlets_per_mesh);
    let scene_params = SceneCullParams::new(1, max_meshlets_per_mesh);

    Some(BenchRig {
        device,
        queue,
        cull,
        vbuf_raster,
        deferred,
        meshlet_bg,
        material_bg,
        vbuf_view,
        depth_view,
        depth_sample_view,
        color_view,
        gpu_pool,
        scene,
        hiz_prev,
        hiz_curr,
        cull_params,
        scene_params,
        view_proj,
    })
}

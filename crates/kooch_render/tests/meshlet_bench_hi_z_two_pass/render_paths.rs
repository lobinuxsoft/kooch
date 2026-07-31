use kooch_render::meshlet::HiZTestParams;

use crate::RT_SIZE;
use crate::rig::BenchRig;

pub(crate) fn render_single_pass(rig: &BenchRig) {
    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench_single_pass_frame"),
        });
    rig.cull.dispatch_scene_pool_atomic(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.gpu_pool,
        &rig.scene,
        &rig.cull_params,
        &rig.scene_params,
    );
    rig.vbuf_raster.render_scene(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.vbuf_view,
        &rig.depth_view,
        &rig.meshlet_bg,
        &rig.cull,
        &rig.scene,
        rig.view_proj,
        0,
        true,
    );
    rig.deferred.shade_scene(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.vbuf_view,
        &rig.color_view,
        &rig.meshlet_bg,
        &rig.material_bg,
        &rig.cull,
        &rig.scene,
        rig.view_proj,
        (RT_SIZE, RT_SIZE),
        0,
    );
    rig.queue.submit(std::iter::once(enc.finish()));
    let _ = rig.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
}

pub(crate) fn render_two_pass(rig: &mut BenchRig, arena: &mut Vec<wgpu::BindGroup>) {
    let (hiz_w, hiz_h) = rig.hiz_prev.dimensions();
    let mip_count = rig.hiz_prev.mip_count();
    let hi_z_params = HiZTestParams::new(rig.view_proj, hiz_w, hiz_h, mip_count);

    let mut enc = rig
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bench_two_pass_frame"),
        });
    rig.cull.dispatch_scene_pool_atomic_hi_z(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.gpu_pool,
        &rig.scene,
        &rig.cull_params,
        &rig.scene_params,
        &hi_z_params,
        rig.hiz_prev.full_view(),
        arena,
    );
    rig.vbuf_raster.render_scene(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.vbuf_view,
        &rig.depth_view,
        &rig.meshlet_bg,
        &rig.cull,
        &rig.scene,
        rig.view_proj,
        0,
        true,
    );
    rig.hiz_curr
        .build_from_depth(&rig.device, &mut enc, &rig.depth_sample_view, arena);
    rig.cull.dispatch_cull_pass_b(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.gpu_pool,
        &rig.scene,
        &hi_z_params,
        rig.hiz_curr.full_view(),
        arena,
    );
    rig.vbuf_raster.render_scene(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.vbuf_view,
        &rig.depth_view,
        &rig.meshlet_bg,
        &rig.cull,
        &rig.scene,
        rig.view_proj,
        0,
        false,
    );
    rig.deferred.shade_scene(
        &rig.device,
        &rig.queue,
        &mut enc,
        &rig.vbuf_view,
        &rig.color_view,
        &rig.meshlet_bg,
        &rig.material_bg,
        &rig.cull,
        &rig.scene,
        rig.view_proj,
        (RT_SIZE, RT_SIZE),
        0,
    );
    rig.queue.submit(std::iter::once(enc.finish()));
    let _ = rig.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    std::mem::swap(&mut rig.hiz_prev, &mut rig.hiz_curr);
}

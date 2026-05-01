//! Pass 0 (Morton encode) byte-identity vs the CPU `MortonCode`.

use super::helpers::*;
use crate::gpu::builder::test_device;
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::gpu::types::GpuSceneBounds;

#[test]
fn tlas_morton_byte_identical_to_cpu() {
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let (_descs, aabbs, chunk_descs_buf, mortons_buf, _sorted_indices_buf, n) =
        prepare_inputs(&device);

    let scene = GpuSceneBounds::from_aabbs(&aabbs);
    let builder = TlasGpuBuilder::new(&device, None);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_morton_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &chunk_descs_buf,
        &mortons_buf,
        scene,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let gpu_mortons = readback_u32(&device, &queue, &mortons_buf, n);
    let cpu = cpu_mortons(&scene, &aabbs);

    assert_eq!(gpu_mortons.len(), cpu.len());
    for (i, (g, c)) in gpu_mortons.iter().zip(cpu.iter()).enumerate() {
        assert_eq!(
            g, c,
            "GPU/CPU Morton diverge at chunk[{i}]: gpu={g:#010x} cpu={c:#010x}",
        );
    }
}

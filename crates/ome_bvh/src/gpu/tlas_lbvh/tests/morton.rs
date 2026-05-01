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
    let inputs = prepare_inputs(&device);

    let scene = GpuSceneBounds::from_aabbs(&inputs.aabbs);
    let builder = TlasGpuBuilder::new(&device, None);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_morton_test_encoder"),
    });
    builder.dispatch_morton(
        &device,
        &queue,
        &mut encoder,
        &inputs.chunk_descs_buf,
        &inputs.mortons_buf,
        &inputs.live_chunk_indices_buf,
        scene,
        inputs.n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let gpu_mortons = readback_u32(&device, &queue, &inputs.mortons_buf, inputs.n);
    let cpu = cpu_mortons(&scene, &inputs.aabbs);

    assert_eq!(gpu_mortons.len(), cpu.len());
    for (i, (g, c)) in gpu_mortons.iter().zip(cpu.iter()).enumerate() {
        assert_eq!(
            g, c,
            "GPU/CPU Morton diverge at chunk[{i}]: gpu={g:#010x} cpu={c:#010x}",
        );
    }
}

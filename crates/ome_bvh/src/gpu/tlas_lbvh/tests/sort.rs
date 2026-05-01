//! Pass 1 (onesweep sort) byte-identity vs CPU stable sort. Pins
//! AC6 determinism: same permutation across runs and devices.

use super::helpers::*;
use crate::gpu::builder::test_device;
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::gpu::types::GpuSceneBounds;

#[test]
fn tlas_sort_permutation_byte_identical_to_cpu() {
    // Pins determinism: the GPU onesweep and the CPU stdlib stable
    // sort must produce *the same* permutation when keys tie. AC6
    // of epic #370 (scene hash byte-identical across runs) depends
    // on this — any divergence here would surface as a non-deterministic
    // TLAS topology under churn.
    let Some((device, queue)) = test_device::try_acquire() else {
        eprintln!("ome_bvh::gpu::tlas_lbvh: no GPU adapter — skipping");
        return;
    };
    let (_descs, aabbs, chunk_descs_buf, mortons_buf, sorted_indices_buf, n) =
        prepare_inputs(&device);

    let scene = GpuSceneBounds::from_aabbs(&aabbs);
    let mut builder = TlasGpuBuilder::new(&device, None);
    builder.ensure_capacity(&device, n as u64);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tlas_sort_test_encoder"),
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
    builder.dispatch_sort(
        &device,
        &queue,
        &mut encoder,
        &mortons_buf,
        &sorted_indices_buf,
        n,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let gpu_sorted_keys = readback_u32(&device, &queue, &mortons_buf, n);
    let gpu_sorted_indices = readback_u32(&device, &queue, &sorted_indices_buf, n);

    // CPU reference: stable sort of (morton, original_idx) pairs by
    // morton ascending. Index ties broken by original index, which
    // is what `sort_by_key` does (stable sort preserves relative
    // order). The onesweep contract is also stable in the same way.
    let cpu_unsorted = cpu_mortons(&scene, &aabbs);
    let mut cpu_indexed: Vec<(u32, u32)> = cpu_unsorted
        .iter()
        .enumerate()
        .map(|(i, k)| (*k, i as u32))
        .collect();
    cpu_indexed.sort_by_key(|(k, _)| *k);
    let cpu_sorted_keys: Vec<u32> = cpu_indexed.iter().map(|(k, _)| *k).collect();
    let cpu_sorted_indices: Vec<u32> = cpu_indexed.iter().map(|(_, i)| *i).collect();

    assert_eq!(
        gpu_sorted_keys, cpu_sorted_keys,
        "GPU sorted keys must byte-match CPU stable sort",
    );
    assert_eq!(
        gpu_sorted_indices, cpu_sorted_indices,
        "GPU sorted permutation must byte-match CPU stable sort \
         (determinism is AC6 of epic #370)",
    );
}

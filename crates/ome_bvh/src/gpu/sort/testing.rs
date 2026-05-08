use crate::gpu::sort_types::global_histogram_size_bytes;

use super::buffers::SortBuffers;

/// Test-only readback of the global histogram. Returns
/// `[pass][bucket]` as a flat `Vec<u32>` of `4 * 256 = 1024` entries.
pub fn readback_histogram_for_test(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffers: &SortBuffers,
) -> Vec<u32> {
    let bytes = global_histogram_size_bytes();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_bvh::sort_histogram_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ome_bvh::sort_histogram_readback_encoder"),
    });
    encoder.copy_buffer_to_buffer(&buffers.global_histogram, 0, &staging, 0, bytes);
    queue.submit(std::iter::once(encoder.finish()));
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll failed");
    receiver
        .recv()
        .expect("map_async sender dropped")
        .expect("map_async failed");
    let data = slice.get_mapped_range();
    let v = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();
    v
}

#[cfg(test)]
mod tests {
    use super::readback_histogram_for_test;
    use crate::gpu::builder::test_device;
    use crate::gpu::sort::buffers::SortBuffers;
    use crate::gpu::sort::dispatch::{
        dispatch_exclusive_scan, dispatch_histogram, dispatch_init, dispatch_sort,
    };
    use crate::gpu::sort::pipelines::SortPipelines;
    use crate::gpu::sort_types::{RADIX_BITS, RADIX_BUCKETS, RADIX_PASSES};

    fn cpu_histogram(keys: &[u32]) -> Vec<u32> {
        let mut hist = vec![0u32; (RADIX_PASSES * RADIX_BUCKETS) as usize];
        for &k in keys {
            for p in 0..RADIX_PASSES {
                let digit = ((k >> (p * RADIX_BITS)) & 0xFF) as usize;
                hist[(p * RADIX_BUCKETS) as usize + digit] += 1;
            }
        }
        hist
    }

    fn cpu_exclusive_scan(hist: &[u32]) -> Vec<u32> {
        // Per-pass exclusive prefix sum.
        let mut out = hist.to_vec();
        for p in 0..RADIX_PASSES as usize {
            let base = p * RADIX_BUCKETS as usize;
            let slice = &mut out[base..base + RADIX_BUCKETS as usize];
            let mut acc = 0u32;
            for v in slice.iter_mut() {
                let n = *v;
                *v = acc;
                acc += n;
            }
        }
        out
    }

    fn upload_keys(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffers: &mut SortBuffers,
        keys: &[u32],
    ) -> u32 {
        let count = keys.len() as u32;
        let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
            / crate::gpu::sort_types::ITEMS_PER_TILE;
        buffers.ensure_capacity(device, count as u64, partitions);
        if !keys.is_empty() {
            queue.write_buffer(&buffers.keys_a, 0, bytemuck::cast_slice(keys));
        }
        partitions
    }

    #[test]
    fn histogram_matches_cpu_random_keys() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        // 5000 deterministic-pseudo-random u32 keys.
        let mut state = 0xfeedfeedu32;
        let keys: Vec<u32> = (0..5000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();

        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        let cpu = cpu_histogram(&keys);
        assert_eq!(gpu, cpu, "GPU histogram must match CPU reference");
    }

    #[test]
    fn histogram_total_is_count_per_pass() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let keys: Vec<u32> = (0..100u32).collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        // Each pass's 256 buckets must sum to `keys.len()`.
        for p in 0..RADIX_PASSES as usize {
            let base = p * RADIX_BUCKETS as usize;
            let total: u32 = gpu[base..base + RADIX_BUCKETS as usize].iter().sum();
            assert_eq!(total, keys.len() as u32, "pass {p} total mismatch");
        }
    }

    #[test]
    fn exclusive_scan_matches_cpu() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut state = 0xcafebabeu32;
        let keys: Vec<u32> = (0..2000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        for pass in 0..RADIX_PASSES {
            dispatch_exclusive_scan(&device, &mut encoder, &pipelines, &buffers, pass);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        let cpu_hist = cpu_histogram(&keys);
        let cpu_scan = cpu_exclusive_scan(&cpu_hist);
        assert_eq!(gpu, cpu_scan, "exclusive scan must match CPU reference");
    }

    /// Test-only readback of the keys array (after sort lands here).
    fn readback_keys(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        keys_buffer: &wgpu::Buffer,
        count: u32,
    ) -> Vec<u32> {
        let bytes = (count as u64) * 4;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_bvh::sort_keys_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_bvh::sort_keys_readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(keys_buffer, 0, &staging, 0, bytes);
        queue.submit(std::iter::once(encoder.finish()));
        let slice = staging.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            sender.send(res).ok();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("device poll failed");
        receiver
            .recv()
            .expect("map_async sender dropped")
            .expect("map_async failed");
        let data = slice.get_mapped_range();
        let v = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
        drop(data);
        staging.unmap();
        v
    }

    fn upload_keys_and_values(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffers: &mut SortBuffers,
        keys: &[u32],
    ) -> u32 {
        let count = keys.len() as u32;
        let partitions = (count + crate::gpu::sort_types::ITEMS_PER_TILE - 1)
            / crate::gpu::sort_types::ITEMS_PER_TILE;
        buffers.ensure_capacity(device, count as u64, partitions);
        if !keys.is_empty() {
            queue.write_buffer(&buffers.keys_a, 0, bytemuck::cast_slice(keys));
            // Values = original index 0..count.
            let values: Vec<u32> = (0..count).collect();
            queue.write_buffer(&buffers.values_a, 0, bytemuck::cast_slice(&values));
        }
        partitions
    }

    #[test]
    fn full_sort_matches_cpu_sort_small() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut keys: Vec<u32> = vec![3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5];
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);

        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys);
    }

    #[test]
    fn full_sort_matches_cpu_sort_random() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let mut state = 0xfeedfeedu32;
        let mut keys: Vec<u32> = (0..2000)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state
            })
            .collect();
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);
        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys, "GPU sort must match CPU sort byte-for-byte");
    }

    #[test]
    fn full_sort_handles_multi_partition() {
        // Larger than ITEMS_PER_TILE — exercises the decoupled-lookback
        // chained scan across partitions.
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        // 10 000 keys = 4 partitions (ITEMS_PER_TILE = 3072).
        let mut state = 0xcafebabeu32;
        let mut keys: Vec<u32> = (0..10_000)
            .map(|_| {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                state
            })
            .collect();
        let _ = upload_keys_and_values(&device, &queue, &mut buffers, &keys);
        let encoder = dispatch_sort(&device, &queue, &pipelines, &buffers, keys.len() as u32);
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_keys(&device, &queue, &buffers.keys_a, keys.len() as u32);
        keys.sort();
        assert_eq!(gpu, keys);
    }

    #[test]
    fn exclusive_scan_first_bucket_is_zero() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let pipelines = SortPipelines::new(&device, None);
        let mut buffers = SortBuffers::new(&device);

        let keys: Vec<u32> = (0..50u32).collect();
        let partitions = upload_keys(&device, &queue, &mut buffers, &keys);

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        dispatch_init(&device, &queue, &mut encoder, &pipelines, &buffers, partitions);
        dispatch_histogram(
            &device,
            &queue,
            &mut encoder,
            &pipelines,
            &buffers,
            &buffers.keys_a,
            keys.len() as u32,
        );
        for pass in 0..RADIX_PASSES {
            dispatch_exclusive_scan(&device, &mut encoder, &pipelines, &buffers, pass);
        }
        queue.submit(std::iter::once(encoder.finish()));

        let gpu = readback_histogram_for_test(&device, &queue, &buffers);
        for p in 0..RADIX_PASSES as usize {
            assert_eq!(
                gpu[p * RADIX_BUCKETS as usize],
                0,
                "pass {p} bucket 0 exclusive scan must be 0"
            );
        }
    }
}

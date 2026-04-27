//! GPU Morton encoding pass — public entry point + CPU/GPU consistency
//! tests.
//!
//! The actual dispatch lives on [`BvhGpuBuilder::dispatch_morton`]; this
//! module owns the integration test that proves CPU and GPU encodings
//! match byte-for-byte.

#[cfg(test)]
mod tests {
    use crate::aabb::Aabb;
    use crate::gpu::builder::{BvhGpuBuilder, test_device};
    use crate::morton::MortonCode;
    use glam::Vec3;

    fn cpu_codes(aabbs: &[Aabb]) -> Vec<u32> {
        // Replicate the GPU normalisation: scene min + 1/extent on
        // each axis; degenerate axes collapse to cell 0.
        let scene = aabbs.iter().fold(Aabb::EMPTY, |acc, a| acc.union(a));
        if aabbs.is_empty() || scene.is_empty() {
            return Vec::new();
        }
        let extent = scene.max - scene.min;
        let inv = Vec3::new(
            if extent.x > 0.0 { 1.0 / extent.x } else { 0.0 },
            if extent.y > 0.0 { 1.0 / extent.y } else { 0.0 },
            if extent.z > 0.0 { 1.0 / extent.z } else { 0.0 },
        );
        aabbs
            .iter()
            .map(|a| {
                let centre = a.center();
                let normalized = (centre - scene.min) * inv;
                MortonCode::from_normalized(normalized).0
            })
            .collect()
    }

    #[test]
    fn gpu_morton_matches_cpu_random_aabbs() {
        let Some((device, queue)) = test_device::try_acquire() else {
            eprintln!("ome_bvh::gpu::morton: no GPU adapter with TIMESTAMP_QUERY available — skipping");
            return;
        };
        let mut builder = BvhGpuBuilder::new(&device, &queue, None);

        // 100 deterministic-pseudo-random AABBs across a 10×10×10 box.
        let mut state: u32 = 0xdeadbeef;
        let mut rand = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) as f32 / 32768.0
        };
        let aabbs: Vec<Aabb> = (0..100)
            .map(|_| {
                let centre = Vec3::new(rand(), rand(), rand()) * 10.0;
                Aabb::from_centre(centre, Vec3::splat(0.2))
            })
            .collect();

        let encoder = builder.dispatch_morton(&device, &queue, &aabbs);
        queue.submit(std::iter::once(encoder.finish()));
        let gpu = builder.readback_morton_for_test(&device, &queue, aabbs.len() as u64);
        let cpu = cpu_codes(&aabbs);

        assert_eq!(gpu, cpu, "GPU and CPU Morton codes must match byte-for-byte");
    }

    #[test]
    fn gpu_morton_handles_degenerate_axes() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let mut builder = BvhGpuBuilder::new(&device, &queue, None);

        // All AABBs collapsed to y = z = 5; only x varies.
        let aabbs: Vec<Aabb> = (0..16)
            .map(|i| {
                Aabb::new(
                    Vec3::new(i as f32, 5.0, 5.0),
                    Vec3::new(i as f32 + 0.1, 5.0, 5.0),
                )
            })
            .collect();

        let encoder = builder.dispatch_morton(&device, &queue, &aabbs);
        queue.submit(std::iter::once(encoder.finish()));
        let gpu = builder.readback_morton_for_test(&device, &queue, aabbs.len() as u64);
        let cpu = cpu_codes(&aabbs);
        assert_eq!(gpu, cpu);
    }

    #[test]
    fn gpu_morton_single_item() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let mut builder = BvhGpuBuilder::new(&device, &queue, None);
        let aabbs = vec![Aabb::from_centre(Vec3::splat(5.0), Vec3::splat(0.5))];
        let encoder = builder.dispatch_morton(&device, &queue, &aabbs);
        queue.submit(std::iter::once(encoder.finish()));
        let gpu = builder.readback_morton_for_test(&device, &queue, 1);
        let cpu = cpu_codes(&aabbs);
        assert_eq!(gpu, cpu);
    }

    #[test]
    fn gpu_morton_grows_buffer_on_large_input() {
        let Some((device, queue)) = test_device::try_acquire() else {
            return;
        };
        let mut builder = BvhGpuBuilder::new(&device, &queue, None);
        // First small dispatch: caps capacity at 256 (the initial).
        let small: Vec<Aabb> = (0..64).map(|i| Aabb::from_centre(Vec3::splat(i as f32), Vec3::splat(0.5))).collect();
        let encoder = builder.dispatch_morton(&device, &queue, &small);
        queue.submit(std::iter::once(encoder.finish()));
        let _ = builder.readback_morton_for_test(&device, &queue, small.len() as u64);

        // Second large dispatch: exceeds 256 → grows.
        let large: Vec<Aabb> = (0..1500).map(|i| Aabb::from_centre(Vec3::splat(i as f32), Vec3::splat(0.5))).collect();
        let encoder = builder.dispatch_morton(&device, &queue, &large);
        queue.submit(std::iter::once(encoder.finish()));
        let gpu = builder.readback_morton_for_test(&device, &queue, large.len() as u64);
        let cpu = cpu_codes(&large);
        assert_eq!(gpu.len(), cpu.len());
        assert_eq!(gpu, cpu);
    }
}

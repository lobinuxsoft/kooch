use super::*;

fn skip_if_no_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ome_accel::tests"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()?;
    Some((device, queue))
}

#[test]
fn new_pre_allocates_six_buffers() {
    let Some((device, _queue)) = skip_if_no_device() else {
        return;
    };
    let accel = OmeAccel::new(&device, AccelCaps::TEST, 64).unwrap();
    assert_eq!(accel.live_chunk_count(), 0);
    assert_eq!(accel.tlas_dirty_count(), 0);
    // Free chunk stack covers every slot in low-first pop order.
    assert_eq!(accel.free_chunk_slots.len(), AccelCaps::TEST.max_chunks as usize);
    assert_eq!(*accel.free_chunk_slots.last().unwrap(), 0);
    assert_eq!(*accel.free_chunk_slots.first().unwrap(), AccelCaps::TEST.max_chunks - 1);
}

#[test]
fn new_rejects_excessive_max_chunks() {
    let Some((device, _queue)) = skip_if_no_device() else {
        return;
    };
    let mut caps = AccelCaps::TEST;
    caps.max_chunks = MAX_CHUNKS_LIMIT + 1;
    assert_eq!(
        OmeAccel::new(&device, caps, 64).err(),
        Some(AccelError::OutOfChunkSlots)
    );
}

#[test]
fn lookup_misses_when_empty() {
    let Some((device, _queue)) = skip_if_no_device() else {
        return;
    };
    let accel = OmeAccel::new(&device, AccelCaps::TEST, 64).unwrap();
    assert!(accel.lookup(0xDEADBEEF).is_none());
}

fn make_leaf_aabb(centre_x: f32) -> crate::leaf::LeafAabb {
    crate::leaf::LeafAabb {
        aabb_min: [centre_x - 0.5, -0.5, -0.5],
        flags: crate::leaf::IS_RAYMARCH,
        aabb_max: [centre_x + 0.5, 0.5, 0.5],
        entity_id: 0,
    }
}

#[test]
fn insert_chunk_round_trips_descriptor() {
    let Some((device, queue)) = skip_if_no_device() else {
        return;
    };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let leaves: Vec<_> = (0..4).map(|i| make_leaf_aabb(i as f32)).collect();
    let primitives_bytes = vec![0u8; 16 * 4];
    let handle = accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 0xCAFE,
                leaf_aabbs: &leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: 0.25,
            },
        )
        .unwrap();
    assert_eq!(accel.live_chunk_count(), 1);
    assert_eq!(accel.lookup(0xCAFE), Some(handle));
    let desc = accel.descriptor(handle).unwrap();
    // 4 leaves → 2*4 - 1 = 7 nodes total.
    assert_eq!(desc.leaf_count, 4);
    assert_eq!(desc.node_count, 7);
    assert_eq!(desc.primitive_count, 4);
    // AABB inflated by max_smoothness_radius.
    assert!(desc.aabb_min[0] <= -0.5 - 0.25 + 1e-6);
    assert!(desc.aabb_max[0] >= 3.5 + 0.25 - 1e-6);
    assert_eq!(desc.max_smoothness_radius, 0.25);
}

#[test]
fn remove_chunk_frees_slots() {
    let Some((device, queue)) = skip_if_no_device() else {
        return;
    };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let leaves: Vec<_> = (0..2).map(|i| make_leaf_aabb(i as f32)).collect();
    let primitives_bytes = vec![0u8; 16 * 2];
    accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves,
                primitives_bytes: &primitives_bytes,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    let used_slots_before = AccelCaps::TEST.max_chunks - accel.free_chunk_slots.len() as u32;
    assert_eq!(used_slots_before, 1);
    accel.remove_chunk(&queue, 1).unwrap();
    assert_eq!(accel.live_chunk_count(), 0);
    assert!(accel.lookup(1).is_none());
    // The slot was returned for reuse.
    assert_eq!(accel.free_chunk_slots.len() as u32, AccelCaps::TEST.max_chunks);
    // Trying to remove twice fails cleanly.
    assert_eq!(
        accel.remove_chunk(&queue, 1),
        Err(crate::accel::AccelError::UnknownChunk)
    );
}

#[test]
fn insert_two_chunks_distinct_offsets() {
    let Some((device, queue)) = skip_if_no_device() else {
        return;
    };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let leaves_a: Vec<_> = (0..3).map(|i| make_leaf_aabb(i as f32)).collect();
    let leaves_b: Vec<_> = (0..5).map(|i| make_leaf_aabb(i as f32 + 100.0)).collect();
    let prim_a = vec![0u8; 16 * 3];
    let prim_b = vec![0u8; 16 * 5];
    let h_a = accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves_a,
                primitives_bytes: &prim_a,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    let h_b = accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 2,
                leaf_aabbs: &leaves_b,
                primitives_bytes: &prim_b,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    assert_ne!(h_a.chunk_idx, h_b.chunk_idx);
    let da = accel.descriptor(h_a).unwrap();
    let db = accel.descriptor(h_b).unwrap();
    // Ranges must be disjoint.
    assert!(
        da.first_node + da.node_count <= db.first_node
            || db.first_node + db.node_count <= da.first_node
    );
    assert!(
        da.first_leaf + da.leaf_count <= db.first_leaf
            || db.first_leaf + db.leaf_count <= da.first_leaf
    );
    assert!(
        da.first_primitive + da.primitive_count <= db.first_primitive
            || db.first_primitive + db.primitive_count <= da.first_primitive
    );
}

#[test]
fn tlas_dirty_flips_via_update_gpu() {
    let Some((device, queue)) = skip_if_no_device() else {
        return;
    };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let leaves: Vec<_> = (0..2).map(|i| make_leaf_aabb(i as f32)).collect();
    let prim = vec![0u8; 16 * 2];
    accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 1,
                leaf_aabbs: &leaves,
                primitives_bytes: &prim,
                max_smoothness_radius: 0.0,
            },
        )
        .unwrap();
    assert!(accel.tlas_dirty_count() > 0);
    accel.update_gpu_standalone(&device, &queue, 0.1, 0.1);
    assert_eq!(accel.tlas_dirty_count(), 0);
}

#[test]
fn refit_chunk_preserves_handle() {
    let Some((device, queue)) = skip_if_no_device() else {
        return;
    };
    let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
    let leaves: Vec<_> = (0..3).map(|i| make_leaf_aabb(i as f32)).collect();
    let prim = vec![0u8; 16 * 3];
    let handle = accel
        .insert_chunk(
            &queue,
            crate::accel::ChunkInsert {
                key: 99,
                leaf_aabbs: &leaves,
                primitives_bytes: &prim,
                max_smoothness_radius: 0.5,
            },
        )
        .unwrap();
    // Move every primitive +10 in x.
    let leaves_moved: Vec<_> = (0..3)
        .map(|i| make_leaf_aabb(i as f32 + 10.0))
        .collect();
    accel
        .refit_chunk(
            &queue,
            crate::accel::ChunkRefit {
                key: 99,
                leaf_aabbs: &leaves_moved,
                primitives_bytes: &prim,
                max_smoothness_radius: 0.5,
            },
        )
        .unwrap();
    let desc = accel.descriptor(handle).unwrap();
    assert!(desc.aabb_min[0] >= 9.0);
    assert!(desc.aabb_max[0] >= 12.0);
}

//! The pool bind group is kept across frames: an edit that fits writes in place and leaves the
//! buffers where they are, so the group it was built over is still the right one (#1558).

mod common;

use common::{build_cube_mesh, try_acquire_device};
use kooch_render::meshlet::{GlobalMeshPool, PoolKeys, build_default_meshlets};

#[test]
fn an_edit_keeps_the_buffers() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no suitable GPU adapter; skipping");
        return;
    };
    let mesh = build_default_meshlets(&build_cube_mesh()).expect("meshlets");
    let mut pool = GlobalMeshPool::new();
    // Enough slack that the next edit cannot outgrow what the first upload reserved.
    for _ in 0..8 {
        pool.register(&mesh);
    }
    let mut gpu = pool.upload(&device, &queue);
    let before = PoolKeys::of(&gpu);

    pool.release(3);
    pool.register(&mesh);
    gpu.sync(&device, &queue, &mut pool);

    assert!(
        before == PoolKeys::of(&gpu),
        "an edit that fits must not replace a buffer, or every frame rebuilds the bind group",
    );
}

/// The other half: outgrowing a buffer *must* move it, so the cache cannot serve a freed one.
#[test]
fn growth_moves_the_buffers() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no suitable GPU adapter; skipping");
        return;
    };
    let mesh = build_default_meshlets(&build_cube_mesh()).expect("meshlets");
    let mut pool = GlobalMeshPool::new();
    pool.register(&mesh);
    let mut gpu = pool.upload(&device, &queue);
    let before = PoolKeys::of(&gpu);

    for _ in 0..64 {
        pool.register(&mesh);
    }
    gpu.sync(&device, &queue, &mut pool);

    assert!(
        before != PoolKeys::of(&gpu),
        "the pool outgrew its buffers, so they were reallocated and the keys must differ",
    );
}

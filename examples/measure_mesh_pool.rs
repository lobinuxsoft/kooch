//! What the meshlet pool costs, per asset.
//!
//! Answers one question before #592 picks a shape: if a second view meant
//! a second pool, how much VRAM would that be? The pool stores geometry
//! **per unique mesh** — `register_mesh` is idempotent by guid, so a
//! hundred instances of the same model are one entry — which means the
//! number that matters is the size of a scene's distinct assets, not its
//! object count.
//!
//! Reproduces the loader's own path exactly (`parse_mesh_bytes_full` then
//! `build_meshlets_lod_chain` with the same constants), so the figure is
//! what the engine actually uploads rather than an estimate of it.
//!
//! ```bash
//! cargo run --example measure_mesh_pool
//! ```

use std::path::Path;

use kooch_render::mesh::parse_mesh_bytes_full;
use kooch_render::meshlet::{
    DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, GlobalMeshPool, LodConfig, MeshletMesh,
    build_meshlets_lod_chain,
};

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn main() {
    let assets = [
        "assets/meshes/scattering_skull.glb",
        "assets/meshes/damaged_helmet.glb",
        "assets/meshes/suzanne.glb",
        "assets/meshes/primitives/cube.glb",
    ];

    let mut pool = GlobalMeshPool::default();
    let mut previous = 0u64;

    println!(
        "{:<44} {:>10} {:>12} {:>10} {:>12}",
        "asset", "file", "pool bytes", "meshlets", "pool total"
    );

    for path in assets {
        let Ok(bytes) = std::fs::read(path) else {
            println!("{path:<44} {:>10}", "missing");
            continue;
        };
        let file_size = bytes.len() as u64;

        let mesh = match parse_mesh_bytes_full(&bytes, 1.0, Path::new(path).parent()) {
            Ok(mesh) => mesh,
            Err(e) => {
                println!("{path:<44} parse failed: {e}");
                continue;
            }
        };
        let meshlets: MeshletMesh = match build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        ) {
            Ok(m) => m,
            Err(e) => {
                println!("{path:<44} build failed: {e}");
                continue;
            }
        };
        let meshlet_count = meshlets.meshlet_count();
        pool.register(&meshlets);

        let total = pool.byte_size();
        let delta = total - previous;
        previous = total;

        println!(
            "{:<44} {:>9.2}M {:>11.2}M {:>10} {:>11.2}M",
            path.rsplit('/').next().unwrap_or(path),
            mib(file_size),
            mib(delta),
            meshlet_count,
            mib(total),
        );
    }

    println!();
    println!(
        "pool total: {:.2} MiB for {} meshes",
        mib(pool.byte_size()),
        assets.len()
    );
    println!();
    println!("A second MeshletRenderStage duplicates exactly this figure —");
    println!("it does not grow with instance count, only with distinct assets.");
}

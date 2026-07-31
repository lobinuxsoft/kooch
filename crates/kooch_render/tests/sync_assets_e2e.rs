//! End-to-end test for `MeshletRenderStage::sync_assets_to_gpu`.
//!
//! Validates the full chain that PR3 wires: `MeshRenderer.mesh: Some(guid)`
//! → `AssetServer::load_by_guid::<MeshletMesh>` → `Assets<MeshletMesh>`
//! lookup → `ensure_gpu_mesh` upload + `MeshletPipeline.registry`
//! registration. Headless except for the wgpu device the upload step
//! needs — gated on `try_acquire_device`.

mod common;

use std::path::PathBuf;

use glam::Mat4;
use kooch_core::Guid;
use kooch_core::asset_database::{AssetDatabase, AssetEntry};
use kooch_core::asset_loader::AssetServer;
use kooch_core::asset_meta::{AssetMeta, write_meta};
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::registry::ComponentRegistry;
use kooch_ecs::hierarchy::global_transform::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::AccessTracker;
use kooch_render::mesh::GltfMeshLoader;
use kooch_render::mesh::Mesh;
use kooch_render::meshlet::{
    MeshletMesh, MeshletMeshLoader, MeshletRenderStage, MeshletRenderStageConfig,
};

use common::try_acquire_device;

fn ecs_resources() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r
}

/// Minimal triangle GLB — same fixture as `meshlet::loader::tests`,
/// duplicated here to keep the integration test self-contained.
fn build_minimal_triangle_glb() -> Vec<u8> {
    let indices: [u32; 3] = [0, 1, 2];
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
    let uvs: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

    let mut bin = Vec::new();
    bin.extend_from_slice(bytemuck::cast_slice(&indices));
    let positions_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&positions));
    let normals_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&normals));
    let uvs_offset = bin.len();
    bin.extend_from_slice(bytemuck::cast_slice(&uvs));
    let bin_len_unpadded = bin.len();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let bin_padded_len = bin.len();

    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": {bin_len_unpadded} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 12, "target": 34963 }},
    {{ "buffer": 0, "byteOffset": {positions_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {normals_offset}, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": {uvs_offset}, "byteLength": 24, "target": 34962 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5125, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0] }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3" }},
    {{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC2" }}
  ],
  "meshes": [
    {{ "primitives": [
      {{ "attributes": {{ "POSITION": 1, "NORMAL": 2, "TEXCOORD_0": 3 }}, "indices": 0 }}
    ] }}
  ]
}}"#
    );
    let mut json_padded = json.into_bytes();
    while json_padded.len() % 4 != 0 {
        json_padded.push(b' ');
    }

    let json_chunk_len = json_padded.len() as u32;
    let bin_chunk_len = bin_padded_len as u32;
    let total = 12 + 8 + json_chunk_len + 8 + bin_chunk_len;

    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());

    out.extend_from_slice(&json_chunk_len.to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_padded);

    out.extend_from_slice(&bin_chunk_len.to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);

    out
}

struct TempDir {
    path: PathBuf,
}
impl TempDir {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("kooch_sync_assets_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn sync_resolves_guid_to_gpu_mesh() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let dir = TempDir::new("resolve");
    let asset_path = dir.path.join("triangle.glb");
    std::fs::write(&asset_path, build_minimal_triangle_glb()).expect("write glb");

    let meta = AssetMeta::new();
    let guid = meta.guid;
    write_meta(&asset_path, &meta).expect("write meta");

    let mut server = AssetServer::new();
    server.register_loader::<Mesh, _>(GltfMeshLoader);
    server.register_loader::<MeshletMesh, _>(MeshletMeshLoader);

    let mut database = AssetDatabase::new();
    database.register(
        guid,
        AssetEntry {
            path: asset_path.clone(),
            mtime: std::time::SystemTime::now(),
            type_name: Some("kooch_render::meshlet::MeshletMesh".to_owned()),
        },
    );

    let mut resources = ecs_resources();
    resources.insert(server);
    resources.insert(database);
    resources.insert(Assets::<Mesh>::new());
    resources.insert(Assets::<MeshletMesh>::new());

    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(guid),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut resources);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            ..Default::default()
        },
    );
    assert_eq!(stage.gpu_mesh_count(), 0);

    stage.sync_assets_to_gpu(&device, &queue, &mut resources);

    assert_eq!(
        stage.gpu_mesh_count(),
        1,
        "one GUID should be registered in the pool",
    );
    assert!(
        stage.pipeline().lookup(guid).is_some(),
        "pool registry must hold the registered GUID",
    );

    // Second sync must be a no-op — already cached.
    stage.sync_assets_to_gpu(&device, &queue, &mut resources);
    assert_eq!(stage.gpu_mesh_count(), 1, "second sync must not duplicate");
}

#[test]
fn sync_skips_when_asset_database_lacks_guid() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let mut resources = ecs_resources();
    resources.insert(AssetServer::new());
    resources.insert(AssetDatabase::new());
    resources.insert(Assets::<MeshletMesh>::new());

    let stranger = Guid::new_v4();
    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(stranger),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut resources);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            ..Default::default()
        },
    );

    // GUID is dangling. Sync must log + skip without panicking.
    stage.sync_assets_to_gpu(&device, &queue, &mut resources);
    assert_eq!(stage.gpu_mesh_count(), 0);
}

#[test]
fn sync_without_asset_server_is_noop() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let mut resources = ecs_resources();
    // No AssetServer / AssetDatabase / Assets<MeshletMesh> inserted.

    let mut commands = Commands::new();
    commands
        .spawn(&mut resources)
        .insert(MeshRenderer {
            mesh: Some(Guid::new_v4()),
            visible: true,
            ..Default::default()
        })
        .insert(GlobalTransform {
            matrix: Mat4::IDENTITY,
        });
    commands.apply(&mut resources);

    let mut stage = MeshletRenderStage::new(
        &device,
        MeshletRenderStageConfig {
            size: (64, 64),
            instance_capacity: 4,
            ..Default::default()
        },
    );
    stage.sync_assets_to_gpu(&device, &queue, &mut resources);
    assert_eq!(stage.gpu_mesh_count(), 0);
}

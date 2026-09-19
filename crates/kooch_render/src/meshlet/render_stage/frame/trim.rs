//! The static masks' geometry step (#452): one pair a frame, baked, cut, and published as a mesh of
//! its own. What it publishes the asset sync uploads in the same call.

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;

use crate::material::MaterialPipeline;
use crate::meshlet::GeneratedMeshes;
use crate::meshlet::asset::MeshletMesh;

use super::super::MeshletRenderStage;

impl MeshletRenderStage {
    /// Cuts at most one masked pair into geometry: the first whose material has held still. One a
    /// frame, because each one bakes its cut and reads it back.
    pub fn trim_static_masks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut Resources,
    ) {
        let pairs = self.pipeline.collect_trim_pairs(resources);
        let Some((mesh, material, slot, stamp)) = self.pipeline.trim.next(&pairs) else {
            return;
        };
        // A mesh the asset step cannot reach yet is not a refusal: generated meshes never arrive,
        // but a file being loaded does, and remembering now would refuse it for the session.
        let Some(source) = source(resources, mesh) else {
            return;
        };
        let Some(materials) = resources.get::<MaterialPipeline>() else {
            return;
        };
        let cut = crate::meshlet::trim::build(device, queue, materials, slot, &source);
        let cut = match cut {
            Ok(cut) => cut,
            Err(why) => {
                tracing::info!(
                    target: "kooch_render::meshlet::trim",
                    mesh = %mesh,
                    material = %material,
                    reason = ?why,
                    "not cut into geometry: the pair keeps its masked raster",
                );
                self.pipeline.trim.remember(mesh, material, stamp, None);
                return;
            }
        };
        tracing::info!(
            target: "kooch_render::meshlet::trim",
            mesh = %mesh,
            material = %material,
            before = source.total_triangle_count(),
            after = cut.total_triangle_count(),
            "masked mesh cut to its alpha; it draws opaque from here",
        );
        let guid = Guid::new_v4();
        match resources.get_mut::<GeneratedMeshes>() {
            Some(generated) => generated.insert(guid, cut),
            None => {
                let mut generated = GeneratedMeshes::new();
                generated.insert(guid, cut);
                resources.insert(generated);
            }
        }
        self.pipeline
            .trim
            .remember(mesh, material, stamp, Some(guid));
    }
}

/// The mesh to cut, as the asset holds it. Cloned: the cut reads it while the trim writes into the
/// same `Resources`.
fn source(resources: &mut Resources, guid: Guid) -> Option<MeshletMesh> {
    let mut server = resources.remove::<AssetServer>()?;
    let handle = server.load_by_guid::<MeshletMesh>(guid, resources);
    resources.insert(server);
    let assets = resources.get::<Assets<MeshletMesh>>()?;
    assets.get(handle.ok()?).cloned()
}

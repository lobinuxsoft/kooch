//! What the forward pass draws this frame, and in what order (#452).

use glam::Vec3;

use crate::meshlet::MeshDescriptor;
use crate::meshlet::asset::MeshletDescriptor;
use crate::meshlet::scene::{MeshInstance, encode_scene_visible_id};

/// A stretch of the list drawn with one material's pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub material: u32,
    pub range: std::ops::Range<u32>,
}

/// The packed `(instance, meshlet)` entries the forward frame reads, far to near, and the runs that
/// share a pipeline. Kept between frames so the vectors are reused.
#[derive(Default, Clone)]
pub struct ForwardList {
    pub entries: Vec<u32>,
    pub runs: Vec<Run>,
}

impl ForwardList {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Rebuilds it from the scene's instances; the transparent ones are those from `first` on.
    ///
    /// 🔴 Sorted by the distance to each instance's bounding centre, farthest first: blending is
    /// done in submission order, so this order is the only thing that makes glass over glass
    /// right. Within an instance the order is the mesh's own.
    pub(crate) fn rebuild(
        &mut self,
        instances: &[MeshInstance],
        first: usize,
        bounds: &[crate::shadow::InstanceBounds],
        camera: Vec3,
        meshes: &[MeshDescriptor],
        meshlets: &[MeshletDescriptor],
    ) {
        self.entries.clear();
        self.runs.clear();
        let mut order: Vec<(f32, usize)> = (first..instances.len())
            .map(|i| {
                let centre = bounds.get(i).map_or(Vec3::ZERO, |b| b.center);
                (centre.distance_squared(camera), i)
            })
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));

        for (_, index) in order {
            let instance = &instances[index];
            let Some(mesh) = meshes.get(instance.mesh_id as usize) else {
                continue;
            };
            let start = self.entries.len() as u32;
            let first_meshlet = mesh.first_meshlet;
            for id in first_meshlet..first_meshlet + mesh.meshlet_count {
                // The finest level alone: a transparent surface drawn from two levels at once
                // would blend the same area twice.
                if meshlets.get(id as usize).is_some_and(|m| m.lod_level == 0) {
                    self.entries.push(encode_scene_visible_id(index as u32, id));
                }
            }
            let end = self.entries.len() as u32;
            if start == end {
                continue;
            }
            match self.runs.last_mut() {
                Some(run) if run.material == instance.material_id && run.range.end == start => {
                    run.range.end = end;
                }
                _ => self.runs.push(Run {
                    material: instance.material_id,
                    range: start..end,
                }),
            }
        }
    }
}

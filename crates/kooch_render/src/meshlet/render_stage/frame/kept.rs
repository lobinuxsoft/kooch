//! The [`Submitted`] rows into the instance slots, by their changes alone (#2014): a planet block
//! that did not change costs nothing on its frame.

use kooch_core::Guid;
use kooch_core::resource::Resources;

use crate::material::MaterialPipeline;
use crate::meshlet::{DrawKey, Submitted};

use super::super::MeshletRenderStage;

/// Where the stage is in the [`Submitted`] log, and what the rows it kept were resolved against.
#[derive(Default)]
pub(crate) struct Kept {
    seen: Option<u64>,
    generation: u32,
    /// Each material's slot and whether it blends, as the kept rows were built with.
    materials: Vec<(Guid, u32, bool)>,
    /// Rows whose mesh is not in the pool yet, tried again every frame.
    waiting: Vec<DrawKey>,
}

impl MeshletRenderStage {
    pub(super) fn keep_submitted(&mut self, resources: &Resources) {
        profiling::scope!("keep submitted");
        self.slots.settle();
        let Some(submitted) = resources.get::<Submitted>() else {
            if self.kept.seen.take().is_some() {
                self.slots.unkeep_all();
            }
            return;
        };
        let materials = resources.get::<MaterialPipeline>();
        let resolved = resolved(submitted, materials);
        let generation = self.pipeline.generation();
        let since = self.kept.seen.and_then(|seen| submitted.since(seen));
        let changed: Vec<DrawKey> = match since {
            // A repacked pool or a material that resolves anew renames what every row was built
            // with, so all of them are built again.
            Some(keys) if generation == self.kept.generation && resolved == self.kept.materials => {
                keys.chain(std::mem::take(&mut self.kept.waiting)).collect()
            }
            _ => {
                self.slots.unkeep_all();
                self.kept.waiting.clear();
                submitted.keys().to_vec()
            }
        };
        self.kept.seen = Some(submitted.changes());
        self.kept.generation = generation;
        self.kept.materials = resolved;
        let descriptors = &self.pipeline.pool().mesh_descriptors;
        for key in changed {
            let Some((matrix, renderer)) = submitted.get(key) else {
                self.slots.unkeep(key);
                continue;
            };
            match self.pipeline.instance(key, renderer, matrix, materials) {
                Some(instance) => {
                    let groups = descriptors
                        .get(instance.mesh_id as usize)
                        .map_or(0, |d| d.group_count);
                    self.slots.keep(instance, key, groups);
                }
                None => {
                    self.slots.unkeep(key);
                    if renderer.visible && renderer.mesh.is_some() {
                        self.kept.waiting.push(key);
                    }
                }
            }
        }
    }
}

fn resolved(submitted: &Submitted, materials: Option<&MaterialPipeline>) -> Vec<(Guid, u32, bool)> {
    let mut out: Vec<(Guid, u32, bool)> = submitted
        .materials()
        .map(|guid| {
            let slot = materials.map_or(crate::material::FALLBACK_MATERIAL_ID, |m| {
                m.lookup_or_fallback(Some(guid))
            });
            let blends = materials
                .and_then(|m| m.slot_surface(slot))
                .is_some_and(|(_, s)| s.kind.blends());
            (guid, slot, blends)
        })
        .collect();
    out.sort_unstable_by_key(|&(guid, ..)| guid.as_uuid());
    out
}

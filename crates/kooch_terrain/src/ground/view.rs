//! What the editor's ground overlay draws (#1949): every block's state and pose each frame, and a
//! block's triangles only the first time it is asked for them.

use std::sync::Arc;

use glam::{Quat, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::WorldOrigin;

use super::sync::{GroundColliders, GroundWire, Place};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockState {
    /// In its planet's patch in the solver.
    Built,
    /// Generated, and holds no surface.
    Empty,
    /// On the GPU or a worker, or meshed and waiting for its patch.
    Pending,
}

/// A block's state and where it is, in the solver's space.
pub struct BlockView {
    /// The build's id for a held block, 0 for one not meshed yet.
    pub id: u64,
    pub state: BlockState,
    /// The block's minimum corner.
    pub position: Vec3,
    pub rotation: Quat,
    /// Its edges, the planet's scale applied.
    pub size: Vec3,
}

pub struct GroundView {
    pub blocks: Vec<BlockView>,
    /// The triangles of the merged blocks whose id is past the caller's `since`.
    pub meshes: Vec<(u64, Arc<GroundWire>)>,
}

impl GroundColliders {
    /// Every block, and the triangles built after `since`.
    pub fn view(&self, resources: &Resources, since: u64) -> GroundView {
        let origin = WorldOrigin::get(resources);
        let place_of = |place: &Place| {
            let position = place.transform.position - origin.from(Some(&place.cell));
            (position.as_vec3(), place.transform.rotation)
        };
        let size = |place: &Place| place.transform.scale * place.side;
        let mut view = GroundView {
            blocks: Vec::with_capacity(self.held.len() + self.pending.len()),
            meshes: Vec::new(),
        };
        for held in self.held.values() {
            let (position, rotation) = place_of(&held.place);
            view.blocks.push(BlockView {
                id: held.id,
                state: match (&held.wire, held.merged) {
                    (None, _) => BlockState::Empty,
                    (Some(_), true) => BlockState::Built,
                    (Some(_), false) => BlockState::Pending,
                },
                position,
                rotation,
                size: size(&held.place),
            });
            if held.id > since
                && held.merged
                && let Some(wire) = &held.wire
            {
                view.meshes.push((held.id, Arc::clone(wire)));
            }
        }
        for place in self.pending.values() {
            let (position, rotation) = place_of(place);
            view.blocks.push(BlockView {
                id: 0,
                state: BlockState::Pending,
                position,
                rotation,
                size: size(place),
            });
        }
        view
    }
}

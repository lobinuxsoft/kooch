//! Renderers with no entity of their own: a producer that draws many meshes for one entity — a
//! planet's blocks (#1991) — hands them here instead of spawning an entity per mesh.

use glam::Mat4;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;

/// What a drawn mesh is from frame to frame: its entity, and which of the entity's meshes. An
/// entity's own renderer is part 0; a producer names its parts, which must stay put while they draw,
/// or last frame's matrix is looked up for the wrong mesh (#481).
pub type DrawKey = (Entity, u32);

/// This frame's entity-less renderers, as parallel arrays. Rebuilt by its producer every frame,
/// with matrices relative to this frame's world origin.
#[derive(Default)]
pub struct Submitted {
    /// The entity each one draws for: what picking and the instance list name.
    pub entity: Vec<Entity>,
    /// Which of the entity's meshes; see [`DrawKey`].
    pub part: Vec<u32>,
    pub matrix: Vec<Mat4>,
    pub renderer: Vec<MeshRenderer>,
}

impl Submitted {
    pub fn clear(&mut self) {
        self.entity.clear();
        self.part.clear();
        self.matrix.clear();
        self.renderer.clear();
    }

    pub fn push(&mut self, (entity, part): DrawKey, matrix: Mat4, renderer: MeshRenderer) {
        self.entity.push(entity);
        self.part.push(part);
        self.matrix.push(matrix);
        self.renderer.push(renderer);
    }
}

/// Every renderer the frame draws from: each entity's, then the [`Submitted`] ones. 🔴 Every walk
/// that asks what is drawn goes through here, or a submitted mesh or its material is evicted while
/// on screen.
pub fn for_each_drawn(resources: &Resources, mut each: impl FnMut(DrawKey, &MeshRenderer, &Mat4)) {
    Query::<(&MeshRenderer, &GlobalTransform)>::new(resources).for_each_entity(
        |entity, (renderer, transform)| each((entity, 0), renderer, &transform.matrix),
    );
    let Some(submitted) = resources.get::<Submitted>() else {
        return;
    };
    let keys = submitted.entity.iter().zip(&submitted.part);
    let rows = keys.zip(&submitted.matrix).zip(&submitted.renderer);
    for (((entity, part), matrix), renderer) in rows {
        each((*entity, *part), renderer, matrix);
    }
}

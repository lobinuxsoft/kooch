//! Renderers with no entity of their own: a producer that draws many meshes for one entity — a
//! planet's blocks (#1991) — hands them here instead of spawning an entity per mesh.

use glam::Mat4;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;

/// This frame's entity-less renderers, as parallel arrays. Rebuilt by its producer every frame,
/// with matrices relative to this frame's world origin.
#[derive(Default)]
pub struct Submitted {
    /// The entity each one draws for: what picking and the instance list name.
    pub entity: Vec<Entity>,
    pub matrix: Vec<Mat4>,
    pub renderer: Vec<MeshRenderer>,
}

impl Submitted {
    pub fn clear(&mut self) {
        self.entity.clear();
        self.matrix.clear();
        self.renderer.clear();
    }

    pub fn push(&mut self, entity: Entity, matrix: Mat4, renderer: MeshRenderer) {
        self.entity.push(entity);
        self.matrix.push(matrix);
        self.renderer.push(renderer);
    }
}

/// Every renderer the frame draws from: each entity's, then the [`Submitted`] ones. 🔴 Every walk
/// that asks what is drawn goes through here, or a submitted mesh or its material is evicted while
/// on screen.
pub fn for_each_drawn(resources: &Resources, mut each: impl FnMut(Entity, &MeshRenderer, &Mat4)) {
    Query::<(&MeshRenderer, &GlobalTransform)>::new(resources)
        .for_each_entity(|entity, (renderer, transform)| each(entity, renderer, &transform.matrix));
    let Some(submitted) = resources.get::<Submitted>() else {
        return;
    };
    let rows = submitted.entity.iter().zip(&submitted.matrix);
    for ((entity, matrix), renderer) in rows.zip(&submitted.renderer) {
        each(*entity, renderer, matrix);
    }
}

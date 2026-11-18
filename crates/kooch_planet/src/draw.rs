//! The entities that draw the blocks: one per pool slot, spawned the first time the slot holds a
//! surface and reused after, so the entity count is bounded by the pool, never by the planets.

use glam::{DVec3, Mat4};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Commands, Entity, MeshRenderer, Transform};
use kooch_render::mesh::Mesh;
use kooch_render::meshlet::{GeneratedMeshes, build_default_meshlets};

use crate::component::PlanetBlock;
use crate::sync::Found;
use crate::{MeshedBlock, Residency};

/// Per slot, as parallel arrays.
#[derive(Default)]
pub(crate) struct Blocks {
    entity: Vec<Option<Entity>>,
    triangles: Vec<u32>,
    /// The block's corner in its planet's space, and the roster index of that planet.
    origin: Vec<DVec3>,
    planet: Vec<u32>,
    /// Per roster index: the matrix and material its blocks were last drawn with.
    world: Vec<Mat4>,
    material: Vec<Option<Guid>>,
}

impl Blocks {
    /// Hides every slot that was freed or changed hands: it shows its old block until the new
    /// one's mesh comes back.
    pub(crate) fn hide(&mut self, resources: &mut Resources, residency: &Residency) {
        for &slot in residency.removed.iter().chain(&residency.added) {
            let s = slot as usize;
            if let Some(triangles) = self.triangles.get_mut(s) {
                *triangles = 0;
            }
            let Some(entity) = self.entity.get(s).copied().flatten() else {
                continue;
            };
            if let Some(renderer) = Query::<&mut MeshRenderer>::new(resources).get(entity) {
                renderer.visible = false;
            }
        }
    }

    /// Publishes each mesh that came back and points its slot's entity at it.
    pub(crate) fn draw(
        &mut self,
        resources: &mut Resources,
        placed: &[Option<usize>],
        found: &[Found],
        meshed: &mut Vec<MeshedBlock>,
    ) {
        for mut block in meshed.drain(..) {
            let Some(planet) = placed
                .get(block.key.planet as usize)
                .copied()
                .flatten()
                .and_then(|at| found.get(at))
            else {
                continue;
            };
            let s = block.slot as usize;
            if self.entity.len() <= s {
                self.entity.resize(s + 1, None);
                self.triangles.resize(s + 1, 0);
                self.origin.resize(s + 1, DVec3::ZERO);
                self.planet.resize(s + 1, 0);
            }
            let triangles = (block.indices.len() / 3) as u32;
            let mesh = Guid::from_bytes(slot_guid(block.slot));
            let visible = triangles > 0 && publish(resources, mesh, &mut block);
            self.triangles[s] = if visible { triangles } else { 0 };
            self.origin[s] = block.origin;
            self.planet[s] = block.key.planet;
            let transform = place(planet.world, block.origin);
            let renderer = MeshRenderer {
                mesh: Some(mesh),
                material: planet.planet.material,
                visible,
                ..Default::default()
            };
            self.entity[s] = put(resources, self.entity[s], transform, renderer);
        }
    }

    /// Re-places the blocks of every planet that moved, turned or changed material since its blocks
    /// were drawn.
    pub(crate) fn follow(
        &mut self,
        resources: &mut Resources,
        placed: &[Option<usize>],
        found: &[Found],
    ) {
        if self.world.len() < placed.len() {
            self.world.resize(placed.len(), Mat4::NAN);
            self.material.resize(placed.len(), None);
        }
        for (index, at) in placed.iter().enumerate() {
            let Some(planet) = at.and_then(|at| found.get(at)) else {
                continue;
            };
            let material = planet.planet.material;
            if self.world[index] == planet.world && self.material[index] == material {
                continue;
            }
            self.world[index] = planet.world;
            self.material[index] = material;
            for s in 0..self.entity.len() {
                let Some(entity) = self.entity[s].filter(|_| self.planet[s] == index as u32) else {
                    continue;
                };
                let query = Query::<(&mut Transform, &mut MeshRenderer)>::new(resources);
                if let Some((transform, renderer)) = query.get(entity) {
                    *transform = place(planet.world, self.origin[s]);
                    renderer.material = material;
                }
            }
        }
    }

    pub(crate) fn triangles(&self) -> u64 {
        self.triangles.iter().map(|&t| u64::from(t)).sum()
    }
}

/// Builds the block's meshlets and hands them to the render. Whether there is anything to draw.
fn publish(resources: &mut Resources, mesh: Guid, block: &mut MeshedBlock) -> bool {
    let slot = block.slot;
    let built = build_default_meshlets(&Mesh::with_normals(
        &block.positions,
        &block.normals,
        std::mem::take(&mut block.indices),
    ));
    match (built, resources.get_mut::<GeneratedMeshes>()) {
        (Ok(meshlets), Some(generated)) => {
            generated.insert(mesh, meshlets);
            true
        }
        (Err(why), _) => {
            tracing::warn!(target: "kooch_planet", slot, "a block gave no meshlets: {why}");
            false
        }
        (_, None) => false,
    }
}

/// Updates the slot's entity, or spawns one when it has none or it was despawned under us.
fn put(
    resources: &mut Resources,
    held: Option<Entity>,
    transform: Transform,
    renderer: MeshRenderer,
) -> Option<Entity> {
    if let Some(entity) = held
        && let Some((t, r)) =
            Query::<(&mut Transform, &mut MeshRenderer)>::new(resources).get(entity)
    {
        *t = transform;
        *r = renderer;
        return Some(entity);
    }
    let mut commands = resources.remove::<Commands>()?;
    let entity = commands
        .spawn(resources)
        .insert(PlanetBlock)
        .insert(transform)
        .insert(renderer)
        .id();
    resources.insert(commands);
    Some(entity)
}

/// A block's transform: its corner carried by the planet's.
fn place(world: Mat4, origin: DVec3) -> Transform {
    let (scale, rotation, _) = world.to_scale_rotation_translation();
    Transform::new(world.transform_point3(origin.as_vec3()), rotation, scale)
}

/// A mesh id of the slot's own, in a namespace no asset's GUID uses.
fn slot_guid(slot: u32) -> [u8; 16] {
    let mut bytes = *b"kooch:planet\0\0\0\0";
    bytes[12..].copy_from_slice(&slot.to_le_bytes());
    bytes
}

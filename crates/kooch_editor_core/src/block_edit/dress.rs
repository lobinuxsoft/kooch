//! The material a generated mesh starts with (#1534).
//!
//! 🔴 **One place decides it.** A block spawned from the toolbar, a block a curve fills in and a
//! mesh bent along a curve are three ways into the same state: an entity with a `MeshRenderer` and
//! geometry nobody picked a material for. Two of those paths used to set the prototype grid
//! themselves and the third did not, which is the drift `block_components()` exists to prevent —
//! so none of them sets it now and this does.
//!
//! 🔴 **Only when the renderer has none.** Written on every rebuild it would erase the material
//! the author chose, one frame after they chose it.

use std::collections::HashSet;

use kooch_blockmesh::{Block, SplineDeform};
use kooch_core::resource::Resources;
use kooch_ecs::component::{ComponentId, ComponentRegistry};
use kooch_ecs::entity::Entity;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;
use kooch_ecs::reflect::ReflectValue;

use crate::actions::EditorAction;

/// The engine's prototype grid. Its UVs are one repeat per world unit, so a wall pulled two metres
/// and one pulled four read as different sizes — on flat white they are the same picture.
const PROTOTYPE: &str = "materials/prototype/orange/orange_texture_01.material";

/// Entities already offered a material.
///
/// 🔴 Offered, not given. An author who clears the material back to none means it, and a system
/// that re-offers every frame would be arguing with them. It also keeps one `SetField` from going
/// down the wire on each frame of the round trip it is waiting for.
#[derive(Default)]
pub(crate) struct Dressed {
    offered: HashSet<Entity>,
}

/// Gives the prototype grid to every generated mesh that has no material yet.
pub(crate) fn dress_blocks(resources: &mut Resources, actions: &mut Vec<EditorAction>) {
    let mut dressed = resources.remove::<Dressed>().unwrap_or_default();
    let bare = bare_renderers(resources, &dressed);
    // Resolved once per frame, and only when there is something to dress: it walks the asset
    // database, which is not free on a project with thousands of files.
    if !bare.is_empty()
        && let Some(material) = prototype_material(resources)
        && let Some(component) = renderer_id(resources)
    {
        for entity in bare {
            dressed.offered.insert(entity);
            actions.push(EditorAction::SetField {
                entity,
                component,
                field: "material".to_owned(),
                value: ReflectValue::AssetRef {
                    guid: Some(material),
                    asset_type: std::any::type_name::<kooch_render::material::Material>()
                        .to_owned(),
                },
            });
            // Written here too, like a sweep's geometry is: the project owns the world, but until
            // its answer comes back the author is looking at this one (#1517).
            set_material(resources, entity, material);
        }
    }
    resources.insert(dressed);
}

/// Every generated mesh whose renderer has no material and has not been offered one.
fn bare_renderers(resources: &Resources, dressed: &Dressed) -> Vec<Entity> {
    // A set, because a deform on a block is found by both queries and would be dressed twice.
    let mut found = HashSet::new();
    let mut bare = |entity: Entity, renderer: &MeshRenderer| {
        if renderer.material.is_none() && !dressed.offered.contains(&entity) {
            found.insert(entity);
        }
    };
    Query::<(&MeshRenderer, &Block)>::new(resources)
        .for_each_entity(|entity, (renderer, _)| bare(entity, renderer));
    Query::<(&MeshRenderer, &SplineDeform)>::new(resources)
        .for_each_entity(|entity, (renderer, _)| bare(entity, renderer));
    found.into_iter().collect()
}

/// The guid of the prototype material, or `None` where there is no asset database to ask.
pub(crate) fn prototype_material(resources: &Resources) -> Option<kooch_core::Guid> {
    let resolved = resources
        .get::<kooch_core::asset_loader::AssetServer>()?
        .resolve_path(std::path::Path::new(PROTOTYPE));
    resources
        .get::<kooch_core::asset_database::AssetDatabase>()?
        .guid_for(&resolved)
}

fn renderer_id(resources: &Resources) -> Option<ComponentId> {
    resources
        .get::<kooch_ecs::component::ComponentNames>()?
        .id(std::any::type_name::<MeshRenderer>())
}

fn set_material(resources: &mut Resources, entity: Entity, material: kooch_core::Guid) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(mut storage) = registry.get_cpu_mut::<MeshRenderer>()
        && let Some(renderer) = storage.get_mut(entity)
    {
        renderer.material = Some(material);
    }
}

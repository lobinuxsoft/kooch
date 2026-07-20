//! Executes a [`Request`] against the live ECS on the main thread.
//!
//! Every handler runs where the main loop runs — the [`server`] thread
//! only ferries bytes — so mutation is single-threaded and needs no
//! locking. Component identity arrives as a name and is resolved to a
//! local `TypeId` here; a name this binary has no type for is a
//! [`RemoteError::UnknownComponent`], never a panic.
//!
//! [`server`]: crate::server

use std::any::TypeId;

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::Parent;
use ome_ecs::name::Name;
use ome_ecs::scene::{SceneDocument, sync_scene_to_ecs};

use crate::protocol::{
    ComponentSchema, ComponentSnapshot, EntityId, EntitySnapshot, FieldSchema, Method, RemoteError,
    Request, Response, ResponseData,
};

/// Runs `request` against `resources` and returns the response to send.
pub fn handle(request: &Request, resources: &mut Resources) -> Response {
    let id = request.id;
    match &request.method {
        Method::Ping => Response::ok(id, ResponseData::Pong),
        Method::ListEntities => list_entities(id, resources),
        Method::GetSchema => get_schema(id, resources),
        Method::SetField {
            entity,
            component,
            field,
            value,
        } => match set_field(resources, *entity, component, field, value.clone()) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::AddComponent { entity, component } => {
            match add_component(resources, *entity, component) {
                Ok(()) => Response::ok(id, ResponseData::Ok),
                Err(e) => Response::err(id, e),
            }
        }
        Method::RemoveComponent { entity, component } => {
            match remove_component(resources, *entity, component) {
                Ok(()) => Response::ok(id, ResponseData::Ok),
                Err(e) => Response::err(id, e),
            }
        }
        Method::Spawn { name } => {
            let entity = spawn(resources, name.as_deref());
            Response::ok(
                id,
                ResponseData::Spawned {
                    entity: entity.into(),
                },
            )
        }
        Method::Despawn { entity } => match despawn(resources, *entity) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::SaveScene { path } => {
            match SceneDocument::from_ecs(resources).save(path.as_ref()) {
                Ok(()) => Response::ok(id, ResponseData::Ok),
                Err(e) => Response::err(
                    id,
                    RemoteError::SceneError {
                        detail: e.to_string(),
                    },
                ),
            }
        }
        Method::LoadScene { path } => match load_scene(resources, path) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
    }
}

/// Snapshots every non-hierarchy component on every alive entity by name.
///
/// Built from [`SceneDocument::from_ecs`] so it captures exactly what a
/// save would, then annotated with live [`EntityId`]s the client needs
/// to address entities — the scene format keys parents by name, but a
/// remote client needs stable handles.
fn list_entities(id: u64, resources: &Resources) -> Response {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Response::err(
            id,
            RemoteError::Unavailable {
                detail: "no ComponentRegistry".into(),
            },
        );
    };
    let Some(archetypes) = resources.get::<ArchetypeRegistry>() else {
        return Response::err(
            id,
            RemoteError::Unavailable {
                detail: "no ArchetypeRegistry".into(),
            },
        );
    };

    let skip = [
        TypeId::of::<Parent>(),
        TypeId::of::<ome_ecs::hierarchy::Children>(),
        TypeId::of::<ome_ecs::hierarchy::GlobalTransform>(),
    ];
    let parents = registry.get_cpu::<Parent>();

    let mut entities = Vec::new();
    for archetype in archetypes.iter_matching(&[]) {
        for &entity in archetype.entities() {
            let mut components = Vec::new();
            for &type_id in archetype.components() {
                if skip.contains(&type_id) || !registry.has_reflector(&type_id) {
                    continue;
                }
                let Some(type_name) = registry.component_name(&type_id) else {
                    continue;
                };
                let Some(fields) = registry.reflect_get_fields(&type_id, entity) else {
                    continue;
                };
                components.push(ComponentSnapshot {
                    type_name: type_name.to_owned(),
                    fields,
                });
            }
            let name = registry
                .get_cpu::<Name>()
                .and_then(|s| s.get(entity))
                .map(|n| n.value.clone());
            let parent = parents
                .and_then(|s| s.get(entity))
                .map(|p| EntityId::from(p.entity));
            entities.push(EntitySnapshot {
                id: entity.into(),
                name,
                parent,
                components,
            });
        }
    }

    Response::ok(id, ResponseData::Entities { entities })
}

/// Reports every registered component type and its editable field layout.
fn get_schema(id: u64, resources: &Resources) -> Response {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Response::err(
            id,
            RemoteError::Unavailable {
                detail: "no ComponentRegistry".into(),
            },
        );
    };

    let components = registry
        .all_type_names()
        .into_iter()
        .map(|(type_id, type_name)| {
            let fields = registry.reflect_field_metas(&type_id).map(|metas| {
                metas
                    .iter()
                    .map(|m| FieldSchema {
                        name: m.name.to_owned(),
                        type_name: m.type_name.to_owned(),
                        choices: m.choices.iter().map(|c| c.label.to_owned()).collect(),
                        asset_type: m.asset_type.to_owned(),
                    })
                    .collect()
            });
            ComponentSchema {
                type_name: type_name.to_owned(),
                fields,
                category: registry.reflect_category(&type_id).map(str::to_owned),
            }
        })
        .collect();

    Response::ok(id, ResponseData::Schema { components })
}

/// Resolves a live entity handle, erroring if it is not alive.
fn resolve_entity(resources: &Resources, id: EntityId) -> Result<Entity, RemoteError> {
    let entity = Entity::from(id);
    let alive = resources
        .get::<EntityAllocator>()
        .is_some_and(|a| a.is_alive(entity));
    if alive {
        Ok(entity)
    } else {
        Err(RemoteError::NoSuchEntity { entity: id })
    }
}

/// Resolves a component name to a local `TypeId`.
fn resolve_component(resources: &Resources, type_name: &str) -> Result<TypeId, RemoteError> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.type_id_by_name(type_name))
        .ok_or_else(|| RemoteError::UnknownComponent {
            type_name: type_name.to_owned(),
        })
}

fn set_field(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
    field: &str,
    value: ome_ecs::reflect::ReflectValue,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;
    let registry =
        resources
            .get_mut::<ComponentRegistry>()
            .ok_or_else(|| RemoteError::Unavailable {
                detail: "no ComponentRegistry".into(),
            })?;
    registry
        .reflect_set_field(&type_id, entity, field, value)
        .map_err(|e| RemoteError::FieldError {
            detail: e.to_string(),
        })
}

fn add_component(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;

    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
    Ok(())
}

fn remove_component(
    resources: &mut Resources,
    entity: EntityId,
    component: &str,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let type_id = resolve_component(resources, component)?;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.remove_component(entity, &type_id);
    }
    update_archetype_remove(resources, entity, type_id);
    Ok(())
}

fn spawn(resources: &mut Resources, name: Option<&str>) -> Entity {
    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    let entity = commands.spawn(resources).id();
    resources.insert(commands);

    if let Some(name) = name
        && resources
            .get_mut::<ComponentRegistry>()
            .is_some_and(|r| r.insert_default_reflected(&TypeId::of::<Name>(), entity))
    {
        update_archetype_add(resources, entity, TypeId::of::<Name>());
        if let Some(storage) = resources
            .get_mut::<ComponentRegistry>()
            .and_then(|r| r.get_cpu_mut::<Name>())
            && let Some(n) = storage.get_mut(entity)
        {
            n.value = name.to_owned();
        }
    }
    entity
}

fn despawn(resources: &mut Resources, entity: EntityId) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    commands.despawn(entity);
    commands.apply(resources);
    resources.insert(commands);
    Ok(())
}

fn load_scene(resources: &mut Resources, path: &str) -> Result<(), RemoteError> {
    let doc = SceneDocument::load(path.as_ref()).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    sync_scene_to_ecs(&doc, resources).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })
}

/// Moves an entity to the archetype it belongs in after adding `type_id`.
fn update_archetype_add(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let new_arch = archetypes.archetype_after_add_dynamic(current, type_id);
        archetypes.register_entity(entity, new_arch);
    }
}

/// Moves an entity to the archetype it belongs in after removing `type_id`.
fn update_archetype_remove(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let new_arch = archetypes.archetype_after_remove_dynamic(current, type_id);
        archetypes.register_entity(entity, new_arch);
    }
}

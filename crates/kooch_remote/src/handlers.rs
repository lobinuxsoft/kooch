//! Executes a [`Request`] against the live ECS on the main thread — the server thread only ferries
//! bytes. An unknown component name is [`RemoteError::UnknownComponent`], never a panic.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::name::Name;
use kooch_ecs::scene::{SceneDocument, sync_scene_to_ecs};
use kooch_ecs::transform::Transform;
use kooch_ecs::world_snapshot::WorldSnapshot;

use crate::protocol::MovedTransform;
use crate::protocol::{
    ComponentSchema, ComponentSnapshot, EntityId, EntitySnapshot, FieldSchema, Method, RemoteError,
    Request, Response, ResponseData, SceneEntry,
};

/// Runs `request` against `resources` and returns the response to send.
pub fn handle(request: &Request, resources: &mut Resources) -> Response {
    let id = request.id;
    match &request.method {
        Method::Ping => Response::ok(id, ResponseData::Pong),
        Method::Extension { name, payload } => {
            match crate::extensions::call(resources, name, payload) {
                Some(Ok(result)) => Response::ok(
                    id,
                    ResponseData::Extension {
                        name: name.clone(),
                        result,
                    },
                ),
                Some(Err(detail)) => Response::err(
                    id,
                    RemoteError::ExtensionFailed {
                        name: name.clone(),
                        detail,
                    },
                ),
                None => Response::err(id, RemoteError::UnknownExtension { name: name.clone() }),
            }
        }
        Method::ListEntities { since } => list_entities(id, resources, *since),
        Method::ListMoved { since } => list_moved(id, resources, *since),
        Method::GetSchema => get_schema(id, resources),
        Method::SetField {
            entity,
            component,
            field,
            value,
        } => match set_field(resources, *entity, component, field, value.clone()) {
            Ok(()) => {
                touch_entity(resources, *entity);
                Response::ok(id, ResponseData::Ok)
            }
            Err(e) => Response::err(id, e),
        },
        Method::AddComponent { entity, component } => {
            match add_component(resources, *entity, component) {
                Ok(()) => {
                    touch_entity(resources, *entity);
                    Response::ok(id, ResponseData::Ok)
                }
                Err(e) => Response::err(id, e),
            }
        }
        Method::RemoveComponent { entity, component } => {
            match remove_component(resources, *entity, component) {
                Ok(()) => {
                    touch_entity(resources, *entity);
                    Response::ok(id, ResponseData::Ok)
                }
                Err(e) => Response::err(id, e),
            }
        }
        Method::Spawn {
            name,
            scene,
            parent,
        } => {
            let entity = spawn(resources, name.as_deref(), *scene, *parent);
            // The scene it actually landed in, which `spawn` has just
            // recorded — not the active one, which is only where it goes
            // when nobody said otherwise.
            touch_entity(resources, entity.into());
            Response::ok(
                id,
                ResponseData::Spawned {
                    entity: entity.into(),
                },
            )
        }
        Method::Despawn { entity } => {
            // Read before it goes: an entity that no longer exists cannot
            // say which scene just lost it.
            let scene = scene_of(resources, *entity);
            match despawn(resources, *entity) {
                Ok(()) => {
                    touch_scene(resources, scene);
                    Response::ok(id, ResponseData::Ok)
                }
                Err(e) => Response::err(id, e),
            }
        }
        Method::SetParent { entity, parent } => match set_parent(resources, *entity, *parent) {
            Ok(()) => {
                touch_entity(resources, *entity);
                Response::ok(id, ResponseData::Ok)
            }
            Err(e) => Response::err(id, e),
        },
        Method::SaveScene { path, scene } => match save_scene(resources, path, *scene) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::SavePrefab { entity, path } => match save_prefab(resources, *entity, path) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::ReloadAsset { path } => {
            reload_asset(resources, path);
            Response::ok(id, ResponseData::Ok)
        }
        Method::InstantiatePrefab { path } => match instantiate_prefab(resources, path) {
            Ok(entity) => {
                touch_entity(resources, entity);
                Response::ok(id, ResponseData::Spawned { entity })
            }
            Err(e) => Response::err(id, e),
        },
        Method::MoveEntity {
            entity,
            parent,
            before,
        } => match move_entity(resources, *entity, *parent, *before) {
            Ok(()) => {
                touch_entity(resources, *entity);
                Response::ok(id, ResponseData::Ok)
            }
            Err(e) => Response::err(id, e),
        },
        Method::RevertScene { scene } => match revert_scene(resources, *scene) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::NewScene => match resources.get_mut::<kooch_ecs::SceneManager>() {
            Some(manager) => Response::ok(
                id,
                ResponseData::SceneOpened {
                    scene: manager.new_scene(),
                },
            ),
            None => Response::err(
                id,
                RemoteError::Unavailable {
                    detail: "no SceneManager; there is no open set to add to".into(),
                },
            ),
        },
        Method::LoadScene { path } => match load_scene(resources, path) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::CloseScene { scene } => match close_scene(resources, *scene) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::SetActiveScene { scene } => match set_active_scene(resources, *scene) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::LoadSceneAdditive { path } => match load_scene_additive(resources, path) {
            Ok(scene) => Response::ok(id, ResponseData::SceneOpened { scene }),
            Err(e) => Response::err(id, e),
        },
        Method::SetPlaying { playing } => match set_playing(resources, *playing) {
            Ok(()) => Response::ok(id, ResponseData::Ok),
            Err(e) => Response::err(id, e),
        },
        Method::ListSystems => Response::ok(
            id,
            ResponseData::Systems {
                systems: list_systems(resources),
            },
        ),
        Method::SetSystemEnabled { name, nth, enabled } => {
            set_system_enabled(resources, name, *nth, *enabled);
            Response::ok(id, ResponseData::Ok)
        }
    }
}

/// Snapshots every non-hierarchy component on every alive entity, built from
/// [`SceneDocument::from_ecs`] and annotated with the live [`EntityId`]s a client addresses.
fn list_entities(id: u64, resources: &mut Resources, since: Option<u64>) -> Response {
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

    // Membership travels beside the components in `scene`, not among them, or the same fact goes
    // out twice.
    let skip = [
        TypeId::of::<Parent>(),
        TypeId::of::<kooch_ecs::hierarchy::Children>(),
        TypeId::of::<kooch_ecs::hierarchy::GlobalTransform>(),
        TypeId::of::<kooch_ecs::SceneMember>(),
    ];
    let parents = registry.get_cpu::<Parent>();
    let members = registry.get_cpu::<kooch_ecs::SceneMember>();

    // Ascending index is authored order, since entities are allocated in scene order — archetype
    // iteration scrambles it.
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
            let scene = members.and_then(|s| s.get(entity)).map(|m| m.scene);
            entities.push(EntitySnapshot {
                id: entity.into(),
                name,
                parent,
                scene,
                components,
            });
        }
    }
    entities.sort_by_key(|e| e.id.index);

    // Diffed against the last world described, so a scene that did not
    // move costs the editor nothing to receive or parse (#691). The
    // cache lives in `Resources` because it has to outlive the request.
    let mut cache = resources
        .remove::<crate::snapshot_cache::SnapshotCache>()
        .unwrap_or_default();
    let delta = cache.reply(entities, since);
    resources.insert(cache);

    Response::ok(
        id,
        ResponseData::Entities {
            entities: delta.entities,
            removed: delta.removed,
            revision: delta.revision,
            full: delta.full,
            host: host_metrics(resources),
            scenes: open_scenes(resources),
        },
    )
}

/// What moved since the caller's revision (#1012). 🔴 No reflection: one Transform column and
/// sixteen floats, where reflecting everything took 38.9 ms on 2159 entities. `full` means the
/// entity set changed.
fn list_moved(id: u64, resources: &mut Resources, since: Option<u64>) -> Response {
    let current = {
        let Some(registry) = resources.get::<ComponentRegistry>() else {
            return Response::err(
                id,
                RemoteError::Unavailable {
                    detail: "no ComponentRegistry".into(),
                },
            );
        };
        let Some(transforms) = registry.get_cpu::<Transform>() else {
            return Response::err(
                id,
                RemoteError::Unavailable {
                    detail: "no Transform storage".into(),
                },
            );
        };
        // 🔴 One pass over the Transform column. ⚠️ Despawn is deferred, so the allocator check
        // filters entities already gone.
        let alive = resources.get::<EntityAllocator>();
        let live = |entity: Entity| alive.is_none_or(|a| a.is_alive(entity));
        let mut current: Vec<MovedTransform> = transforms
            .iter()
            .filter(|(entity, _)| live(**entity))
            .map(|(&entity, transform)| MovedTransform {
                id: entity.into(),
                matrix: transform.to_matrix().to_cols_array(),
            })
            .collect();
        // The cache compares by id, and a HashMap hands them out in an
        // order that changes with its own internals. Sorted so the
        // reply is stable frame to frame and a diff means what it says.
        current.sort_unstable_by_key(|m| m.id.index);
        current
    };

    let mut cache = resources
        .remove::<crate::moved_cache::MovedCache>()
        .unwrap_or_default();
    let delta = cache.reply(current, since);
    resources.insert(cache);

    Response::ok(
        id,
        ResponseData::Moved {
            moved: delta.moved,
            removed: delta.removed,
            revision: delta.revision,
            full: delta.full,
            host: host_metrics(resources),
        },
    )
}

/// The scenes this project has open; `None` without a `SceneManager`, distinct from none open so
/// the editor keeps its list.
fn open_scenes(resources: &Resources) -> Option<Vec<SceneEntry>> {
    let manager = resources.get::<kooch_ecs::SceneManager>()?;
    let active = manager.active_id();
    Some(
        manager
            .scenes()
            .iter()
            .map(|scene| SceneEntry {
                id: scene.id,
                path: scene
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                active: active == Some(scene.id),
                dirty: scene.dirty,
            })
            .collect(),
    )
}

/// The host's frame cost from the engine's own measurement; `None` before the first frame is
/// described, not a zero that reads as infinitely fast.
fn host_metrics(resources: &Resources) -> Option<crate::protocol::HostMetrics> {
    let metrics = resources.get::<kooch_core::frame_metrics::FrameMetrics>()?;
    if metrics.frame_ms <= 0.0 {
        return None;
    }
    Some(crate::protocol::HostMetrics {
        frame_ms: metrics.frame_ms,
        cpu_frame_ms: metrics.cpu_frame_ms,
        ticks_instant: metrics.fps_instant,
        ticks_per_second: metrics.fps_average,
    })
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
                        doc: m.doc.to_owned(),
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

/// Every system the host schedules and whether it runs, from the catalog the `App` published; empty
/// before `App::run`.
fn list_systems(resources: &Resources) -> Vec<crate::protocol::SystemEntry> {
    let Some(catalog) = resources.get::<kooch_core::schedule::SystemCatalog>() else {
        return Vec::new();
    };
    let toggles = resources.get::<kooch_core::schedule::SystemToggles>();
    catalog
        .all()
        .iter()
        .map(|system| crate::protocol::SystemEntry {
            stage: format!("{:?}", system.stage),
            name: system.name.clone(),
            short: system.short_name().to_owned(),
            nth: system.key.nth,
            project: system.source == kooch_core::schedule::SystemSource::Project,
            gpu: system.gpu,
            enabled: !toggles
                .as_ref()
                .is_some_and(|toggles| toggles.is_disabled(&system.key)),
        })
        .collect()
}

/// Stops or restarts one system, creating the toggle set on demand.
fn set_system_enabled(resources: &mut Resources, name: &str, nth: u32, enabled: bool) {
    use kooch_core::schedule::{SystemKey, SystemToggles};

    if resources.get::<SystemToggles>().is_none() {
        resources.insert(SystemToggles::new());
    }
    let Some(toggles) = resources.get_mut::<SystemToggles>() else {
        return;
    };
    let key = SystemKey::nth(name, nth);
    match enabled {
        true => toggles.enable(key),
        false => toggles.disable(key),
    }
}

/// The authored world held during play — a [`WorldSnapshot`], not a [`SceneDocument`]: loading a
/// scene respawns with new identities, and Stop must keep every handle valid.
struct PlaySnapshot(WorldSnapshot);

/// Starts or stops gameplay in place: snapshot on start, restore on stop, preserving handles,
/// generations, order and the allocator. Idempotent both ways.
fn set_playing(resources: &mut Resources, playing: bool) -> Result<(), RemoteError> {
    if playing == kooch_core::run_state::Playing::is_playing(resources) {
        return Ok(());
    }

    if playing {
        resources.insert(PlaySnapshot(WorldSnapshot::capture(resources)));
        kooch_core::run_state::Playing::set(resources, true);
        tracing::info!("remote: play");
        return Ok(());
    }

    // Stop: the gate goes down before the restore so no system observes
    // a half-rebuilt world.
    kooch_core::run_state::Playing::set(resources, false);
    if let Some(snapshot) = resources.remove::<PlaySnapshot>() {
        snapshot.0.restore(resources);
        forget_the_world(resources);
    }
    tracing::info!("remote: stop");
    Ok(())
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

mod entities;
mod scenes;

use entities::*;
use scenes::*;

#[cfg(test)]
mod tests;

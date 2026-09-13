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

/// Captures `entity` and its descendants into a scene file.
fn save_prefab(resources: &mut Resources, entity: EntityId, path: &str) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let document = SceneDocument::from_ecs_subtree(resources, entity);
    // A prefab file promises exactly one root — the invariant its extension
    // names. Enforced on write so it cannot be discovered at the click that
    // instances it.
    document.root_index().map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    // Writes the `.meta` alongside, so the prefab is a registered asset the
    // moment it exists rather than the first time something loads it.
    kooch_ecs::scene::prefab::save(&document, path.as_ref())
        .map(|_| ())
        .map_err(|e| RemoteError::SceneError {
            detail: e.to_string(),
        })
}

/// Brings the project's copy of an asset back in line with the disk: overwritten in place so
/// existing handles see it, and registered so a new file's guid resolves.
fn reload_asset(resources: &mut Resources, path: &str) {
    let written = kooch_core::asset_loader::asset_written(path.as_ref(), resources);
    tracing::info!(
        target: "kooch_remote::assets",
        %path,
        reloaded = written.reloaded,
        registered = written.guid.is_some(),
        "asset written by the editor",
    );
}

/// Stamps a prefab file into the live world and hands back its root.
fn instantiate_prefab(resources: &mut Resources, path: &str) -> Result<EntityId, RemoteError> {
    let prefab = SceneDocument::load(path.as_ref()).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;

    // The instance belongs to the scene being edited. With no scene active
    // it becomes its own — a fresh guid rather than a shared sentinel,
    // which two unrelated instances would collide inside.
    let into = resources
        .get::<kooch_ecs::SceneManager>()
        .and_then(|scenes| scenes.active_id())
        .unwrap_or_else(kooch_core::Guid::new_v4);

    let (root, members) =
        kooch_ecs::scene::instantiate_members(&prefab, resources, into).map_err(|e| {
            RemoteError::SceneError {
                detail: e.to_string(),
            }
        })?;

    // The prefab link is attached here because this method *is* the editor's instancing; a game's
    // `spawn_prefab` attaches nothing.
    match kooch_core::asset_meta::read_meta(path.as_ref()) {
        Ok(meta) => {
            kooch_ecs::prefab_instance::attach(resources, root, &members, meta.guid);
            tracing::info!(
                target: "kooch_remote::prefab",
                prefab = %meta.guid,
                members = members.len(),
                "instance linked to its prefab",
            );
        }
        // Without an identity there is nothing to link *to*. Said out loud
        // because the instance still spawns, so the only visible symptom
        // is that it never follows the prefab afterwards.
        Err(e) => tracing::warn!(
            target: "kooch_remote::prefab",
            path = %path,
            "instanced but not linked, no asset identity: {e}",
        ),
    }
    Ok(EntityId::from(root))
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
    value: kooch_ecs::reflect::ReflectValue,
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

/// Spawns an entity with what every authored entity carries: `Name` and `Transform`, named or not,
/// as the editor's local spawn does.
fn spawn(
    resources: &mut Resources,
    name: Option<&str>,
    scene: Option<kooch_core::Guid>,
    parent: Option<EntityId>,
) -> Entity {
    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    let entity = commands.spawn(resources).id();
    resources.insert(commands);

    add_default(resources, entity, TypeId::of::<Name>());
    add_default(resources, entity, TypeId::of::<Transform>());

    if let Some(name) = name
        && let Some(storage) = resources
            .get_mut::<ComponentRegistry>()
            .and_then(|r| r.get_cpu_mut::<Name>())
        && let Some(n) = storage.get_mut(entity)
    {
        n.value = name.to_owned();
    }

    if let Some(parent) = parent {
        let _ = set_parent(resources, entity.into(), Some(parent));
    }
    // The parent's scene wins: an entity's scene *is* its parent's, so a
    // child authored into a different one would be written to a file its
    // parent is not in and come back an orphan.
    let home = parent
        .and_then(|parent| scene_of(resources, parent))
        .or(scene)
        .or_else(|| {
            resources
                .get::<kooch_ecs::SceneManager>()
                .and_then(|manager| manager.active_id())
        });
    if let Some(home) = home {
        tag_with_scene(resources, entity, home);
    }
    entity
}

/// Records which scene a newly spawned entity belongs to. 🔴 Without it the entity is Unsaved until
/// a save adopts it into the active scene.
fn tag_with_scene(resources: &mut Resources, entity: Entity, scene: kooch_core::Guid) {
    use kooch_ecs::SceneMember;

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    update_archetype_add(resources, entity, TypeId::of::<SceneMember>());
}

/// Inserts `type_id`'s default on `entity` and moves it to its new archetype; an unregistered type
/// is skipped, not fatal.
fn add_default(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
}

/// Reparents an entity, or unparents with `None`, through `kooch_ecs::hierarchy::reparent` — the
/// editor's own code, keeping the world transform.
fn set_parent(
    resources: &mut Resources,
    entity: EntityId,
    parent: Option<EntityId>,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    // Resolved before the call so an unknown parent is reported rather than
    // silently unparenting the child to the root.
    let parent = match parent {
        Some(parent) => Some(resolve_entity(resources, parent)?),
        None => None,
    };
    kooch_ecs::hierarchy::reparent(resources, entity, parent);
    Ok(())
}

/// Despawns an entity **and everything under it**, or children survive with a dead `Parent` and
/// accumulate in the saved scene.
fn despawn(resources: &mut Resources, entity: EntityId) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;

    // Collected before anything is despawned: the walk reads `Children`,
    // and despawning as it goes would cut the branch it is standing on.
    let doomed = match resources.get::<ComponentRegistry>() {
        Some(registry) => kooch_ecs::hierarchy::collect_descendants(entity, &registry),
        None => vec![entity],
    };

    let mut commands = resources
        .remove::<Commands>()
        .expect("Commands not in Resources");
    for entity in doomed {
        commands.despawn(entity);
    }
    commands.apply(resources);
    resources.insert(commands);
    Ok(())
}

fn load_scene(resources: &mut Resources, path: &str) -> Result<(), RemoteError> {
    match load_through_manager(resources, path) {
        Some(result) => result?,
        None => load_directly(resources, path)?,
    }
    // Refreshes prefab copies left stale while this scene was closed — here, where the scene
    // arrives.
    kooch_ecs::scene::propagate::refresh_all(resources);
    Ok(())
}

/// Closes one open scene, despawning only its entities; asking about unsaved edits is the editor's
/// job.
fn close_scene(resources: &mut Resources, scene: kooch_core::Guid) -> Result<(), RemoteError> {
    let mut manager = resources
        .remove::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?;
    let closed = manager.close(scene, resources);
    resources.insert(manager);
    match closed {
        true => Ok(()),
        false => Err(RemoteError::SceneError {
            detail: format!("scene {scene} is not open"),
        }),
    }
}

/// Points the active slot at an already-open scene.
fn set_active_scene(resources: &mut Resources, scene: kooch_core::Guid) -> Result<(), RemoteError> {
    let known = resources
        .get_mut::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?
        .set_active(scene);
    match known {
        true => Ok(()),
        false => Err(RemoteError::SceneError {
            detail: format!("scene {scene} is not open"),
        }),
    }
}

/// Opens a scene beside the loaded ones; unlike [`load_scene`] nothing is despawned, so client
/// handles stay valid.
fn load_scene_additive(
    resources: &mut Resources,
    path: &str,
) -> Result<kooch_core::Guid, RemoteError> {
    let path = std::path::PathBuf::from(path);
    let mut manager = resources
        .remove::<kooch_ecs::SceneManager>()
        .ok_or_else(|| RemoteError::Unavailable {
            detail: "no SceneManager".to_owned(),
        })?;
    let opened = manager.open_additive(&path, resources);
    resources.insert(manager);
    let scene = opened.map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    // Same reason as `load_scene`: a prefab edited while this scene was
    // closed left stale copies in it, and this is where it arrives.
    kooch_ecs::scene::propagate::refresh_all(resources);
    Ok(scene)
}

/// Marks dirty the scene holding `entity` — its own scene, not the active one. 🔴 Before this
/// nothing marked a scene dirty, so the asterisk never appeared.
fn touch_entity(resources: &mut Resources, entity: EntityId) {
    let scene = scene_of(resources, entity);
    touch_scene(resources, scene);
}

/// Which scene an entity belongs to, or `None` for one that belongs to
/// none — spawned here and not yet adopted by a save.
fn scene_of(resources: &Resources, entity: EntityId) -> Option<kooch_core::Guid> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<kooch_ecs::SceneMember>()?
        .get(Entity::from(entity))
        .map(|member| member.scene)
}

/// Marks one scene dirty, or the active one when the entity belonged to
/// none.
fn touch_scene(resources: &mut Resources, scene: Option<kooch_core::Guid>) {
    let Some(manager) = resources.get_mut::<kooch_ecs::SceneManager>() else {
        return;
    };
    match scene {
        // A scene the project does not have open is not this host's to
        // record. `mark_scene_dirty` says so; nothing here can act on it.
        Some(id) => {
            manager.mark_scene_dirty(id);
        }
        None => manager.mark_dirty(),
    }
}

/// Writes one open scene to `path` through the project's manager, so it adopts the path and clears
/// dirty. 🔴 One scene, not the world, keeping its id. `None` saves the active one.
fn save_scene(
    resources: &mut Resources,
    path: &str,
    scene: Option<kooch_core::Guid>,
) -> Result<(), RemoteError> {
    let Some(mut manager) = resources.remove::<kooch_ecs::SceneManager>() else {
        // Refused rather than falling back to writing everything alive:
        // that fallback is the bug this function exists to remove, and a
        // silent one is worse than an error naming what is missing.
        return Err(RemoteError::Unavailable {
            detail: "no SceneManager; nothing knows which scene to write".into(),
        });
    };
    let result = match scene.or_else(|| manager.active_id()) {
        Some(id) => manager
            .save_scene_as(id, std::path::PathBuf::from(path), resources)
            .map_err(|e| RemoteError::SceneError {
                detail: e.to_string(),
            }),
        None => Err(RemoteError::SceneError {
            detail: "no scene is open".into(),
        }),
    };
    resources.insert(manager);
    result
}

/// Moves an entity among its siblings, through the engine's own policy.
fn move_entity(
    resources: &mut Resources,
    entity: EntityId,
    parent: Option<EntityId>,
    before: Option<EntityId>,
) -> Result<(), RemoteError> {
    let entity = resolve_entity(resources, entity)?;
    let parent = parent.map(|p| resolve_entity(resources, p)).transpose()?;
    // A `before` that is no longer alive means "last", not an error: the
    // client is describing a list it read a frame ago, and refusing would
    // turn a stale row into a failed drag.
    let before = before.map(Entity::from).filter(|e| {
        resources
            .get::<EntityAllocator>()
            .is_some_and(|a| a.is_alive(*e))
    });

    match kooch_ecs::order::place(resources, entity, parent, before) {
        true => Ok(()),
        // The one refusal `place` makes: into its own subtree, which
        // would detach that subtree from the world.
        false => Err(RemoteError::FieldError {
            detail: "an entity cannot be moved into its own subtree".into(),
        }),
    }
}

/// Throws away one scene's edits and reads it back from its file, with the manager lifted out as a
/// load does.
fn revert_scene(
    resources: &mut Resources,
    scene: Option<kooch_core::Guid>,
) -> Result<(), RemoteError> {
    let Some(mut manager) = resources.remove::<kooch_ecs::SceneManager>() else {
        return Err(RemoteError::Unavailable {
            detail: "no SceneManager; nothing knows which scene to revert".into(),
        });
    };
    let result = match scene.or_else(|| manager.active_id()) {
        Some(id) => manager
            .revert(id, resources)
            .map_err(|e| RemoteError::SceneError {
                detail: e.to_string(),
            }),
        None => Err(RemoteError::SceneError {
            detail: "no scene is open".into(),
        }),
    };
    resources.insert(manager);
    // A prefab edited while this scene held stale copies of it: the
    // entities were just respawned from the file, so they need the same
    // refresh a load gives them.
    if result.is_ok() {
        kooch_ecs::scene::propagate::refresh_all(resources);
    }
    result
}

/// Loads through the project's [`SceneManager`](kooch_ecs::SceneManager), so its record matches the
/// world — straight to the ECS, the second scene opened kept the first one's name. `None` without a
/// manager.
fn load_through_manager(resources: &mut Resources, path: &str) -> Option<Result<(), RemoteError>> {
    // Lifted out and put back: `load` needs `&mut Resources` for the ECS
    // it is about to replace, and the manager lives in there too.
    let mut manager = resources.remove::<kooch_ecs::SceneManager>()?;
    let result = manager.load(path.as_ref(), resources);
    resources.insert(manager);
    Some(result.map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    }))
}

/// Loads without a manager: the entities arrive, nothing records them.
fn load_directly(resources: &mut Resources, path: &str) -> Result<(), RemoteError> {
    let doc = SceneDocument::load(path.as_ref()).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })?;
    sync_scene_to_ecs(&doc, resources).map_err(|e| RemoteError::SceneError {
        detail: e.to_string(),
    })
}

/// Drops both diff caches so the next reply describes the whole world. 🔴 A restore is invisible to
/// a diff, and the cache answering after Stop never saw play (#1035).
fn forget_the_world(resources: &mut Resources) {
    resources.remove::<crate::snapshot_cache::SnapshotCache>();
    resources.remove::<crate::moved_cache::MovedCache>();
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

#[cfg(test)]
mod tests;

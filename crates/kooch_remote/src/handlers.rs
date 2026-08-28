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
        Method::LoadSceneAdditive { path } => match load_scene_additive(resources, path) {
            Ok(scene) => Response::ok(id, ResponseData::SceneOpened { scene }),
            Err(e) => Response::err(id, e),
        },
        Method::SetPlaying { playing } => match set_playing(resources, *playing) {
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

    // Membership is reflected — so a world rebuild carries it — but it
    // still travels beside the components in `scene`, not among them.
    // Sending both would put the same fact on the wire twice and let a
    // client act on whichever it read last.
    let skip = [
        TypeId::of::<Parent>(),
        TypeId::of::<kooch_ecs::hierarchy::Children>(),
        TypeId::of::<kooch_ecs::hierarchy::GlobalTransform>(),
        TypeId::of::<kooch_ecs::SceneMember>(),
    ];
    let parents = registry.get_cpu::<Parent>();
    let members = registry.get_cpu::<kooch_ecs::SceneMember>();

    // Archetype iteration groups by component set, which scrambles the
    // order the user authored. Entities are allocated in the order the
    // scene lists them, so ascending index is that authored order — and
    // it is what a client shows in its hierarchy.
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

/// The scenes this project has open, for the editor to list.
///
/// `None` when there is no [`SceneManager`], which is what a host that
/// never loaded one looks like — distinct from "none are open", so the
/// editor keeps showing what it had rather than blanking the panel.
///
/// [`SceneManager`]: kooch_ecs::SceneManager
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

/// The host's own frame cost, read from the engine's measurement rather
/// than timed again here.
///
/// `None` before the first frame has been described — `FrameMetrics`
/// publishes a frame late on purpose, and a zero would read as a project
/// running infinitely fast rather than as one that has not been measured
/// yet.
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

/// Brings the project's copy of an asset file back in line with the disk.
///
/// Overwrites what is loaded rather than dropping it: the project's world
/// holds handles into `Assets<T>`, and forgetting a cache entry would
/// leave every one of them pointing at the bytes from before the edit —
/// the next load would allocate a new slot that nobody is looking at.
///
/// Registering the identity is the other half, and it is what makes a
/// file the project has never seen usable: a lookup by guid can only find
/// what the database knows about.
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

    // The link is attached here and not inside `instantiate`, because this
    // method *is* the editor's instancing — a shipped game does not run
    // this server, and its own spawner calls `spawn_prefab`, which
    // deliberately attaches nothing.
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

/// Spawns an entity carrying what every authored entity carries.
///
/// `Name` and `Transform` go on unconditionally, named or not. The
/// editor's local path (`undo/commands/spawn.rs`) has always added both,
/// and this one only added `Name`, and only when a name came with it — so
/// "Spawn → Entity", which sends no name, produced an entity with neither
/// in a remote project and one with both in a local one. An entity with no
/// `Name` cannot be renamed from the Inspector at all: the name editor
/// reads the component, and there was nothing to read.
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

/// Records which scene a newly spawned entity belongs to.
///
/// 🔴 Without this a spawned entity carries no `SceneMember` at all, so
/// the World panel files it under "Unsaved" and it only joins a scene
/// when a save adopts it — which is the active scene, whatever the user
/// actually asked for.
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

/// Inserts `type_id`'s default on `entity` and moves it to the archetype
/// that now describes it. A type this binary has no registration for is
/// skipped rather than fatal — the same stance the rest of this module
/// takes towards names it cannot resolve.
fn add_default(resources: &mut Resources, entity: Entity, type_id: TypeId) {
    let inserted = resources
        .get_mut::<ComponentRegistry>()
        .is_some_and(|r| r.insert_default_reflected(&type_id, entity));
    if inserted {
        update_archetype_add(resources, entity, type_id);
    }
}

/// Reparents an entity, or unparents it when `parent` is `None`.
///
/// Delegates to `kooch_ecs::hierarchy::reparent`, which is the same code the
/// editor's local path runs — the operation preserves the child's
/// world-space transform and moves it between archetypes, and having two
/// implementations of that would guarantee they drift.
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

/// Despawns an entity **and everything under it**.
///
/// A child holds a `Parent` pointing at an entity that no longer exists;
/// leaving it behind gives an entity whose transform is derived from a
/// dead handle and which nothing in the hierarchy can reach. It survives
/// the save, too, so the orphans accumulate in the scene file.
///
/// `collect_descendants` existed for exactly this and had never been
/// wired to anything but its own tests.
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
    // A prefab edited while this scene was closed left stale copies in it.
    // Done here rather than editor-side because this is where the scene
    // actually arrives — the editor would have to wait for the mirror
    // before it even knew what was in it.
    kooch_ecs::scene::propagate::refresh_all(resources);
    Ok(())
}

/// Opens a scene beside the ones already loaded.
///
/// Unlike [`load_scene`] nothing is despawned, so the entities already
/// in the world keep their identities and every handle the client holds
/// stays valid.
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

/// Records that the scene holding `entity` has edits not on disk.
///
/// 🔴 Nothing marked a scene dirty anywhere in the engine before this.
/// `SceneManager::mark_dirty` was called by its own tests and by nobody
/// else, so `dirty` was permanently `false`: the World panel's asterisk
/// could never appear, `any_dirty()` always answered "nothing to lose",
/// and a close-without-saving prompt built on it would have waved the
/// user straight through. Nobody had ever seen the asterisk, so nobody
/// noticed it was inert.
///
/// The scene of the entity that changed, not the active one — with two
/// scenes open those are different, and marking the active one puts the
/// asterisk on the file that did not change.
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

/// Writes one open scene to `path`, through the project's manager.
///
/// 🔴 One scene, not the world. This used to call
/// [`SceneDocument::from_ecs`] — `Capture::Everything` plus a fresh
/// `Guid` for the document. With two scenes open it wrote both into the
/// file, so the next load spawned every entity twice; and the new id on
/// every save broke anything that referred to the scene by identity.
///
/// Through the manager rather than straight to `from_ecs_scene` so the
/// scene adopts the path and its dirty flag is cleared — a save that
/// leaves the record saying "unsaved" is a save the user cannot see
/// happened.
///
/// `None` saves the active scene, which is what a client that knows of
/// only one sends.
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

/// Throws away one scene's edits and reads it back from its file.
///
/// Lifted out and put back for the same reason a load is: the manager
/// needs `&mut Resources` for the ECS it is about to replace, and it
/// lives in there.
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

/// Loads through the project's [`SceneManager`], so it knows what it has.
///
/// 🔴 This used to go straight to [`sync_scene_to_ecs`], which loads the
/// entities and tells the manager nothing — so after this call the
/// manager still described the scene *before* it, and every entity in
/// the world named a file it had never heard of.
///
/// The boot scene hid it. `SceneBootstrapPlugin` loads through the
/// manager, so a host that opens its startup scene and is never asked
/// for another looks perfectly correct: the record and the world agree,
/// because neither has moved since. It is the second scene that breaks
/// — the editor opening a different one — and the project would then go
/// on naming the first with the entities of the second inside it.
///
/// `None` when there is no manager to load through, which is a host that
/// never installed `EcsPlugin` rather than a failure.
///
/// [`SceneManager`]: kooch_ecs::SceneManager
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

/// The authored world, held while a play session runs so Stop can put
/// it back. Present only between a start and the matching stop.
///
/// A [`WorldSnapshot`], not a [`SceneDocument`]: the scene format is
/// name-keyed, so loading one back respawns everything with fresh
/// indices, fresh generations and a different order. Stop must be
/// indistinguishable from never having pressed play, which means the
/// identities have to survive — a client mirroring this world addresses
/// entities by handle, and so does every `Parent` in it.
struct PlaySnapshot(WorldSnapshot);

/// Starts or stops gameplay in place.
///
/// Play is destructive by nature — systems mutate the very entities the
/// user authored — so the world is snapshotted on start and restored on
/// stop. The restore preserves entity handles, generations, order and
/// the allocator state, so a client's [`EntityId`]s stay valid across a
/// play session and its mirror sees fields change, not a new world.
///
/// Idempotent in both directions: starting while already playing keeps
/// the original snapshot (so a double Play cannot lose the authored
/// state), and stopping while stopped is a no-op.
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

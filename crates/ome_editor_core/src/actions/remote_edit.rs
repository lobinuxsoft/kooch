//! The remote sink of the dual-sink edit dispatch.
//!
//! In local mode an [`EditorAction`] mutates the editor's own ECS. In
//! remote mode the ECS is a mirror of a project that owns the real state,
//! so the same action is routed over the wire instead — the mirror then
//! catches up on the next refresh. This is the *only* place the two modes
//! diverge: panels, DTOs and the viewport are identical either way.
//!
//! Translation resolves the editor's local identities back to the
//! project's: a mirrored [`Entity`] to its remote
//! [`EntityId`](ome_remote::protocol::EntityId) via the mirror, and a
//! [`ComponentId`] to its type name via the interner — the same name the
//! server keys components by.

use ome_core::resource::Resources;
use ome_ecs::component::ComponentNames;

use crate::actions::EditorAction;
use crate::remote_session::RemoteState;

/// Attempts to handle `action` over the wire.
///
/// Returns `true` when the action is an ECS edit that was routed to the
/// server (or dropped because it cannot be, e.g. an unresolved entity) —
/// the caller must not also apply it locally. Returns `false` for actions
/// that remote mode does not own (project management, editor settings),
/// which the caller handles through the normal local path.
///
/// The caller guarantees a connected session before calling.
pub(crate) fn dispatch(resources: &Resources, action: &EditorAction) -> bool {
    // Undo/Redo have no remote form yet; swallow them in remote mode
    // rather than replaying against the mirror (which the next refresh
    // overwrites anyway).
    if matches!(action, EditorAction::Undo | EditorAction::Redo) {
        return true;
    }

    // Non-ECS actions stay on the local path even in remote mode: closing
    // the project, toggling power profiles all act on the editor, not the
    // remote world.
    let Some(edit) = classify(action) else {
        return false;
    };

    let Some(state) = resources.get::<RemoteState>() else {
        return true;
    };
    let Some(session) = state.session.as_ref() else {
        return true;
    };
    let names = resources.get::<ComponentNames>();

    if let Err(e) = send(edit, session, &state.mirror, names, resources) {
        tracing::warn!("remote edit dropped: {e}");
    }
    true
}

/// An ECS edit reduced to the fields the remote protocol needs.
enum Edit<'a> {
    SetField {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
        field: &'a str,
        value: &'a ome_ecs::reflect::ReflectValue,
    },
    AddComponent {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
    },
    RemoveComponent {
        entity: ome_ecs::entity::Entity,
        component: ome_ecs::component::ComponentId,
    },
    Despawn(ome_ecs::entity::Entity),
    Spawn {
        name: Option<String>,
    },
    /// Every field of a `Transform`, from a gizmo drag.
    TransformEdit {
        entity: ome_ecs::entity::Entity,
        transform: ome_ecs::transform::Transform,
    },
    /// Write the project's world to a scene file, or replace it from one.
    SaveScene,
    LoadScene,
}

/// Reduces an action to an [`Edit`], or `None` if remote mode does not
/// own it.
fn classify(action: &EditorAction) -> Option<Edit<'_>> {
    match action {
        EditorAction::SetField {
            entity,
            component,
            field,
            value,
        } => Some(Edit::SetField {
            entity: *entity,
            component: *component,
            field,
            value,
        }),
        EditorAction::AddComponent { entity, component } => Some(Edit::AddComponent {
            entity: *entity,
            component: *component,
        }),
        EditorAction::RemoveComponent { entity, component } => Some(Edit::RemoveComponent {
            entity: *entity,
            component: *component,
        }),
        EditorAction::Despawn(entity) => Some(Edit::Despawn(*entity)),
        EditorAction::Spawn { name, .. } => Some(Edit::Spawn { name: name.clone() }),
        EditorAction::TransformEdit { entity, after, .. } => Some(Edit::TransformEdit {
            entity: *entity,
            transform: *after,
        }),
        // Scene I/O belongs to the project: the mirror is a view, and
        // saving it locally would write a partly-parked copy over the
        // project's own scene file.
        EditorAction::SaveScene => Some(Edit::SaveScene),
        EditorAction::OpenScene => Some(Edit::LoadScene),
        // Not something remote mode owns (project mgmt, settings, …).
        _ => None,
    }
}

/// Sends one [`Edit`] to the project's server.
fn send(
    edit: Edit<'_>,
    session: &crate::remote_session::RemoteSession,
    mirror: &crate::remote_mirror::RemoteMirror,
    names: Option<&ComponentNames>,
    resources: &Resources,
) -> Result<(), String> {
    use ome_ecs::reflect::ReflectValue;

    let client = session.client();
    // Maps a local mirror entity to the remote id the server addresses.
    let remote = |e| {
        mirror
            .remote_of(e)
            .ok_or_else(|| "entity not in mirror".to_owned())
    };
    // Resolves a portable component id to the type name the server keys by.
    let name = |c| {
        names
            .and_then(|n| n.name(c))
            .map(str::to_owned)
            .ok_or_else(|| "component id not interned".to_owned())
    };
    let map_err = |e: ome_remote::ClientError| e.to_string();

    match edit {
        Edit::SetField {
            entity,
            component,
            field,
            value,
        } => client
            .set_field(remote(entity)?, &name(component)?, field, value.clone())
            .map_err(map_err),
        Edit::AddComponent { entity, component } => client
            .add_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::RemoveComponent { entity, component } => client
            .remove_component(remote(entity)?, &name(component)?)
            .map_err(map_err),
        Edit::Despawn(entity) => client.despawn(remote(entity)?).map_err(map_err),
        Edit::Spawn { name } => client.spawn(name.as_deref()).map(|_| ()).map_err(map_err),
        Edit::TransformEdit { entity, transform } => {
            let id = remote(entity)?;
            let ty = std::any::type_name::<ome_ecs::transform::Transform>();
            // A gizmo drag replaces the whole transform; push each field.
            for (field, value) in [
                ("position", ReflectValue::Vec3(transform.position)),
                ("rotation", ReflectValue::Quat(transform.rotation)),
                ("scale", ReflectValue::Vec3(transform.scale)),
            ] {
                client.set_field(id, ty, field, value).map_err(map_err)?;
            }
            Ok(())
        }
        // Both processes see the same filesystem, so the path the user
        // picks here is meaningful on the project's side of the wire.
        Edit::SaveScene => match crate::actions::scene_io::scene_dialog(resources).save_file() {
            Some(path) => client.save_scene(&path.to_string_lossy()).map_err(map_err),
            None => Ok(()),
        },
        Edit::LoadScene => match crate::actions::scene_io::scene_dialog(resources).pick_file() {
            Some(path) => client.load_scene(&path.to_string_lossy()).map_err(map_err),
            None => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ome_core::resource::Resources;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::commands::Commands;
    use ome_ecs::component::{ComponentNames, ComponentRegistry};
    use ome_ecs::dynamic_components::DynamicComponents;
    use ome_ecs::name::Name;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::ReflectValue;
    use ome_ecs::transform::Transform;

    use ome_remote::RemoteClient;
    use ome_remote::handlers::handle;
    use ome_remote::protocol::{Method, Request};
    use ome_remote::server::RemoteServer;

    use super::*;
    use crate::remote_session::{ConnectionState, RemoteSession, RemoteState};

    fn ecs() -> Resources {
        let mut r = Resources::new();
        r.insert(EntityAllocator::new());
        r.insert(ComponentRegistry::new());
        r.insert(ArchetypeRegistry::new());
        r.insert(AccessTracker::new());
        r.insert(Commands::new());
        r.insert(DynamicComponents::new());
        r.insert(ComponentNames::new());
        {
            let reg = r.get_mut::<ComponentRegistry>().unwrap();
            reg.register_cpu_reflected::<Name>();
            reg.register_cpu_reflected::<Transform>();
        }
        r
    }

    /// A `SetField` issued in the editor lands on the project's server.
    #[test]
    fn set_field_routes_to_the_server() {
        let transform_ty = std::any::type_name::<Transform>();

        // Server side: a project with one Transform-bearing entity.
        let server = (0..8)
            .find_map(|i| RemoteServer::start(17760 + i).ok())
            .expect("bind");
        let port = server.port();
        let done = Arc::new(AtomicBool::new(false));
        let loop_done = Arc::clone(&done);
        let main_loop = std::thread::spawn(move || {
            let mut res = ecs();
            let hero = match handle(
                &Request {
                    id: 0,
                    method: Method::Spawn {
                        name: Some("Hero".into()),
                    },
                },
                &mut res,
            )
            .payload
            {
                ome_remote::protocol::ResponsePayload::Result(
                    ome_remote::protocol::ResponseData::Spawned { entity },
                ) => entity,
                _ => panic!("spawn"),
            };
            handle(
                &Request {
                    id: 1,
                    method: Method::AddComponent {
                        entity: hero,
                        component: transform_ty.into(),
                    },
                },
                &mut res,
            );
            while !loop_done.load(Ordering::Relaxed) {
                for item in server.take_pending() {
                    let resp = handle(&item.request, &mut res);
                    let _ = item.reply.send(resp);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        // Editor side: connect, mirror, then issue an edit.
        let mut editor = ecs();
        let mut state = RemoteState::new();
        state.session = Some(RemoteSession::attach(port));
        for _ in 0..200 {
            if state.session.as_mut().unwrap().poll_ready() == ConnectionState::Connected {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.is_connected(), "did not connect");

        let snapshot = state.session.as_ref().unwrap().snapshot().to_vec();
        state.mirror.apply(&snapshot, &mut editor);
        let remote_id = snapshot[0].id;
        let local = state.mirror.local_of(remote_id).expect("mirrored");
        let comp = editor
            .get_mut::<ComponentNames>()
            .unwrap()
            .intern(transform_ty);
        editor.insert(state);

        let action = EditorAction::SetField {
            entity: local,
            component: comp,
            field: "position".into(),
            value: ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)),
        };
        assert!(
            dispatch(&editor, &action),
            "remote dispatch should own SetField"
        );

        // The server now holds the edited value.
        let client = RemoteClient::new(port);
        let entities = client.list_entities().unwrap();
        let pos = entities[0]
            .components
            .iter()
            .find(|c| c.type_name.ends_with("Transform"))
            .and_then(|c| c.fields.iter().find(|(n, _)| n == "position"))
            .map(|(_, v)| v.clone());
        assert_eq!(
            pos,
            Some(ReflectValue::Vec3(glam::Vec3::new(9.0, 9.0, 9.0)))
        );

        done.store(true, Ordering::Relaxed);
        main_loop.join().unwrap();
    }

    /// A non-ECS action is not owned by the remote sink.
    #[test]
    fn non_ecs_action_falls_through() {
        let editor = ecs();
        assert!(!dispatch(&editor, &EditorAction::CloseProject));
    }
}

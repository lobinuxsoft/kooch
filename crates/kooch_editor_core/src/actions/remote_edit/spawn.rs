//! Spawning on the project's side: meshes, blocks, and duplicates of what is already there.

use super::*;

/// Builds a mesh-bound entity on the project's side.
pub(super) fn spawn_mesh(resources: &mut Resources, path: &std::path::Path, name: &str) {
    const TARGET: &str = "kooch_editor_core::remote_edit::spawn_mesh";

    let Some((guid, asset_type)) = resolve_mesh_asset(resources, path) else {
        return;
    };

    let Some(state) = resources.get::<RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let client = session.client();

    // The active scene: a mesh dropped into the viewport is authored
    // where new things go, and nothing in that gesture names another.
    let entity = match client.spawn(Some(name), None, None) {
        Ok(entity) => entity,
        Err(e) => {
            tracing::warn!(target: TARGET, error = %e, "remote spawn failed");
            return;
        }
    };

    // Remote `spawn` creates only `Name`, so both of these are needed.
    let transform_ty = std::any::type_name::<kooch_ecs::transform::Transform>();
    let renderer_ty = std::any::type_name::<kooch_ecs::mesh_renderer::MeshRenderer>();
    for ty in [transform_ty, renderer_ty] {
        if let Err(e) = client.add_component(entity, ty) {
            tracing::warn!(target: TARGET, component = ty, error = %e, "add_component failed");
            return;
        }
    }

    let value = kooch_ecs::reflect::ReflectValue::AssetRef {
        guid: Some(guid),
        asset_type,
    };
    if let Err(e) = client.set_field(entity, renderer_ty, "mesh", value) {
        tracing::warn!(target: TARGET, error = %e, "could not write the mesh reference");
        return;
    }

    tracing::info!(
        target: TARGET,
        %name,
        path = %path.display(),
        %guid,
        "spawned a mesh entity on the project",
    );

    // Set here rather than through `created`: this arm never reaches
    // `send`, because resolving the asset needs a mutable world and an
    // `Edit` has to be sendable from an immutable one.
    if let Some(state) = resources.get_mut::<RemoteState>() {
        state.pending_selection = vec![entity];
    }
}

/// Builds a block on the project's side.
pub(super) fn spawn_block(resources: &mut Resources, shape: kooch_blockmesh::Shape) {
    const TARGET: &str = "kooch_editor_core::remote_edit::spawn_block";
    use crate::undo::prototype_material;

    let Some((path, guid)) = crate::actions::asset_ops::new_block_asset(resources, shape) else {
        return;
    };

    let Some(state) = resources.get::<RemoteState>() else {
        return;
    };
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let client = session.client();

    let entity = match client.spawn(Some(shape.label()), None, None) {
        Ok(entity) => entity,
        Err(e) => {
            tracing::warn!(target: TARGET, error = %e, "remote spawn failed");
            return;
        }
    };

    // Remote `spawn` creates only `Name`, so every one of these is needed.
    let block_ty = std::any::type_name::<kooch_blockmesh::Block>();
    let body_ty = std::any::type_name::<kooch_physics::components::PhysicsBody>();
    // Remote `spawn` creates only `Name`, so `Transform` is added here
    // and the rest come from the one list both spawn paths share.
    let transform_ty = std::any::type_name::<kooch_ecs::transform::Transform>();
    let block_components = kooch_blockmesh::block_components();
    let types = std::iter::once(transform_ty).chain(block_components.map(|(_, name)| name));
    for ty in types {
        if let Err(e) = client.add_component(entity, ty) {
            // 🔴 The likeliest cause is a project built without the `blockmesh` feature: the
            // component exists in this editor and not over there, and "add_component failed" on its
            // own sends you looking at the wire.
            tracing::warn!(
                target: TARGET, component = ty, error = %e,
                "add_component failed — a project that does not enable \
                 kooch/blockmesh has no Block type to add",
            );
            return;
        }
    }

    // The parameters stay on the block for the Inspector. The menu spawns defaults, so `kind` is
    // the only field that differs (`defaults_differ_only_in_kind`).
    let shape_ty = std::any::type_name::<kooch_blockmesh::BlockShape>();
    let kind = kooch_ecs::reflect::ReflectValue::U32(kooch_blockmesh::BlockShape::from(shape).kind);
    if let Err(e) = client
        .add_component(entity, shape_ty)
        .and_then(|()| client.set_field(entity, shape_ty, "kind", kind))
    {
        tracing::warn!(target: TARGET, error = %e, "could not give the block its shape");
    }

    let value = kooch_ecs::reflect::ReflectValue::AssetRef {
        guid: Some(guid),
        asset_type: std::any::type_name::<kooch_blockmesh::BlockMesh>().to_owned(),
    };
    if let Err(e) = client.set_field(entity, block_ty, "source", value) {
        tracing::warn!(target: TARGET, error = %e, "could not write the block source");
        return;
    }

    // The engine's prototype grid, so the block shows its size. Its
    // UVs are one repeat per world unit, and on flat white a wall
    // pulled two metres and one pulled four look identical.
    if let Some(material) = prototype_material(resources) {
        let renderer_ty = std::any::type_name::<kooch_ecs::mesh_renderer::MeshRenderer>();
        let value = kooch_ecs::reflect::ReflectValue::AssetRef {
            guid: Some(material),
            asset_type: std::any::type_name::<kooch_render::material::Material>().to_owned(),
        };
        if let Err(e) = client.set_field(entity, renderer_ty, "material", value) {
            tracing::warn!(target: TARGET, error = %e, "could not give the block a material");
        }
    }

    // Static, because a wall that falls over is not a level. Set over
    // the wire like any other field: the project owns the world, and
    // `kind` is a plain reflected u32.
    if let Err(e) = client.set_field(
        entity,
        body_ty,
        "kind",
        kooch_ecs::reflect::ReflectValue::U32(kooch_physics::components::KIND_STATIC),
    ) {
        tracing::warn!(target: TARGET, error = %e, "could not make the block static");
    }

    tracing::info!(
        target: TARGET, path = %path.display(), %guid,
        "spawned a block on the project",
    );

    if let Some(state) = resources.get_mut::<RemoteState>() {
        state.pending_selection = vec![entity];
    }
}

/// Loads a mesh asset locally and returns its GUID and asset type name.
pub(super) fn resolve_mesh_asset(
    resources: &mut Resources,
    path: &std::path::Path,
) -> Option<(kooch_core::Guid, String)> {
    const TARGET: &str = "kooch_editor_core::remote_edit::spawn_mesh";
    use kooch_core::asset_database::AssetDatabase;
    use kooch_core::asset_loader::AssetServer;
    use kooch_render::meshlet::MeshletMesh;

    // Taken out so the load can borrow the server and the world at once,
    // the same dance `SpawnMeshCommand` does on the local path.
    let mut server = match resources.remove::<AssetServer>() {
        Some(server) => server,
        None => {
            tracing::warn!(target: TARGET, "AssetServer missing; cannot resolve the mesh");
            return None;
        }
    };
    let loaded = server.load::<MeshletMesh>(path, resources);
    let resolved = server.resolve_path(path);
    resources.insert(server);

    if let Err(e) = loaded {
        tracing::warn!(
            target: TARGET,
            path = %path.display(),
            error = %e,
            "failed to load the mesh asset",
        );
        return None;
    }

    let guid = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.guid_for(&resolved));
    if guid.is_none() {
        tracing::warn!(
            target: TARGET,
            resolved = %resolved.display(),
            "the AssetDatabase has no entry for the loaded asset",
        );
    }
    guid.map(|guid| (guid, std::any::type_name::<MeshletMesh>().to_owned()))
}

/// Copies an entity on the project side, out of what the mirror already knows.
pub(super) fn duplicate(
    entity: kooch_ecs::entity::Entity,
    client: &kooch_remote::RemoteClient,
    mirror: &crate::remote_mirror::RemoteMirror,
    resources: &Resources,
) -> Result<kooch_remote::protocol::EntityId, String> {
    // Only to fail early with a clear reason: an entity the mirror does
    // not know is one the project never had.
    mirror
        .remote_of(entity)
        .ok_or_else(|| "entity not in mirror".to_owned())?;

    let state = crate::actions::entity_state::as_copy(&crate::actions::entity_state::capture(
        resources, entity,
    ));
    let copy = build(client, mirror, &state, None)?;
    tracing::info!(
        target: "kooch_editor_core::remote_edit::duplicate",
        components = state.components.len(),
        "duplicated an entity on the project",
    );
    Ok(copy)
}

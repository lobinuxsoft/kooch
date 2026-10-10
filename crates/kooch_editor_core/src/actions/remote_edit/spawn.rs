//! Spawning on the project's side: meshes, blocks, and duplicates of what is already there.

use super::*;

/// Builds a mesh-bound entity on the project's side.
pub(super) fn spawn_mesh(
    resources: &mut Resources,
    path: &std::path::Path,
    name: &str,
    at: Option<glam::Vec3>,
) {
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
    place(client, entity, transform_ty, at);

    tracing::info!(
        target: TARGET,
        %name,
        path = %path.display(),
        %guid,
        "spawned a mesh entity on the project",
    );

    remember(resources, "Spawn Mesh Entity", entity);

    // Set here rather than through `created`: this arm never reaches
    // `send`, because resolving the asset needs a mutable world and an
    // `Edit` has to be sendable from an immutable one.
    if let Some(state) = resources.get_mut::<RemoteState>() {
        state.pending_selection = vec![entity];
    }
}

/// Builds a block on the project's side.
pub(super) fn spawn_block(
    resources: &mut Resources,
    shape: kooch_blockmesh::Shape,
    at: Option<glam::Vec3>,
) {
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
    place(client, entity, transform_ty, at);

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

    remember(resources, "Spawn Block", entity);

    if let Some(state) = resources.get_mut::<RemoteState>() {
        state.pending_selection = vec![entity];
    }
}

/// Puts a freshly spawned entity where the cursor asked for.
///
/// A failure is logged and swallowed: the entity exists either way, and an author who sees it at
/// the origin can drag it. Refusing the spawn over a misplaced one would be worse.
fn place(
    client: &kooch_remote::RemoteClient,
    entity: kooch_remote::protocol::EntityId,
    transform_ty: &str,
    at: Option<glam::Vec3>,
) {
    let Some(at) = at else {
        return;
    };
    let value = kooch_ecs::reflect::ReflectValue::Vec3(at);
    if let Err(e) = client.set_field(entity, transform_ty, "position", value) {
        tracing::warn!(
            target: "kooch_editor_core::remote_edit::place",
            error = %e,
            "could not place the new entity",
        );
    }
}

/// Puts the spawn in the remote history.
///
/// 🔴 These two arms return from `dispatch` before it reaches `remote_undo::record`, so until this
/// neither a mesh nor a block spawned against a connected project could be undone at all (#1461).
fn remember(resources: &mut Resources, label: &str, entity: kooch_remote::protocol::EntityId) {
    crate::actions::remote_undo::record_step(
        resources,
        label,
        crate::actions::remote_undo::Inverse::Despawn(vec![entity]),
    );
    // 🔴 Undoing reads the MIRROR to capture what it is about to despawn, so a redo can rebuild
    // it. These arms skip the `sent` path that normally asks for a refresh, and a Ctrl+Z pressed
    // before the half-second poll would undo an entity the mirror has never seen — and redo
    // nothing.
    super::pull_soon(resources);
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

    // The whole subtree: an entity with children is one thing to an author (#1292).
    let tree = crate::actions::entity_state::capture_tree(resources, entity);
    let parent = crate::actions::entity_state::parent_of(resources, entity)
        .and_then(|parent| mirror.remote_of(parent));
    let copies = build_tree(client, mirror, &tree, None, parent)?;
    tracing::info!(
        target: "kooch_editor_core::remote_edit::duplicate",
        entities = copies.len(),
        "duplicated a subtree on the project",
    );
    copies
        .first()
        .copied()
        .ok_or_else(|| "the copy built nothing".to_owned())
}

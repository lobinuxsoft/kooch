//! A Collider pointed at a baked collision mesh takes the shape the bake was made for (#1660).

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_meta;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentNames;
use kooch_ecs::reflect::ReflectValue;
use kooch_physics::components::{
    Collider, SHAPE_CONVEX_DECOMPOSITION, SHAPE_CONVEX_HULL, SHAPE_TRIMESH,
};

use super::{BakeKind, EditorAction};

/// One `shape` edit per action that sets `Collider.mesh` to a baked asset.
pub(super) fn shape_edits(resources: &Resources, actions: &[&EditorAction]) -> Vec<EditorAction> {
    let Some(names) = resources.get::<ComponentNames>() else {
        return Vec::new();
    };
    let collider = std::any::type_name::<Collider>();
    actions
        .iter()
        .filter_map(|action| {
            let EditorAction::SetField {
                entity,
                component,
                field,
                value:
                    ReflectValue::AssetRef {
                        guid: Some(guid), ..
                    },
            } = action
            else {
                return None;
            };
            if field != "mesh" || names.name(*component) != Some(collider) {
                return None;
            }
            Some(EditorAction::SetField {
                entity: *entity,
                component: *component,
                field: "shape".to_owned(),
                value: ReflectValue::U32(baked_shape(resources, *guid)?),
            })
        })
        .collect()
}

/// The shape a bake's sidecar asks for; `None` for an ordinary mesh, whose shape is the author's.
fn baked_shape(resources: &Resources, mesh: Guid) -> Option<u32> {
    let path = resources.get::<AssetDatabase>()?.entry(mesh)?.path.clone();
    let import = asset_meta::read_meta(&path).ok()?.import?;
    let kind = import.get("collision")?.as_str()?;
    match kind {
        _ if kind == BakeKind::Hull.tag() => Some(SHAPE_CONVEX_HULL),
        _ if kind == BakeKind::Parts.tag() => Some(SHAPE_CONVEX_DECOMPOSITION),
        _ if kind == BakeKind::Mesh.tag() => Some(SHAPE_TRIMESH),
        _ => None,
    }
}

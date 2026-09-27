//! Mesh renderer component.

use kooch_core::Guid;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Component that binds an entity to a mesh and a material for rendering.
#[derive(Debug, Clone, Reflect)]
#[reflect(category = "Rendering")]
pub struct MeshRenderer {
    /// When `Some`, the meshlet pipeline picks this entity up via the scene cull. Persistent across
    /// runs — the GUID lives in the asset's `.meta` sidecar. The inspector renders this as a typed
    /// dropdown picker that lists every `MeshletMesh` the `AssetDatabase` has registered.
    #[reflect(asset = "kooch_render::meshlet::asset::MeshletMesh")]
    pub mesh: Option<Guid>,
    /// When `Some`, the meshlet scene system resolves the GUID to a `MaterialPool` slot via the
    /// `MaterialPipeline` resource. The inspector renders this as a typed dropdown of every
    /// `Material` registered with the asset database.
    #[reflect(asset = "kooch_render::material::asset::Material")]
    pub material: Option<Guid>,
    /// Whether this renderer is drawn.
    pub visible: bool,
    /// Whether this renderer casts shadows.
    pub cast_shadows: bool,
    /// Whether this renderer receives shadows.
    pub receive_shadows: bool,
    /// The layers this renderer is in. A camera, a light or a shadow view keeps whatever its own
    /// mask meets, so being in several is how one mesh answers to more than one of them. "Default"
    /// is where everything starts; ticking none hides it from every view.
    #[reflect(layers)]
    pub layers: u32,
    /// The single layer a scene authored by the #1307 editor names. Migrated into `layers` on load
    /// and cleared, so nothing reads two answers for one question (#1320).
    #[reflect(hidden)]
    pub layer: u32,
}

impl MeshRenderer {
    /// What a camera's mask is tested against, migrating the single layer a #1307 scene names.
    pub fn layer_mask(&self) -> u32 {
        match self.layer {
            0 => self.layers,
            named => 1 << named.min(31),
        }
    }
}

/// Folds the single `layer` a #1307 scene names into `layers`, and clears it.
///
/// 🔴 Without this, an author ticking boxes in the Inspector edits a field the legacy one outranks:
/// the edit lands in the scene, changes nothing, and says nothing (#1320).
pub fn migrate_renderer_layers(resources: &mut kooch_core::resource::Resources) {
    let Some(registry) = resources.get_mut::<crate::component::ComponentRegistry>() else {
        return;
    };
    let Some(storage) = registry.get_cpu_mut::<MeshRenderer>() else {
        return;
    };
    for (&entity, renderer) in storage.iter_mut() {
        if renderer.layer != 0 {
            let was = renderer.layer;
            renderer.layers = renderer.layer_mask();
            renderer.layer = 0;
            tracing::info!(
                target: "kooch_ecs",
                entity = entity.index(),
                layer = was,
                layers = renderer.layers,
                "a renderer's layers were migrated from an older field",
            );
        }
    }
}

impl Default for MeshRenderer {
    fn default() -> Self {
        Self {
            mesh: None,
            material: None,
            visible: true,
            cast_shadows: true,
            receive_shadows: true,
            layer: 0,
            layers: kooch_core::layers::DEFAULT_LAYER,
        }
    }
}

impl Component for MeshRenderer {}

#[cfg(test)]
mod tests;

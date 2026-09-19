//! Loading the project's settings asset and publishing what the frame reads.

use super::*;

/// Finds the project's settings asset, loads it, and publishes the values the shading model reads.
pub fn apply_render_settings_system(resources: &mut Resources) {
    let Some(guid) = find_settings_guid(resources) else {
        return;
    };
    let Some(handle) =
        kooch_ecs::reflect::asset_registry::load_handle::<RenderSettings>(resources, guid)
    else {
        return;
    };
    let Some(settings) = resources
        .get::<kooch_core::assets::Assets<RenderSettings>>()
        .and_then(|assets| assets.get(handle))
        .copied()
    else {
        return;
    };

    // Only write when something changed. Inserting unconditionally would
    // be correct and would also mean every frame reports the resource as
    // freshly set, which any future change detection would believe.
    let exposure = Exposure::from_physical(settings.camera());
    let ambient = settings.ambient();
    let shadows = settings.shadows();
    let contact = settings.contact_shadows();
    let shading = settings.shading();
    let temporal = without_missing_dlss(settings.temporal(), resources);
    let meshlet_lod = settings.meshlet_lod();
    let presentation = settings.presentation();
    let window_mode = settings.window_mode();
    let stale = resources.get::<crate::quality::Presentation>() != Some(&presentation)
        || resources.get::<kooch_core::window_mode::WindowMode>() != Some(&window_mode)
        || resources.get::<Exposure>() != Some(&exposure)
        || resources.get::<AmbientLight>() != Some(&ambient)
        || resources.get::<ShadowSettings>() != Some(&shadows)
        || resources.get::<ContactShadowSettings>() != Some(&contact)
        || resources.get::<crate::quality::ShadingSettings>() != Some(&shading)
        || resources.get::<crate::quality::TemporalSettings>() != Some(&temporal)
        || resources.get::<crate::meshlet::MeshletLodSettings>() != Some(&meshlet_lod);
    if stale {
        settings.apply(resources);
        // 🔴 After `apply`, which inserts the technique the FILE asked for. A project authored on a
        // machine with DLSS is opened on one without, and the value it wrote is still the right
        // thing to keep in the asset — what must not survive is the engine then trying to run it.
        resources.insert(temporal);
        tracing::debug!(
            target: "kooch_render::settings",
            ev100 = exposure.ev100,
            "render settings applied",
        );
    }
}

/// Downgrades DLSS to the engine's own resolve when this build, or this adapter, cannot run it
/// (#536).
fn without_missing_dlss(
    mut temporal: crate::quality::TemporalSettings,
    resources: &Resources,
) -> crate::quality::TemporalSettings {
    if temporal.technique != crate::quality::UpscaleTechnique::Dlss {
        return temporal;
    }
    let available = resources
        .get::<kooch_core::gpu::DlssRuntime>()
        .is_some_and(|runtime| runtime.support.super_resolution);
    if available {
        return temporal;
    }
    warn_once_about_missing_dlss();
    temporal.technique = crate::quality::UpscaleTechnique::Taa;
    temporal.render_scale = 100;
    temporal
}

/// Says it once. The condition cannot change within a session — neither
/// the adapter nor the linked SDK does — so a line per frame would be a
/// log nobody reads.
fn warn_once_about_missing_dlss() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            target: "kooch_render::settings",
            "the project asks for DLSS and this build or adapter has none; \
             resolving with the engine's own TAA at full resolution instead",
        );
    });
}

/// The guid of the project's settings asset, if it has one.
fn find_settings_guid(resources: &Resources) -> Option<kooch_core::Guid> {
    let db = resources.get::<kooch_core::asset_database::AssetDatabase>()?;
    let type_name = std::any::type_name::<RenderSettings>();
    let mut found = db.entries_of_type(type_name);
    let first = found.next()?;
    if found.next().is_some() {
        tracing::warn!(
            target: "kooch_render::settings",
            "more than one .rendersettings in the project; using the first found. \
             Settings are per project, so the others do nothing.",
        );
    }
    Some(first.0)
}

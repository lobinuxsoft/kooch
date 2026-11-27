//! The generator as the CPU holds it: loaded from its shader asset, parsed, and revised.

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetServer, ReloadedAssets};
use kooch_core::assets::{Assets, Handle};
use kooch_core::resource::Resources;
use kooch_render::material::shader::{Shader, ShaderKind};

use std::sync::Arc;

use super::PlanetTerrain;
use crate::eval::Generator;

/// The generator planet `index` now has: its WGSL for the ground's GPU pass, its CPU twin, and
/// a new revision so every block made from the old one is rebuilt.
pub(super) fn set_generator(state: &mut PlanetTerrain, index: u32, source: &str) {
    let at = index as usize;
    if state.generators.len() <= at {
        state.generators.resize_with(at + 1, || None);
        state.sources.resize_with(at + 1, || None);
        state.twins.resize(at + 1, 0);
    }
    state.generators[at] = Generator::parse(source)
        .inspect_err(|why| tracing::debug!(target: "kooch_terrain", "no CPU generator: {why}"))
        .ok()
        .map(Arc::new);
    state.sources[at] = Some(Arc::from(source));
    state.twins[at] = state.twins[at].wrapping_add(1);
}

/// The generator's reload count, and its shader once loaded.
pub(super) fn load(resources: &mut Resources, guid: Option<Guid>) -> (u64, Option<Handle<Shader>>) {
    let Some(guid) = guid else {
        return (0, None);
    };
    let revision = resources
        .get::<ReloadedAssets>()
        .map_or(0, |reloaded| reloaded.revision(guid));
    let Some(mut server) = resources.remove::<AssetServer>() else {
        return (revision, None);
    };
    let handle = server.load_by_guid::<Shader>(guid, resources).ok();
    resources.insert(server);
    let loaded = handle.filter(|handle| {
        resources
            .get::<Assets<Shader>>()
            .is_some_and(|shaders| shaders.get(*handle).is_some())
    });
    (revision, loaded)
}

/// The generator's source, when it is a `planet` shader.
pub(super) fn source(resources: &Resources, handle: Handle<Shader>) -> Option<String> {
    let shader = resources.get::<Assets<Shader>>()?.get(handle)?;
    (shader.kind == ShaderKind::Terrain).then(|| shader.source.clone())
}

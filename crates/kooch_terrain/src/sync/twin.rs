//! The generator as the CPU holds it: loaded from its shader asset, parsed, and revised.

use kooch_core::Guid;
use kooch_core::asset_loader::{AssetServer, ReloadedAssets};
use kooch_core::assets::{Assets, Handle};
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_render::material::shader::{Shader, ShaderKind};

use std::sync::Arc;

use glam::DVec3;

use super::PlanetTerrain;
use crate::Field;
use crate::eval::{Generator, may_cross};
use crate::lod::voxel_size;
use crate::stamp::{SHAPE_FIELD, SHAPE_VOLUME, Stack, Stamp};
use crate::{BlockKey, Planets};

/// The generator planet `index` now has: its WGSL for the ground's GPU pass, its CPU twin, and
/// a new revision so every block made from the old one is rebuilt.
pub(super) fn set_generator(
    state: &mut PlanetTerrain,
    index: u32,
    source: &str,
    volumes: Vec<Guid>,
) {
    let at = index as usize;
    if state.generators.len() <= at {
        state.generators.resize_with(at + 1, || None);
        state.sources.resize_with(at + 1, || None);
        state.twins.resize(at + 1, 0);
        state.volumes.resize_with(at + 1, Vec::new);
    }
    state.volumes[at] = volumes;
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

/// A volume a terrain's stamps name: its asset, reload count, and shader once loaded.
pub(super) type Volume = (Guid, u64, Option<Handle<Shader>>);

/// The volumes `stamps` name, each once, in the order they are first named (#1957).
pub(super) fn load_volumes(resources: &mut Resources, stamps: &[Stamp]) -> Vec<Volume> {
    let mut guids: Vec<Guid> = Vec::new();
    for stamp in stamps.iter().filter(|s| s.shape == SHAPE_VOLUME) {
        if let Some(guid) = stamp.volume
            && !guids.contains(&guid)
        {
            guids.push(guid);
        }
    }
    guids
        .into_iter()
        .map(|guid| {
            let (revision, shader) = load(resources, Some(guid));
            (guid, revision, shader)
        })
        .collect()
}

/// The fields `stamps` name that have loaded, each once, in the order they are first named.
pub(super) fn load_fields(resources: &mut Resources, stamps: &[Stamp]) -> Vec<(Guid, Arc<Field>)> {
    let mut found: Vec<(Guid, Arc<Field>)> = Vec::new();
    for stamp in stamps.iter().filter(|s| s.shape == SHAPE_FIELD) {
        let Some(guid) = stamp.field else { continue };
        if found.iter().any(|(held, _)| *held == guid) {
            continue;
        }
        let Some(mut server) = resources.remove::<AssetServer>() else {
            break;
        };
        let handle = server.load_by_guid::<Field>(guid, resources).ok();
        resources.insert(server);
        let field =
            handle.and_then(|handle| resources.get::<Assets<Field>>()?.get(handle).cloned());
        if let Some(field) = field {
            found.push((guid, Arc::new(field)));
        }
    }
    found
}

pub(super) fn store_fields(state: &mut PlanetTerrain, index: u32, fields: Vec<(Guid, Arc<Field>)>) {
    let at = index as usize;
    if state.fields.len() <= at {
        state.fields.resize_with(at + 1, Vec::new);
    }
    state.fields[at] = fields;
}

/// What changes when a volume is named, reloaded or comes in, for the planet's signature.
pub(super) fn key_of(volumes: &[Volume]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (guid, revision, shader) in volumes {
        (guid, revision, shader.is_some()).hash(&mut hasher);
    }
    hasher.finish()
}

/// `generator` with every loaded `volume` shader joined in, and the volumes it joined. One that
/// does not join leaves its stamps off, not the planet bare.
pub(super) fn joined(
    resources: &Resources,
    planet: Entity,
    generator: &str,
    volumes: &[Volume],
) -> (String, Vec<Guid>) {
    let mut guids = Vec::new();
    let mut sources = Vec::new();
    for (guid, _, shader) in volumes {
        let text = shader.and_then(|handle| {
            let shader = resources.get::<Assets<Shader>>()?.get(handle)?;
            (shader.kind == ShaderKind::Volume).then(|| shader.source.clone())
        });
        let Some(text) = text else {
            if shader.is_some() {
                tracing::warn!(target: "kooch_terrain", %planet, %guid, "a stamp's volume is not a `volume` shader");
            }
            continue;
        };
        let mut trial: Vec<&str> = sources.iter().map(String::as_str).collect();
        trial.push(&text);
        match crate::join_volumes(generator, &trial) {
            Ok(_) => {
                guids.push(*guid);
                sources.push(text);
            }
            Err(why) => {
                tracing::warn!(target: "kooch_terrain", %planet, %guid, "a stamp's volume does not join: {why}");
            }
        }
    }
    let texts: Vec<&str> = sources.iter().map(String::as_str).collect();
    match crate::join_volumes(generator, &texts) {
        Ok(source) => (source, guids),
        Err(_) => (generator.to_owned(), Vec::new()),
    }
}

/// The generator's source, when it is a `planet` shader.
pub(super) fn source(resources: &Resources, handle: Handle<Shader>) -> Option<String> {
    let shader = resources.get::<Assets<Shader>>()?.get(handle)?;
    (shader.kind == ShaderKind::Terrain).then(|| shader.source.clone())
}

/// Whether the generator leaves `key`'s tile a chance of holding surface. The tile reaches one
/// sample past the block on every side, so its border is bounded too.
pub(crate) fn crosses(
    (generators, stacks): (&[Option<Arc<Generator>>], &[Arc<Stack>]),
    planets: &Planets,
    key: BlockKey,
    min: DVec3,
    max: DVec3,
) -> bool {
    let planet = key.planet as usize;
    let (Some(Some(generator)), Some(shape)) = (generators.get(planet), planets.shape.get(planet))
    else {
        return true;
    };
    let voxel = voxel_size(shape, key.level);
    let (min, max) = (min - DVec3::splat(voxel), max + DVec3::splat(voxel));
    let range = generator.range(shape, min, max, key.planet);
    let range = match stacks.get(planet) {
        Some(stack) if !stack.is_empty() => range
            .map(|range| stack.bound((min, max), range, Some((generator.as_ref(), key.planet)))),
        _ => range,
    };
    may_cross(range, voxel)
}

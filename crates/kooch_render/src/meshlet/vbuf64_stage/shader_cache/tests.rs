use std::cell::Cell;

use super::*;
use crate::meshlet::DEFAULT_SURFACE_SHADER;

fn source(revision: u64, text: &str) -> SurfaceSource {
    SurfaceSource {
        revision,
        source: text.into(),
        params: Vec::new().into(),
        params_wgsl: "".into(),
    }
}

/// 🔴 Ten materials on one shader are one pipeline.
#[test]
fn materials_share_a_shader_pipeline() {
    let cache = ShaderPipelines::new();
    let builds = Cell::new(0);
    let guid = Guid::new_v4();
    let surface = source(0, DEFAULT_SURFACE_SHADER);
    for _ in 0..10 {
        cache.get(guid, &surface, false, |_| builds.set(builds.get() + 1));
    }
    assert_eq!(builds.get(), 1);
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_new_revision_rebuilds() {
    let cache = ShaderPipelines::new();
    let guid = Guid::new_v4();
    assert_eq!(
        cache.get(guid, &source(0, DEFAULT_SURFACE_SHADER), false, |_| 1),
        Some(1)
    );
    assert_eq!(
        cache.get(guid, &source(1, DEFAULT_SURFACE_SHADER), false, |_| 2),
        Some(2)
    );
}

/// A broken save keeps the last good pipeline, and is tried once rather than every frame.
#[test]
fn a_broken_edit_keeps_the_last() {
    let cache = ShaderPipelines::new();
    let guid = Guid::new_v4();
    cache.get(guid, &source(0, DEFAULT_SURFACE_SHADER), false, |_| 1);
    let broken = source(1, "fn surface( {");
    let builds = Cell::new(0);
    for _ in 0..3 {
        let got = cache.get(guid, &broken, false, |_| {
            builds.set(builds.get() + 1);
            2
        });
        assert_eq!(got, Some(1));
    }
    assert_eq!(builds.get(), 0);
}

#[test]
fn a_never_valid_shader_is_none() {
    let cache = ShaderPipelines::<u32>::new();
    assert_eq!(
        cache.get(Guid::new_v4(), &source(0, "nope"), false, |_| 1),
        None
    );
}

/// The debug variant is its own pipeline.
#[test]
fn debug_is_a_separate_key() {
    let cache = ShaderPipelines::new();
    let guid = Guid::new_v4();
    let surface = source(0, DEFAULT_SURFACE_SHADER);
    cache.get(guid, &surface, false, |_| 1);
    assert_eq!(cache.get(guid, &surface, true, |_| 2), Some(2));
    assert_eq!(cache.len(), 2);
}

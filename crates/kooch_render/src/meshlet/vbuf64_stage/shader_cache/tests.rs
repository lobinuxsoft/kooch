use std::cell::Cell;

use super::*;
use crate::material::Shader;
use crate::meshlet::DEFAULT_SURFACE_SHADER;

fn source(revision: u64, text: &str) -> SurfaceSource {
    // A text that does not read as a shader still reaches the cache, the way a stale revision does.
    let shader = Shader::parse(text).unwrap_or_else(|_| Shader {
        source: text.to_owned(),
        ..Shader::parse("").unwrap()
    });
    SurfaceSource {
        revision,
        params_wgsl: shader.params_wgsl().into(),
        params: shader.params.clone().into(),
        kind: shader.kind,
        source: shader.source.into(),
    }
}

/// 🔴 Ten materials on one shader are one pipeline.
#[test]
fn materials_share_a_shader_pipeline() {
    let cache = ShaderPipelines::new(OPAQUE);
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
    let cache = ShaderPipelines::new(OPAQUE);
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
    let cache = ShaderPipelines::new(OPAQUE);
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
    let cache = ShaderPipelines::<u32>::new(OPAQUE);
    assert_eq!(
        cache.get(Guid::new_v4(), &source(0, "nope"), false, |_| 1),
        None
    );
}

/// The debug variant is its own pipeline.
#[test]
fn debug_is_a_separate_key() {
    let cache = ShaderPipelines::new(OPAQUE);
    let guid = Guid::new_v4();
    let surface = source(0, DEFAULT_SURFACE_SHADER);
    cache.get(guid, &surface, false, |_| 1);
    assert_eq!(cache.get(guid, &surface, true, |_| 2), Some(2));
    assert_eq!(cache.len(), 2);
}

/// 🔴 A post-process material sits in the same pool as the surfaces; the surface path must skip it
/// rather than compose it without `sample_scene` and log an error.
#[test]
fn a_post_process_is_skipped() {
    use tracing_subscriber::layer::SubscriberExt as _;

    let cache = ShaderPipelines::new(OPAQUE);
    let post = source(
        0,
        "// kind: post_process\nfn post_process(input: SurfaceInput) -> vec4<f32> {\n    return sample_scene(input.uv);\n}",
    );
    let logs = kooch_core::LogBuffer::new();
    let subscriber = tracing_subscriber::registry().with(logs.layer());
    let got = tracing::subscriber::with_default(subscriber, || {
        cache.get(Guid::new_v4(), &post, false, |_| 1)
    });
    assert_eq!(got, None);
    assert!(logs.snapshot().is_empty(), "the surface path tried it");
}

/// A transparent shader is the forward pass's: the opaque paths skip it, the forward cache builds it.
#[test]
fn transparent_goes_forward_only() {
    let glass = source(
        0,
        "// kind: transparent\nfn surface(input: SurfaceInput) -> SurfaceOutput {\n    var out: SurfaceOutput;\n    out.normal = normalize(input.world_normal);\n    out.alpha = 0.5;\n    return out;\n}",
    );
    let guid = Guid::new_v4();
    assert_eq!(
        ShaderPipelines::new(OPAQUE).get(guid, &glass, false, |_| 1),
        None
    );
    let forward = ShaderPipelines::new(&[ShaderKind::Transparent]);
    assert_eq!(forward.get(guid, &glass, false, |_| 1), Some(1));
}

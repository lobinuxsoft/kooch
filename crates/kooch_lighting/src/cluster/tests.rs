//! What can be pinned without a device.
//!
//! The grid's arithmetic has its own tests next to it in `grid.rs`.
//! These cover the two things that fail late and loudly otherwise: WGSL
//! that does not compile, and a Rust struct that no longer matches the
//! shader struct it mirrors.

use super::buffers::{ClusterDraw, ClusterViewUniform};
use super::passes::{RASTER_TEMPLATE, shader_sources};

#[test]
fn every_pass_parses_and_validates() {
    for (name, source) in shader_sources() {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{name} should parse: {e:?}"));
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name} should validate: {e:?}"));
    }
}

#[test]
fn the_rasterizer_is_compiled_twice() {
    // One source, two modules. If the placeholder ever stops being
    // substituted, both passes become the same pass and the grid fills
    // with counts nobody allocated for — or with indices nobody counted.
    assert!(RASTER_TEMPLATE.contains("{{CLUSTER_POPULATE}}"));
    let sources = shader_sources();
    let count = &sources[2].1;
    let populate = &sources[3].1;
    assert!(count.contains("const POPULATE: bool = false;"));
    assert!(populate.contains("const POPULATE: bool = true;"));
    assert!(!count.contains("{{CLUSTER_POPULATE}}"));
}

/// 🔴 The Rust structs and the WGSL structs have no compiler between
/// them. A field added on one side shifts every field after it on the
/// other, and the result renders something plausible and wrong.
#[test]
fn the_uniform_matches_the_shader_struct() {
    // Three 4x4 matrices and five vec4s.
    assert_eq!(
        size_of::<ClusterViewUniform>(),
        3 * 64 + 5 * 16,
        "ClusterView in cluster_common.wgsl has to be changed with it",
    );
    // Four words of draw arguments, then the two the CPU reads back.
    assert_eq!(size_of::<ClusterDraw>(), 8 * 4);
}

#[test]
fn a_fresh_draw_record_draws_nothing() {
    let draw = ClusterDraw::empty();
    // Six vertices are the quad; zero instances is a frame with no work
    // in it. A non-zero instance count here would draw from a work list
    // the z-slice pass has not written yet.
    assert_eq!(draw.vertex_count, 6);
    assert_eq!(draw.instance_count, 0);
    assert_eq!(draw.wanted, 0);
    assert_eq!(draw.index_size, 0);
}

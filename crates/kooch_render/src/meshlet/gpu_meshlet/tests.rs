use super::*;
use crate::meshlet::asset::DEFAULT_MAX_VERTICES;

#[test]
fn pad_to_4_is_idempotent_when_already_aligned() {
    assert_eq!(pad_to_4(&[1, 2, 3, 4]), vec![1, 2, 3, 4]);
    assert_eq!(pad_to_4(&[]), Vec::<u8>::new());
}

#[test]
fn pad_to_4_rounds_up() {
    assert_eq!(pad_to_4(&[1, 2, 3]), vec![1, 2, 3, 0]);
    assert_eq!(pad_to_4(&[1, 2, 3, 4, 5]), vec![1, 2, 3, 4, 5, 0, 0, 0]);
}

#[test]
fn binding_slot_constants_are_distinct() {
    let slots = [
        binding::VERTICES,
        binding::MESHLET_VERTICES,
        binding::MESHLET_TRIANGLES,
        binding::DESCRIPTORS,
    ];
    let mut sorted = slots.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), slots.len(), "binding slots collided");
}

#[test]
fn descriptor_size_matches_max_vertex_constant_at_compile_time() {
    // Defensive — DEFAULT_MAX_VERTICES is referenced by the
    // builder; confirm it stays within u8 since meshlet-local
    // triangle indices are u8 (0..max_vertices-1 fits when
    // max_vertices <= 256).
    assert!(DEFAULT_MAX_VERTICES <= 256);
}

#[test]
fn zeroed_descriptor_is_safe_default() {
    let d = zeroed_descriptor();
    assert_eq!(d.vertex_count, 0);
    assert_eq!(d.triangle_count, 0);
    assert_eq!(d.bounding_radius, 0.0);
}

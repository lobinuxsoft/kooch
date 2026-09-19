use glam::{Mat4, Vec3};

use super::list::{ForwardList, Run};
use crate::meshlet::MeshDescriptor;
use crate::meshlet::asset::MeshletDescriptor;
use crate::meshlet::scene::{MeshInstance, decode_scene_visible_id};
use crate::shadow::InstanceBounds;

fn meshlet(lod_level: u32) -> MeshletDescriptor {
    MeshletDescriptor {
        lod_level,
        ..bytemuck::Zeroable::zeroed()
    }
}

fn mesh(first_meshlet: u32, meshlet_count: u32) -> MeshDescriptor {
    MeshDescriptor {
        first_meshlet,
        meshlet_count,
        ..bytemuck::Zeroable::zeroed()
    }
}

fn at(z: f32) -> InstanceBounds {
    InstanceBounds {
        center: Vec3::new(0.0, 0.0, z),
        radius: 1.0,
        hash: 0,
    }
}

/// Instance 0 opaque, then three transparent ones at different distances.
fn scene(materials: [u32; 3]) -> (Vec<MeshInstance>, Vec<InstanceBounds>) {
    let mut instances = vec![MeshInstance::new(Mat4::IDENTITY, 0, 9)];
    let mut bounds = vec![at(0.0)];
    for (z, material) in [-2.0, -9.0, -5.0].into_iter().zip(materials) {
        instances.push(MeshInstance::new(Mat4::IDENTITY, 0, material));
        bounds.push(at(z));
    }
    (instances, bounds)
}

/// Blending is done in submission order, so the farthest instance goes first.
#[test]
fn far_glass_draws_first() {
    let (instances, bounds) = scene([1, 1, 1]);
    let mut list = ForwardList::default();
    list.rebuild(
        &instances,
        1,
        &bounds,
        Vec3::ZERO,
        &[mesh(0, 1)],
        &[meshlet(0)],
    );
    let order: Vec<u32> = list
        .entries
        .iter()
        .map(|&e| decode_scene_visible_id(e).0)
        .collect();
    assert_eq!(order, [2, 3, 1]);
}

/// Only the finest level is drawn: two levels of one surface would blend it twice.
#[test]
fn coarse_levels_are_skipped() {
    let (instances, bounds) = scene([1, 1, 1]);
    let mut list = ForwardList::default();
    let meshlets = [meshlet(0), meshlet(0), meshlet(1)];
    list.rebuild(&instances, 3, &bounds, Vec3::ZERO, &[mesh(0, 3)], &meshlets);
    let ids: Vec<u32> = list
        .entries
        .iter()
        .map(|&e| decode_scene_visible_id(e).1)
        .collect();
    assert_eq!(ids, [0, 1]);
}

/// Neighbours in the order sharing a material share a run; one in between splits it.
#[test]
fn runs_follow_the_order() {
    let (instances, bounds) = scene([1, 2, 1]);
    let mut list = ForwardList::default();
    list.rebuild(
        &instances,
        1,
        &bounds,
        Vec3::ZERO,
        &[mesh(0, 1)],
        &[meshlet(0)],
    );
    // Far to near: material 2, then 1, then 1.
    assert_eq!(
        list.runs,
        [
            Run {
                material: 2,
                range: 0..1
            },
            Run {
                material: 1,
                range: 1..3
            },
        ]
    );
}

/// No transparent instance, nothing to draw.
#[test]
fn an_opaque_scene_is_empty() {
    let (instances, bounds) = scene([1, 1, 1]);
    let mut list = ForwardList::default();
    list.rebuild(
        &instances,
        4,
        &bounds,
        Vec3::ZERO,
        &[mesh(0, 1)],
        &[meshlet(0)],
    );
    assert!(list.is_empty());
}

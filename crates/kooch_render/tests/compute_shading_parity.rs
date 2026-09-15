//! image as the fragment one — not a similar one.

mod common;

use common::lit_scene::{SIZE, render, rig};
use kooch_lighting::ClusterSettings;

/// The parity assertion, in the two halves that are actually provable.
fn assert_same_image(fragment: &[u8], compute: &[u8], what: &str) {
    assert_eq!(fragment.len(), compute.len(), "{what}: different sizes");
    // 🔴 Here rather than in one test, because two renders of nothing match byte for byte and every
    // assertion below then passes having compared an empty frame with an empty frame.
    assert!(
        fragment.chunks_exact(4).any(|p| p[3] != 0),
        "{what}: the scene rendered empty — the parity assertion would be vacuous",
    );

    let coverage = fragment
        .chunks_exact(4)
        .zip(compute.chunks_exact(4))
        .position(|(a, b)| a[3] != b[3]);
    if let Some(idx) = coverage {
        panic!(
            "{what}: the two paths cover different pixels. \
             first at ({}, {}): fragment alpha {}, compute alpha {}",
            idx as u32 % SIZE,
            idx as u32 / SIZE,
            fragment[idx * 4 + 3],
            compute[idx * 4 + 3],
        );
    }

    let worst = fragment
        .chunks_exact(4)
        .zip(compute.chunks_exact(4))
        .enumerate()
        .max_by_key(|(_, (a, b))| a.iter().zip(b.iter()).map(|(x, y)| x.abs_diff(*y)).max())
        .expect("a non-empty image");
    let (idx, (a, b)) = worst;
    let delta = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    assert!(
        delta <= 1,
        "{what}: the shading paths disagree by {delta}/255, past the one unit two \
         stages can differ by through quantisation alone.\n\
         worst at ({}, {}): fragment {a:?}, compute {b:?}",
        idx as u32 % SIZE,
        idx as u32 / SIZE,
    );
}

/// 🔴 The issue's acceptance criterion, in one assertion.
#[test]
fn both_paths_render_the_same() {
    let Some(mut rig) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "floor under a grid of point lights");
}

/// 🔴 From the smoke test of #1157: tile-sized holes wherever materials meet. Several frames, since
/// the holes moved between them.
#[test]
fn mixed_materials_match_every_frame() {
    let Some(mut rig) = common::lit_scene::rig_mixed(4) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let fragment = render(&mut rig, false);
    for frame in 0..4 {
        let compute = render(&mut rig, true);
        assert_same_image(
            &fragment,
            &compute,
            &format!("mixed materials, frame {frame}"),
        );
    }
}

/// The fallback the workgroup cache promises, on the path that is simplest to reach on purpose:
/// with no grid there is no cell to cache, and the compute pass must shade from the light buffer
/// exactly as the fragment path does.
#[test]
fn unclustered_falls_back_and_matches() {
    let Some(mut rig) = rig(3, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };
    rig.resources.insert(ClusterSettings {
        enabled: false,
        ..Default::default()
    });

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "clustering off");
}

/// A wall standing on the floor, so tiles along its silhouette hold pixels many z-slices apart and
/// draw their lights from a block of froxels rather than one.
#[test]
fn a_deep_silhouette_matches() {
    let Some(mut rig) = rig(4, true) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    let fragment = render(&mut rig, false);
    let compute = render(&mut rig, true);
    assert_same_image(&fragment, &compute, "wall silhouette");
}

/// 🔴 `KOOCH_LIGHT_LIMIT` exists to be measured with, and a knob that silently does nothing produces
/// a capture that looks like an answer.
#[test]
fn the_light_limit_darkens_both_paths() {
    let Some(mut rig) = rig(4, false) else {
        eprintln!("no adapter with the 64-bit texture-atomic bundle; skipping");
        return;
    };

    for compute in [false, true] {
        rig.resources.insert(kooch_lighting::LightLimit(0));
        let all = render(&mut rig, compute);
        rig.resources.insert(kooch_lighting::LightLimit(1));
        let capped = render(&mut rig, compute);

        let path = if compute { "compute" } else { "fragment" };
        assert_ne!(
            all, capped,
            "{path}: capping a froxel's lights to one changed nothing on screen",
        );
        // Darker, not merely different: the cap drops light, and a
        // change in the other direction would mean it is doing
        // something other than what it says.
        let sum = |p: &[u8]| p.chunks_exact(4).map(|c| c[0] as u64).sum::<u64>();
        assert!(
            sum(&capped) < sum(&all),
            "{path}: the capped render is not darker",
        );
    }
}

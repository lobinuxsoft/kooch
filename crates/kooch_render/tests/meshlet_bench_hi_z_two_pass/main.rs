//! Bench: Hi-Z 2-pass cull overhead vs single-pass scene-pool-atomic on the sphere fixture.

#[path = "../common/mod.rs"]
mod common;
mod render_paths;
mod rig;

pub(crate) const RT_SIZE: u32 = 256;
pub(crate) const FRAME_COUNT: usize = 32;
pub(crate) const WARMUP_FRAMES: usize = 4;
// 2-pass median ≤ 2.25 × single-pass. Above the current ~1.92 ratio
// observed on Mesa radv with a 1-instance sphere, below a 3× cliff
// that would point at a stray submit / poll insertion.
pub(crate) const OVERHEAD_BUDGET: f64 = 2.25;

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

use render_paths::{render_single_pass, render_two_pass};
use rig::build_rig;

fn measure(label: &str, mut step: impl FnMut()) -> f64 {
    for _ in 0..WARMUP_FRAMES {
        step();
    }
    let mut samples_ms = Vec::with_capacity(FRAME_COUNT);
    for _ in 0..FRAME_COUNT {
        let t0 = std::time::Instant::now();
        step();
        samples_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples_ms[samples_ms.len() / 2];
    let p99 = samples_ms[(samples_ms.len() * 99 / 100).min(samples_ms.len() - 1)];
    eprintln!("{label}: median={median:.3}ms p99={p99:.3}ms over {FRAME_COUNT} frames");
    median
}

#[test]
#[ignore = "bench: long-running, needs GPU"]
fn hi_z_two_pass_overhead_within_budget() {
    let Some(mut rig) = build_rig() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };

    let single_median = measure("single-pass", || render_single_pass(&rig));
    let mut arena: Vec<wgpu::BindGroup> = Vec::new();
    let two_pass_median = measure("two-pass   ", || {
        arena.clear();
        render_two_pass(&mut rig, &mut arena);
    });

    let ratio = two_pass_median / single_median;
    eprintln!(
        "Hi-Z 2-pass overhead: {:.1}% (regression budget = +{}% of single-pass)",
        (ratio - 1.0) * 100.0,
        ((OVERHEAD_BUDGET - 1.0) * 100.0) as i32
    );
    eprintln!(
        "Note: #445 spec target is ≤5% overhead. Current overhead is dominated \
         by pass B's worst-case dispatch + raster B's redraw with LoadOp::Load. \
         The optimisations to close the gap (indirect dispatch for pass B + \
         first_instance offset for raster B) are tracked as follow-up."
    );
    assert!(
        two_pass_median <= single_median * OVERHEAD_BUDGET,
        "Hi-Z 2-pass median {two_pass_median:.3} ms exceeded the regression \
         budget {budget:.3} ms (single-pass {single_median:.3} ms × {OVERHEAD_BUDGET}). \
         Above the current ~1.9× baseline → likely a real regression in the \
         orchestrator (extra submit, poll, or buffer rebuild added since merge).",
        budget = single_median * OVERHEAD_BUDGET,
    );
}

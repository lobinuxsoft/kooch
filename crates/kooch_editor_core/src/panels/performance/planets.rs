//! What the planets' voxel terrain costs (#313): the shared pool, the GPU batches and their time.

use kooch_planet::PlanetStats;

use super::widgets::{grid, metric, metric_with_tooltip};

pub(super) fn planet_readout(ui: &mut egui::Ui, stats: PlanetStats) {
    grid(ui, "perf_grid_planets", |ui| {
        let mib = f64::from(stats.resident) * kooch_planet::BLOCK_BYTES as f64 / 1_048_576.0;
        metric_with_tooltip(
            ui,
            "Blocks",
            &format!("{} / {} · {mib:.1} MiB", stats.resident, stats.capacity),
            "Voxel blocks every planet holds out of the one pool they share, and the atlas \
             memory they take. The pool never grows: a far planet gives up blocks to a near one.",
        );
        let limited = match stats.budget < stats.capacity {
            true => " · limited by mesh memory",
            false => "",
        };
        metric_with_tooltip(
            ui,
            "Budget",
            &format!("{}{limited}", stats.budget),
            "Blocks the shared mesh pool can hold at what a block costs right now. Dense relief \
             costs more per block, so near this limit the ground gets coarser instead of the \
             render failing.",
        );
        metric_with_tooltip(
            ui,
            "Worst error",
            &format!("{:.2}×", stats.worst),
            "How many times closer than its level's ring the block the pool stopped at sits. \
             Up to 1 every block has the level its distance asks for; over 1 the pool ran out \
             first and the ground near the camera is coarser than the Planet's LOD distance.",
        );
        metric(
            ui,
            "Waiting · in flight",
            &format!("{} · {}", stats.waiting, stats.in_flight),
        );
        metric(ui, "Meshed (total)", &stats.meshed.to_string());
        metric(ui, "Triangles", &stats.triangles.to_string());
        if stats.meshlets > 0 {
            let fill = stats.triangles as f64 / stats.meshlets as f64;
            metric_with_tooltip(
                ui,
                "Triangles / meshlet",
                &format!("{fill:.1} of 124"),
                "Mean over the meshlets on screen: how full the mesher packs them",
            );
        }
        match stats.gpu {
            Some(gpu) => {
                metric_with_tooltip(
                    ui,
                    "GPU per batch",
                    &format!("{} blocks", gpu.blocks),
                    "The last batch the terrain sent. A batch goes out at most once a frame, so \
                     this is also the most the terrain adds to one frame's GPU time.",
                );
                metric(ui, "  · generate", &format!("{:.3} ms", gpu.generate_ms));
                metric(ui, "  · count", &format!("{:.3} ms", gpu.mesh_ms));
                metric(ui, "  · write", &format!("{:.3} ms", gpu.write_ms));
                let mean = gpu.mean;
                metric_with_tooltip(
                    ui,
                    "GPU mean",
                    &format!("{:.1} blocks", mean.blocks),
                    &format!(
                        "Over the last {} batches: one batch swings with where the camera is.",
                        mean.batches
                    ),
                );
                metric(ui, "  · generate", &format!("{:.3} ms", mean.generate_ms));
                metric(ui, "  · count", &format!("{:.3} ms", mean.mesh_ms));
                metric(ui, "  · write", &format!("{:.3} ms", mean.write_ms));
            }
            None => metric(ui, "GPU per batch", "no timestamps"),
        }
    });
}

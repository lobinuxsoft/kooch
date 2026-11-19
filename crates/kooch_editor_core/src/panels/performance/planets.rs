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
        metric_with_tooltip(
            ui,
            "Worst error",
            &format!("{:.1} px", stats.worst_px),
            "How big one voxel looks on screen where the pool ran out before refining further. \
             Under a pixel or two the ground is as fine as the screen can show.",
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
                metric(ui, "  · mesh", &format!("{:.3} ms", gpu.mesh_ms));
            }
            None => metric(ui, "GPU per batch", "no timestamps"),
        }
    });
}

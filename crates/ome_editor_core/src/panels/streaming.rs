//! Streaming debug panel — runtime tuning of [`LodRingConfig`].
//!
//! Surfaces a single-ring radius slider on the global LOD ring resource
//! so artists / level designers can probe the streaming horizon without
//! a recompile. Writes go straight to the resource — the activation
//! system reacts on the next tick.
//!
//! Aspirational multi-ring configs (see `LodRingConfig::aspirational_planet_scale`)
//! are read-only here: surfacing N independent rings is out of scope for
//! the editor warmup. When a project opts in via a custom resource, the
//! slider becomes inert and the panel reports the active ring count.

use ome_world::lod::LodRingConfig;

/// Editor-comfortable bounds for the LOD-0 ring radius. Lower bound
/// keeps at least one chunk loaded around the focus; upper bound caps
/// at the current cache-gated activation's safe ceiling (~5³ cells per
/// recompute).
const MIN_RADIUS_M: f32 = 32.0;
const MAX_RADIUS_M: f32 = 4096.0;

/// Draws the Streaming tab content: single-ring radius slider + summary.
pub(crate) fn draw_streaming_content(ui: &mut egui::Ui, config: &mut LodRingConfig) {
    ui.heading("Streaming");
    ui.add_space(4.0);

    if config.rings.is_empty() {
        ui.colored_label(
            egui::Color32::YELLOW,
            "No LOD rings configured — activation is a no-op.",
        );
        return;
    }

    if config.rings.len() > 1 {
        ui.label(format!(
            "{} rings configured (multi-ring tuning UI pending).",
            config.rings.len()
        ));
        for ring in &config.rings {
            ui.label(format!(
                "  LOD {} → {:.0} m",
                ring.lod, ring.radius_meters
            ));
        }
        return;
    }

    let ring = &mut config.rings[0];
    ui.horizontal(|ui| {
        ui.label(format!("LOD {} radius", ring.lod));
        ui.add(
            egui::Slider::new(&mut ring.radius_meters, MIN_RADIUS_M..=MAX_RADIUS_M)
                .suffix(" m")
                .logarithmic(true),
        );
    });
    ui.label(format!(
        "Outer streaming horizon: {:.0} m",
        ring.radius_meters
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radius_bounds_cover_default() {
        let cfg = LodRingConfig::default();
        let r = cfg.rings[0].radius_meters;
        assert!(r >= MIN_RADIUS_M && r <= MAX_RADIUS_M);
    }
}

//! The shadow-page marking pass and what it found (#866).

use super::*;

/// The shadow-page marking pass and what it found (#866).
pub(super) fn shadow_page_readout(
    ui: &mut egui::Ui,
    page_counts: Option<kooch_render::shadow::pages::mark::MarkCounts>,
    raster_counts: Option<kooch_render::shadow::pages::raster::RasterCounts>,
) {
    use kooch_render::shadow::pages::PageConfig;

    let Some(counts) = page_counts else {
        ui.label(
            egui::RichText::new("waiting for the first readback")
                .small()
                .weak(),
        );
        return;
    };

    // 🔴 Every reading that says the frame is WRONG, before anything that says it is fine. Each of
    // these was already computed and each was invisible: a red line in the eleventh position of
    // eleven grey ones is not an alert, it is more text.
    let pool = counts.pool;
    if counts.overflow > 0 {
        alert(
            ui,
            &format!(
                "{} pages past the buffer — every number below is a floor",
                thousands(counts.overflow as u64)
            ),
            "The marking wrote more pages than the readback buffer holds, so the counts \
             below are truncated rather than wrong-by-a-little.",
        );
    }
    if pool.overflow > 0 {
        alert(
            ui,
            &format!(
                "{} pages unallocated — the pool is full",
                thousands(pool.overflow as u64)
            ),
            "Pages the frame needed and the pool could not give a slot to. They render \
             unshadowed. Epic's own pool overflow shows up as checkerboard corruption or \
             missing shadows, which is exactly the kind of failure nobody recognises by \
             sight — so it is named here instead.",
        );
    }
    if !pool.balanced() {
        alert(
            ui,
            &format!(
                "ledger does not close — {} resident + {} free of {}",
                pool.allocated(),
                pool.free,
                pool.capacity
            ),
            "Every slot of the slice is either held by a resident or sitting on the free \
             list. When the two do not add up, slots left the accounting without the \
             double-free counter firing, and requests the plan funded fail to allocate. \
             Only checked once the bump allocator has handed out the whole slice.",
        );
    }
    if pool.empty > 0 && pool.free > 0 {
        alert(
            ui,
            &format!(
                "{} pops found nothing on a list that is not empty",
                pool.empty
            ),
            "The contention case: the count and the array are two separate atomics, so a \
             popper that drives the count below zero makes another popper read the \
             underflow and give up on a list that still holds slots.",
        );
    }
    if pool.leaked > 0 {
        alert(
            ui,
            &format!(
                "{} slots fell out of the free list — a double free",
                pool.leaked
            ),
            "The free list cannot hold more slots than the slice has. Always zero, or the \
             allocator is wrong.",
        );
    }
    // 🔴 RED only for the sun, amber for the lamps, and the split is the reading rather than a
    // refinement of it.
    if let Some(raster) = raster_counts
        && raster.unfilled_sun > 0
    {
        alert(
            ui,
            &format!(
                "{} SUN pages have no geometry — the cull dropped what the marking asked for",
                thousands(raster.unfilled_sun as u64),
            ),
            "The expansion is dispatched as `pages * meshlets`, so a clipmap level holding \
             pages whose cull produced no survivors runs zero threads and emits no pairs. \
             The pages stay resident and still get cleared, and a cleared page stores 0 — \
             FAR under reversed-Z — so every reader over it answers that nothing occludes: \
             a bright patch, with the page present, allocated and correctly keyed.",
        );
    }
    if let Some(raster) = raster_counts
        && raster.unfilled > raster.unfilled_sun
    {
        warn(
            ui,
            &format!(
                "{} lamp pages cleared for nothing — lowest bucket {}",
                thousands((raster.unfilled - raster.unfilled_sun) as u64),
                raster.unfilled_first
            ),
            "Pages a local light made resident and its own cull then found no caster for. \
             Usually correct — the page exists because a receiver asked to be shadowed \
             there, and with nothing in reach to cast, lit IS the answer. It is counted \
             because the clear is paid every frame regardless.\n\nBuckets: the sun's \
             clipmap owns the first levels, then one per lamp.",
        );
    }
    if let Some(raster) = raster_counts
        && (raster.dropped > 0 || raster.overflow > 0)
    {
        alert(
            ui,
            &format!(
                "{} pages dropped · {} pairs past the list — shadows are missing",
                raster.dropped, raster.overflow
            ),
            "The raster could not draw everything the marking asked for, so some resident \
             pages hold no depth and shade as lit.",
        );
    }
    if pool.denied > 0 {
        warn(
            ui,
            &format!(
                "{} denied by rank — funded down to rank {}",
                thousands(pool.denied as u64),
                pool.cutoff
            ),
            "The frame wanted more pages than this view's slice holds, so the seating plan \
             (#942) ranked the demand and funded it coarsest-first: the sun's clipmap ahead \
             of every local light, and within any chain the coarse levels ahead of the fine. \
             What was denied is the finest detail, never a whole light's coverage — and the \
             number to shrink it is #943's resolution bias, not a bigger pool.",
        );
    }
    if pool.bias_local > 0 || pool.bias_sun > 0 {
        warn(
            ui,
            &format!(
                "asking coarser — locals +{} · sun +{} levels",
                pool.bias_local, pool.bias_sun
            ),
            "The demand did not fit the slice, so the marking asks coarser (#943): each \
             level is a quarter of the pages. Locals pay up to four levels before the sun \
             pays one, and it unwinds on its own when the demand shrinks. A bias that sits \
             high is the pool saying it is too small for the scene — raise \
             `shadow_pool_pages` or lower `shadow_density`.",
        );
    }

    // MiB, because pages are the unit and megabytes are the budget.
    let config = PageConfig::default();
    let mib = counts.resident as f64 * config.page_bytes() as f64 / (1024.0 * 1024.0);
    block(ui, "Atlas");
    grid(ui, "shadow_pages_atlas", |ui| {
        metric(ui, "view", &counts.view.to_string());
        metric_with_tooltip(
            ui,
            "resident",
            &format!("{} pages", thousands(counts.resident as u64)),
            "Distinct pages the frame would make resident, at 128-texel pages and \
             Depth32Float. Read it against Unreal's own pool, which is 4096 pages for the \
             WHOLE scene by default (6144 for open worlds, 8192 thrashes) — and against \
             this engine's 152 MiB of fixed shadow allocations, which stand whether or not \
             a light casts.",
        );
        metric(ui, "memory", &format!("{mib:.1} MiB"));
        metric_with_tooltip(
            ui,
            "viewport",
            &format!("{}x{}", counts.size.0, counts.size.1),
            "Part of the reading, not context: a page count without a resolution is not a \
             number, and the View and Game tabs are two cameras at two sizes.",
        );
    });

    block(ui, "Pool");
    grid(ui, "shadow_pages_pool", |ui| {
        metric_with_tooltip(
            ui,
            "slice used",
            &format!(
                "{} / {}  ({:.0}%)",
                thousands(pool.allocated() as u64),
                thousands(pool.capacity as u64),
                pool.load()
            ),
            "The pool is SLICED between the cameras — a layer of the atlas each — so this \
             is what THIS view may spend, not the whole budget: a camera cannot take \
             another camera's pages and cannot be robbed of its own. \
             `shadow_pool_pages` moves it.",
        );
        metric_with_tooltip(
            ui,
            "hit rate",
            &format!("{:.0}%", pool.hit_rate()),
            "The reading persistence exists to produce. A STILL camera should sit at 100%: \
             every page it wants is one it already has, so the raster draws nothing and the \
             atlas is last frame's. A page is freed when nothing has asked for it in \
             `shadow_page_seconds`, or the moment the seating plan stops funding its rank \
             under pressure (#942).",
        );
        metric(
            ui,
            "reused / new",
            &format!(
                "{} / {}",
                thousands(pool.reused as u64),
                thousands(pool.claims as u64)
            ),
        );
        metric(ui, "evicted", &thousands(pool.evicted as u64));
        if pool.preempted > 0 {
            metric_with_tooltip(
                ui,
                "preempted",
                &thousands(pool.preempted as u64),
                "Pages evicted by PRESSURE rather than by age: the plan did not fund their \
                 rank this frame. A camera that stopped moving should drive this to zero \
                 within a frame — persistent churn here means the demand is oscillating \
                 around the cutoff rank.",
            );
        }
        metric(
            ui,
            "demand / free",
            &format!("{} / {}", pool.demand, pool.free),
        );
        metric_with_tooltip(
            ui,
            "bump",
            &format!(
                "{} of {}",
                thousands(pool.high as u64),
                thousands(pool.capacity as u64)
            ),
            "How far the bump allocator has ever reached. It never goes down — a freed slot \
             returns to the free list — so once it reaches capacity every allocation must \
             come off that list.",
        );
        metric_with_tooltip(
            ui,
            "slots",
            &format!(
                "{} popped · {} bumped · {} back",
                pool.popped, pool.bumped, pool.pushed
            ),
            "Every take and every give-back of this frame, so a shortfall can be attributed \
             to an operation rather than inferred. A slot is taken either off the free list \
             or from the bump, and given back only to the free list.",
        );
    });

    block(ui, "Marking");
    grid(ui, "shadow_pages_marking", |ui| {
        // 🔴 The count is for EVERY light the grid holds, not the handful that have a shadow slot
        // today — and that is the measurement, not an oversight. Counting only the four that fit
        // today's slots would be measuring the cap the feature removes.
        metric_with_tooltip(
            ui,
            "samples",
            &thousands(counts.samples as u64),
            "The pass walks the froxel grid, which holds every light that reaches a pixel — \
             so these numbers are what the scene would cost with ALL of its lights casting.",
        );
        metric(ui, "light pairs", &thousands(counts.pairs as u64));
        metric_with_tooltip(
            ui,
            "distant tier",
            &if counts.distant > 0 {
                thousands(counts.distant as u64)
            } else {
                "0 — every light on a chain".to_owned()
            },
            "Lights that cast from ONE page per cube face rather than a chain (#1009). A light \
             qualifies when the finest level ANY pixel could ask it for is already the \
             coarsest it has — derived, no threshold — or when its whole range projects under \
             `shadow_min_pixels`. This used to be the number of lights casting nothing at all.",
        );
        metric_with_tooltip(
            ui,
            "gated by distance",
            &if counts.culled > 0 {
                thousands(counts.culled as u64)
            } else {
                "0 — every light casting".to_owned()
            },
            "Lights standing further than `shadow_page_light_reach` of their own ranges from \
             the camera (#944): they still shade, but they claim no pages.",
        );
        if counts.froxels > 0 && counts.samples > 0 {
            // 🔴 `pairs` counts a different thing on each path, so the ratio has to be read from the
            // side that owns it. Dividing froxel pairs by samples printed `0.0 lights each` and a
            // made-up multiplier beside it.
            let (lights_each, walked, other) = if counts.by_froxel {
                let each = counts.pairs as f32 / counts.froxels as f32;
                (each, counts.pairs as f32, counts.samples as f32 * each)
            } else {
                let each = counts.pairs as f32 / counts.samples as f32;
                (each, counts.pairs as f32, counts.froxels as f32 * each)
            };
            let ratio = (walked.max(1.0) / other.max(1.0)).max(other.max(1.0) / walked.max(1.0));
            metric(ui, "froxels occupied", &thousands(counts.froxels as u64));
            metric(ui, "lights each", &format!("{lights_each:.1}"));
            // 🔴 Olsson §III derives shadow resolution from cluster/light pairs rather than
            // sample/light pairs, because cluster bounds are "several orders of magnitude fewer
            // than the samples".
            metric_with_tooltip(
                ui,
                "walking",
                &format!(
                    "{} · {ratio:.0}x the other way",
                    if counts.by_froxel {
                        "per froxel"
                    } else {
                        "per pixel"
                    }
                ),
                "The marking runs per (pixel, light); the same walk over occupied froxels \
                 would run per (froxel, light), and this is the ratio between the two. It \
                 is an upper bound on the win: a froxel's bounds project to a RANGE of \
                 pages rather than one, so a cluster pass marks conservatively and spends \
                 pool slots the per-pixel version never asked for.",
            );
            // 🔴 Derived from this engine's own budget, not from folklore. On the OneXFly `shade:
            // compute` measured 5.5 ms at 17.9 lights per pixel — about 0.31 ms a light — against a
            // 13.9 ms frame.
            const OVERLAP_WARN: u32 = 16;
            if counts.peak_lights > 0 {
                let text = format!("{} lights", counts.peak_lights);
                let tip = "Point and spot lights whose ranges overlap all land in the same \
                           froxel, and every pixel of that froxel walks all of them — in \
                           the shading loop and again in the page marking. Overlap is \
                           invisible while authoring: lights are placed one at a time and \
                           the cell they share is not drawn anywhere.";
                if counts.peak_lights > OVERLAP_WARN {
                    metric_coloured(
                        ui,
                        "worst froxel",
                        &format!("{text} — overlapping"),
                        egui::Color32::from_rgb(240, 180, 60),
                        tip,
                    );
                } else {
                    metric_with_tooltip(ui, "worst froxel", &text, tip);
                }
            }
        }
    });

    let Some(raster) = raster_counts else {
        return;
    };
    block(ui, "Raster");
    grid(ui, "shadow_pages_raster", |ui| {
        // 🔴 What was actually DRAWN, against what was asked for. The
        // marking count above is a request; this is the answer, and the
        // two differing is the single most useful thing this panel says.
        metric_with_tooltip(
            ui,
            "rastered",
            &format!("{} pages", thousands(raster.pages as u64)),
            "The pages the depth raster actually filled. A still scene should raster near \
             zero; UE5's rule of thumb is under 5% of residents.",
        );
        metric_with_tooltip(
            ui,
            "cached",
            &thousands(raster.cached as u64),
            "Resident pages whose content survived from an earlier frame. They cost nothing.",
        );
        metric_with_tooltip(
            ui,
            "lamp survivors",
            &if raster.lamp_survivors > 0 {
                thousands(raster.lamp_survivors as u64)
            } else {
                "0 — no lamp can cast".to_owned()
            },
            "Meshlets the LAMPS' culls kept, over every bucket. Zero with lamp pages resident \
             means their pages are stamped empty and cleared, and a cleared page reads as \
             'nothing occludes' — every lamp stops casting with every other counter healthy. \
             Read it against `lamp pages cleared for nothing`: that one says pages were \
             cleared, this one says whether there was ever anything to put in them.",
        );
        metric(ui, "meshlet pairs", &thousands(raster.pairs as u64));
        // 🔴 Directly under the pair count and NOT at the foot of this section, because the section
        // does not fit the window: a reading placed after `scatter would cost` fell below the
        // panel's edge and an A/B was called on an absence nobody could observe.
        if raster.walk_overflow > 0 {
            alert(
                ui,
                &format!(
                    "descent overflow {}",
                    thousands(raster.walk_overflow as u64)
                ),
                "A descent ran out of stack and DROPPED a subtree — casters that stop \
                 being drawn into pages that asked for them, silently. The bound is \
                 3 x depth + 4 and the stack is larger than that for every page size the \
                 clipmap builds, so this reading means the page size changed.",
            );
        }
        metric_with_tooltip(
            ui,
            "geometry walk",
            &if raster.walk > 0 {
                let per_pair = raster.walk as f32 / raster.pairs.max(1) as f32;
                format!("{} · {per_pair:.0} per pair", thousands(raster.walk))
            } else {
                "off — pairing".to_owned()
            },
            "Pages the INVERTED expansion reached: one thread per surviving meshlet \
             descending the page pyramid to the pages it lands in, Unreal's arrangement. \
             Reads `off — pairing` when the other shape is running, so the line is there \
             either way and a toggle can be told from a no-op. Compare against `pair \
             tests`, which is what pairing costs for the same pairs — the emitted set is \
             identical, one shared function decides it, so a picture that changes with \
             the switch is a finding.",
        );
        if raster.local > 0 {
            metric_with_tooltip(
                ui,
                "local-light pages",
                &thousands(raster.local as u64),
                "Pages belonging to point and spot lights, rasterised this frame. They share \
                 the sun's buckets: a bucket is an OCTAVE of world texel size, so a lamp and \
                 the sun that want the same fineness draw from the same survivor list. \
                 ⚠️ They spend the same pool the sun does.",
            );
        }
        // 🔴 The number that decides the shape of the local-light raster. The expansion is a product
        // — a level's pages times a level's surviving meshlets — so what it costs is the
        // combinations it walks, not the pairs it finds.
        if raster.tests > 0 {
            let per_pair = raster.tests as f32 / raster.pairs.max(1) as f32;
            metric_with_tooltip(
                ui,
                "pair tests",
                &format!("{} · {per_pair:.0} per pair", thousands(raster.tests)),
                "The expansion asks, for every page of a level and every meshlet that \
                 survived that level's cull, whether the two touch. So its cost is pages \
                 TIMES meshlets, and the pairs it emits are what is left after the question \
                 is answered — the ratio is how much of the pass is spent proving a miss. \
                 ⚠️ It is also the number that decides whether local lights are affordable: \
                 they multiply the page side by roughly eighty, and the inverse form — \
                 asking which pages a meshlet touches — was measured WORSE for the sun, \
                 because a meshlet's rect covers up to 16384 cells at the finest clipmap \
                 levels while only twenty pages are resident there.",
            );
            metric_with_tooltip(
                ui,
                "worst level",
                &format!("{} at {}", raster.worst.0, thousands(raster.worst.1)),
                "The clipmap level whose expansion walked the most combinations. A level \
                 far above the others is the one to bias.",
            );
            // 🔴 The counted cost of the shape this pass does NOT use. Both numbers are measured
            // every frame so the choice between them is arithmetic instead of an opinion — which is
            // what was missing the last time one of them shipped everywhere at once.
            let save = raster.tests.saturating_sub(raster.hybrid);
            let cut = save as f32 / raster.tests.max(1) as f32 * 100.0;
            metric_with_tooltip(
                ui,
                "scatter would cost",
                &format!(
                    "{} · best {} ({cut:.0}% off)",
                    thousands(raster.scatter),
                    thousands(raster.hybrid)
                ),
                "There are two ways to find which meshlet belongs in which page. PAIRING \
                 walks every resident page against every survivor, which is what runs \
                 today. SCATTERING walks the cells each meshlet's bounds cover and looks \
                 them up, which is what the first number would cost — counted here without \
                 being run. Neither wins everywhere: a page at level 0 is centimetres wide \
                 so one meshlet covers thousands of cells against a handful of resident \
                 pages, while at level 12 a page is hundreds of metres and every meshlet \
                 lands in exactly one cell. The second number takes the cheaper shape at \
                 each level separately, and the percentage is the whole prize a hybrid has \
                 to offer.",
            );
        }
    });
    // The hash's two failure meters — tombstones walked and inserts out
    // of probes — are gone with the hash: the flat table has no probe
    // run to degrade. See `page_table.wgsl`.
}

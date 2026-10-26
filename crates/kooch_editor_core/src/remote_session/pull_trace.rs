//! Per presented frame: how many host frames the editor advanced, and the largest turn any mirrored
//! entity made (#1427). Written beside the host's `KOOCH_TRACE_FRAMING` file as `<path>.editor.csv`,
//! so one Settings line traces both ends.

use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use glam::{Mat4, Quat};
use kooch_remote::protocol::{EntityId, MovedTransform};

pub(super) struct PullTrace {
    file: std::io::BufWriter<std::fs::File>,
    start: Instant,
    last_frame: Option<Instant>,
    rotations: HashMap<EntityId, Quat>,
}

impl PullTrace {
    /// A trace when the project's launch environment asks the host for one.
    pub(super) fn from_launch_env(launch_env: &[(String, String)]) -> Option<Self> {
        let (_, path) = launch_env
            .iter()
            .find(|(key, _)| key == "KOOCH_TRACE_FRAMING")?;
        let path = format!("{path}.editor.csv");
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).ok()?);
        writeln!(file, "t,frame_ms,replies,moved,max_turn_deg").ok()?;
        tracing::info!(%path, "tracing the editor's pull");
        Some(Self {
            file,
            start: Instant::now(),
            last_frame: None,
            rotations: HashMap::new(),
        })
    }

    /// One presented frame: `replies` drained, `moved` the transforms they carried, oldest first.
    pub(super) fn frame(&mut self, replies: usize, moved: &[MovedTransform]) {
        let now = Instant::now();
        let frame_ms = self
            .last_frame
            .map_or(0.0, |last| (now - last).as_secs_f64() * 1000.0);
        self.last_frame = Some(now);

        // Only the last pose of each entity is shown, so the step is from the previous frame's.
        let mut latest: HashMap<EntityId, Quat> = HashMap::new();
        for transform in moved {
            let (_, rotation, _) =
                Mat4::from_cols_array(&transform.matrix).to_scale_rotation_translation();
            latest.insert(transform.id, rotation);
        }
        let mut max_turn = 0.0f32;
        for (id, rotation) in latest {
            if let Some(previous) = self.rotations.insert(id, rotation) {
                max_turn = max_turn.max(previous.angle_between(rotation).to_degrees());
            }
        }
        let t = self.start.elapsed().as_secs_f64();
        let _ = writeln!(
            self.file,
            "{t:.6},{frame_ms:.3},{replies},{},{max_turn:.5}",
            moved.len()
        )
        .and_then(|()| self.file.flush());
    }
}

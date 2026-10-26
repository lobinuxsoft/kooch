//! Per-frame CSV of the composer's arithmetic, behind `KOOCH_TRACE_FRAMING=<path>` (#1427). Five
//! hypotheses failed by reading; this names the quantity that oscillates.

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use glam::{Quat, Vec2, Vec3};

struct Sink {
    file: std::io::BufWriter<std::fs::File>,
    start: Instant,
}

fn sink() -> Option<&'static Mutex<Sink>> {
    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var_os("KOOCH_TRACE_FRAMING")?;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).ok()?);
        writeln!(
            file,
            "t,dt,tx,ty,tz,cx,cy,cz,at_x,at_y,owed_x,owed_y,share_x,share_y,alpha_x,alpha_y,turn_x,turn_y,yaw,pitch"
        )
        .ok()?;
        tracing::info!(path = %std::path::Path::new(&path).display(), "tracing the framing");
        Some(Mutex::new(Sink {
            file,
            start: Instant::now(),
        }))
    })
    .as_ref()
}

/// One composed frame. Inputs are what the composer read; `rotation` is what it wrote.
pub(super) struct Row {
    pub dt: f32,
    pub target: Vec3,
    pub camera: Vec3,
    pub at: Vec2,
    pub owed: Vec2,
    pub share: Vec2,
    pub alpha: Vec2,
    pub rotation: Quat,
}

pub(super) fn write(row: Row) {
    let Some(sink) = sink() else {
        return;
    };
    let Ok(mut sink) = sink.lock() else {
        return;
    };
    let t = sink.start.elapsed().as_secs_f64();
    let turn = row.owed * row.share * row.alpha;
    let (yaw, pitch, _) = row.rotation.to_euler(glam::EulerRot::YXZ);
    let (t3, c3) = (row.target, row.camera);
    // Flushed per row: the host is killed on Stop, and a buffer lost there is the run lost.
    let _ = writeln!(
        sink.file,
        "{t:.6},{:.6},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.4},{:.4},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5}",
        row.dt,
        t3.x, t3.y, t3.z,
        c3.x, c3.y, c3.z,
        row.at.x, row.at.y,
        row.owed.x, row.owed.y,
        row.share.x, row.share.y,
        row.alpha.x, row.alpha.y,
        turn.x, turn.y,
        yaw.to_degrees(),
        pitch.to_degrees(),
    )
    .and_then(|()| sink.file.flush());
}

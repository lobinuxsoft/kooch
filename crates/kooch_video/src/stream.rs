//! A WebM's AV1 track, frame by frame as RGBA8, each with when it shows from the start.

use std::collections::VecDeque;
use std::io::Cursor;
use std::time::Duration;

use matroska_demuxer::{Frame, MatroskaFile, TrackType};
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

use crate::av1::{Decoder, Planes};

/// Tile threads per video: enough for 1080p in real time, leaving the game its cores.
const THREADS: u32 = 4;

/// One frame, RGBA8 row by row.
pub struct Picture {
    pub rgba: Vec<u8>,
    pub size: (u32, u32),
    pub at: Duration,
}

pub(crate) struct Stream<B: AsRef<[u8]>> {
    file: MatroskaFile<Cursor<B>>,
    track: u64,
    /// Nanoseconds per timestamp tick.
    scale: u64,
    decoder: Decoder,
    frame: Frame,
    ready: VecDeque<Picture>,
    ended: bool,
}

impl<B: AsRef<[u8]>> Stream<B> {
    pub(crate) fn open(bytes: B) -> Result<Self, String> {
        let file = MatroskaFile::open(Cursor::new(bytes))
            .map_err(|error| format!("failed to read the WebM: {error}"))?;
        let track = file
            .tracks()
            .iter()
            .find(|track| track.track_type() == TrackType::Video && track.codec_id() == "V_AV1")
            .ok_or("the WebM has no AV1 video track")?
            .track_number()
            .get();
        let scale = file.info().timestamp_scale().get();
        Ok(Self {
            file,
            track,
            scale,
            decoder: Decoder::new(THREADS)?,
            frame: Frame::default(),
            ready: VecDeque::new(),
            ended: false,
        })
    }

    /// The next frame, or `None` past the last.
    pub(crate) fn next(&mut self) -> Result<Option<Picture>, String> {
        loop {
            if let Some(picture) = self.ready.pop_front() {
                return Ok(Some(picture));
            }
            if self.ended {
                return Ok(None);
            }
            let scale = self.scale;
            let ready = &mut self.ready;
            let mut show = |planes: Planes| convert(planes, scale).map(|p| ready.push_back(p));
            let more = self
                .file
                .next_frame(&mut self.frame)
                .map_err(|error| format!("failed to read the WebM: {error}"))?;
            if !more {
                self.ended = true;
                self.decoder.drain(&mut show)?;
            } else if self.frame.track == self.track {
                let at = i64::try_from(self.frame.timestamp).unwrap_or(i64::MAX);
                let unit = std::mem::take(&mut self.frame.data);
                self.decoder.decode(unit, at, &mut show)?;
            }
        }
    }
}

fn convert(planes: Planes, scale: u64) -> Result<Picture, String> {
    let (width, height) = planes.size;
    let mut rgba = vec![0; width as usize * height as usize * 4];
    let image = YuvPlanarImage {
        y_plane: &planes.y,
        y_stride: planes.y_stride,
        u_plane: &planes.u,
        u_stride: planes.uv_stride,
        v_plane: &planes.v,
        v_stride: planes.uv_stride,
        width,
        height,
    };
    let range = match planes.full_range {
        true => YuvRange::Full,
        false => YuvRange::Limited,
    };
    // BT.709: what an HD encode signals, and the import step encodes HD.
    yuv::yuv420_to_rgba(
        &image,
        &mut rgba,
        width * 4,
        range,
        YuvStandardMatrix::Bt709,
    )
    .map_err(|error| format!("failed to convert an AV1 picture: {error}"))?;
    let ticks = u64::try_from(planes.at).unwrap_or(0);
    Ok(Picture {
        rgba,
        size: planes.size,
        at: Duration::from_nanos(ticks.saturating_mul(scale)),
    })
}

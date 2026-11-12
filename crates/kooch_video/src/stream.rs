//! A WebM's AV1 track, frame by frame as 8-bit 4:2:0 planes, each with when it shows from the start.

use std::collections::VecDeque;
use std::io::Cursor;
use std::time::Duration;

use matroska_demuxer::{Frame, MatroskaFile, TrackType};
use yuv::{YuvPlanarImage, YuvRange, YuvStandardMatrix};

use crate::av1::{Decoder, Plane, Planes};

/// Tile threads per video: enough for 1080p in real time, leaving the game its cores.
const THREADS: u32 = 4;

/// One frame as the decoder left it: the GPU converts it, so it is never copied on the CPU.
pub struct Picture {
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
    /// Bytes per row of `y`, and of `u` and `v`.
    pub y_stride: u32,
    pub uv_stride: u32,
    pub size: (u32, u32),
    /// Full 0–255 levels rather than the 16–235 of video.
    pub full_range: bool,
    pub at: Duration,
}

impl Picture {
    /// The chroma planes' size: half the frame's, rounded up.
    pub fn chroma(&self) -> (u32, u32) {
        (self.size.0.div_ceil(2), self.size.1.div_ceil(2))
    }

    /// RGBA8 row by row, on the CPU: for the still image shown before playing.
    pub fn rgba(&self) -> Result<Vec<u8>, String> {
        let (width, height) = self.size;
        let mut rgba = vec![0; width as usize * height as usize * 4];
        let image = YuvPlanarImage {
            y_plane: &self.y,
            y_stride: self.y_stride,
            u_plane: &self.u,
            u_stride: self.uv_stride,
            v_plane: &self.v,
            v_stride: self.uv_stride,
            width,
            height,
        };
        let range = match self.full_range {
            true => YuvRange::Full,
            false => YuvRange::Limited,
        };
        // BT.709: what an HD encode signals, and what the GPU conversion assumes too.
        yuv::yuv420_to_rgba(
            &image,
            &mut rgba,
            width * 4,
            range,
            YuvStandardMatrix::Bt709,
        )
        .map_err(|error| format!("failed to convert an AV1 picture: {error}"))?;
        Ok(rgba)
    }
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
        let file = demux(bytes)?;
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

    /// Starts `bytes`, the same file, again from its first frame on the same decoder.
    /// 🔴 A new decoder per loop allocated nine 12.5 MB pictures each pass and freed the old ones on
    /// the render thread, which raised glibc's mmap threshold and fragmented the heap (#1891).
    pub(crate) fn rewind(&mut self, bytes: B) -> Result<(), String> {
        self.file = demux(bytes)?;
        self.decoder.flush();
        self.ready.clear();
        self.ended = false;
        Ok(())
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
            let mut show = |planes: Planes| {
                ready.push_back(timed(planes, scale));
                Ok(())
            };
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

fn demux<B: AsRef<[u8]>>(bytes: B) -> Result<MatroskaFile<Cursor<B>>, String> {
    MatroskaFile::open(Cursor::new(bytes))
        .map_err(|error| format!("failed to read the WebM: {error}"))
}

fn timed(planes: Planes, scale: u64) -> Picture {
    let ticks = u64::try_from(planes.at).unwrap_or(0);
    Picture {
        y: planes.y,
        u: planes.u,
        v: planes.v,
        y_stride: planes.y_stride,
        uv_stride: planes.uv_stride,
        size: planes.size,
        full_range: planes.full_range,
        at: Duration::from_nanos(ticks.saturating_mul(scale)),
    }
}

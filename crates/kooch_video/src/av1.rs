//! AV1 pictures from dav1d (#1793), built from source and linked statically: nothing to ship, and
//! its asm links into a project's `.so`, which rav1d's cannot (memorysafety/rav1d#1322).

use dav1d::pixel::YUVRange;
use dav1d::{
    BitsPerComponent, Decoder as Dav1d, Error, PixelLayout, PlanarImageComponent as Component,
    Plane, Settings,
};

/// Sends retried after the decoder said it was full; it never needs more than one.
const RETRIES: u32 = 8;

/// One 8-bit 4:2:0 picture, held from the decoder while it is converted.
pub(crate) struct Planes {
    pub y: Plane,
    pub u: Plane,
    pub v: Plane,
    pub y_stride: u32,
    pub uv_stride: u32,
    pub size: (u32, u32),
    pub full_range: bool,
    /// The temporal unit's timestamp it was sent with.
    pub at: i64,
}

pub(crate) struct Decoder(Dav1d);

impl Decoder {
    /// `threads` tiles at once, one frame in flight: pictures come out as their unit goes in.
    pub(crate) fn new(threads: u32) -> Result<Self, String> {
        let mut settings = Settings::new();
        settings.set_n_threads(threads.clamp(1, 16));
        settings.set_max_frame_delay(1);
        Dav1d::with_settings(&settings)
            .map(Self)
            .map_err(|error| format!("failed to open the AV1 decoder: {error}"))
    }

    /// Decodes one temporal unit, handing `show` every picture it completes.
    pub(crate) fn decode(
        &mut self,
        unit: Vec<u8>,
        at: i64,
        show: &mut impl FnMut(Planes) -> Result<(), String>,
    ) -> Result<(), String> {
        if unit.is_empty() {
            return Ok(());
        }
        let mut sent = self.0.send_data(unit, None, Some(at), None);
        for _ in 0..RETRIES {
            match sent {
                Ok(()) => break,
                Err(Error::Again) => {
                    self.drain(show)?;
                    sent = self.0.send_pending_data();
                }
                Err(error) => return Err(format!("failed to decode an AV1 unit: {error}")),
            }
        }
        self.drain(show)
    }

    /// Hands `show` every picture ready now.
    pub(crate) fn drain(
        &mut self,
        show: &mut impl FnMut(Planes) -> Result<(), String>,
    ) -> Result<(), String> {
        loop {
            match self.0.get_picture() {
                Ok(picture) => show(planes(&picture)?)?,
                Err(Error::Again) => return Ok(()),
                Err(error) => return Err(format!("failed to take an AV1 picture: {error}")),
            }
        }
    }
}

fn planes(picture: &dav1d::Picture) -> Result<Planes, String> {
    if picture.bits_per_component() != Some(BitsPerComponent(8))
        || picture.pixel_layout() != PixelLayout::I420
    {
        return Err(format!(
            "only 8-bit 4:2:0 AV1 plays, not {}-bit {:?}",
            picture.bit_depth(),
            picture.pixel_layout()
        ));
    }
    Ok(Planes {
        y: picture.plane(Component::Y),
        u: picture.plane(Component::U),
        v: picture.plane(Component::V),
        y_stride: picture.stride(Component::Y),
        uv_stride: picture.stride(Component::U),
        size: (picture.width(), picture.height()),
        full_range: picture.color_range() == YUVRange::Full,
        at: picture.timestamp().unwrap_or(0),
    })
}

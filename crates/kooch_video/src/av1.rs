//! AV1 pictures out of `rav1d`'s C API (#1793). The only `unsafe` on the video path lives here.

use std::mem::MaybeUninit;
use std::ptr::NonNull;

use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::DAV1D_PIXEL_LAYOUT_I420;
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

/// "Not now": the decoder wants a picture taken, or has none yet.
const AGAIN: i32 = -libc::EAGAIN;
/// Sends retried after the decoder said it was full; it never needs more than one.
const RETRIES: u32 = 8;

/// One 8-bit 4:2:0 picture, borrowed from the decoder while it is shown.
pub(crate) struct Planes<'a> {
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub y_stride: u32,
    pub uv_stride: u32,
    pub size: (u32, u32),
    pub full_range: bool,
    /// The temporal unit's timestamp it was sent with.
    pub at: i64,
}

pub(crate) struct Decoder(Dav1dContext);

// SAFETY: the context is an `Arc` of rav1d's own thread-safe state, used by one thread at a time.
unsafe impl Send for Decoder {}

impl Decoder {
    /// `threads` tiles at once, one frame in flight: pictures come out as their unit goes in.
    pub(crate) fn new(threads: u32) -> Result<Self, String> {
        let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
        // SAFETY: `dav1d_default_settings` writes the whole struct.
        let mut settings = unsafe {
            dav1d_default_settings(NonNull::new_unchecked(settings.as_mut_ptr()));
            settings.assume_init()
        };
        settings.n_threads = threads.clamp(1, 16) as i32;
        settings.max_frame_delay = 1;
        let mut context: Option<Dav1dContext> = None;
        // SAFETY: both pointers are to live locals of the right type.
        let opened = unsafe {
            dav1d_open(
                Some(NonNull::from(&mut context)),
                Some(NonNull::from(&mut settings)),
            )
        };
        match (opened.0, context) {
            (0, Some(context)) => Ok(Self(context)),
            (code, _) => Err(format!("failed to open the AV1 decoder: {code}")),
        }
    }

    /// Decodes one temporal unit, handing `show` every picture it completes.
    pub(crate) fn decode(
        &mut self,
        unit: &[u8],
        at: i64,
        show: &mut impl FnMut(Planes<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        if unit.is_empty() {
            return Ok(());
        }
        let mut data = Dav1dData::default();
        // SAFETY: `data` is a live local; the buffer returned holds `unit.len()` bytes.
        let buffer = unsafe { dav1d_data_create(Some(NonNull::from(&mut data)), unit.len()) };
        if buffer.is_null() {
            return Err("failed to allocate an AV1 unit".into());
        }
        // SAFETY: `buffer` was just allocated with `unit.len()` bytes and does not overlap `unit`.
        unsafe { std::ptr::copy_nonoverlapping(unit.as_ptr(), buffer, unit.len()) };
        data.m.timestamp = at;
        let mut result = Ok(());
        for _ in 0..RETRIES {
            // SAFETY: the context is open and `data` is a live, created unit.
            let sent = unsafe { dav1d_send_data(Some(self.0), Some(NonNull::from(&mut data))) };
            if sent.0 != 0 && sent.0 != AGAIN {
                result = Err(format!("failed to decode an AV1 unit: {}", sent.0));
                break;
            }
            result = self.drain(show);
            if result.is_err() || data.sz == 0 {
                break;
            }
        }
        // SAFETY: `data` came from `dav1d_data_create`; unref of a consumed unit is a no-op.
        unsafe { dav1d_data_unref(Some(NonNull::from(&mut data))) };
        result
    }

    /// Hands `show` every picture ready now.
    pub(crate) fn drain(
        &mut self,
        show: &mut impl FnMut(Planes<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        loop {
            let mut picture = Dav1dPicture::default();
            // SAFETY: the context is open and `picture` is a live local.
            let got = unsafe { dav1d_get_picture(Some(self.0), Some(NonNull::from(&mut picture))) };
            match got.0 {
                0 => {}
                AGAIN => return Ok(()),
                code => return Err(format!("failed to take an AV1 picture: {code}")),
            }
            let shown = planes(&picture).and_then(&mut *show);
            // SAFETY: `picture` was filled by `dav1d_get_picture` and is released once.
            unsafe { dav1d_picture_unref(Some(NonNull::from(&mut picture))) };
            shown?;
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let mut context = Some(self.0);
        // SAFETY: the context is open, and closed only here.
        unsafe { dav1d_close(Some(NonNull::from(&mut context))) };
    }
}

fn planes(picture: &Dav1dPicture) -> Result<Planes<'_>, String> {
    if picture.p.bpc != 8 || picture.p.layout != DAV1D_PIXEL_LAYOUT_I420 {
        return Err(format!(
            "only 8-bit 4:2:0 AV1 plays, not {}-bit layout {}",
            picture.p.bpc, picture.p.layout
        ));
    }
    let (width, height) = (picture.p.w.max(0) as usize, picture.p.h.max(0) as usize);
    let (y_stride, uv_stride) = (
        picture.stride[0].max(0) as usize,
        picture.stride[1].max(0) as usize,
    );
    let [Some(y), Some(u), Some(v)] = picture.data else {
        return Err("an AV1 picture without its planes".into());
    };
    // SAFETY: rav1d allocates each plane as `stride` bytes per row for every row of the picture
    // (chroma: half the rows, rounded up), alive until the picture is released.
    let plane = |data: NonNull<std::ffi::c_void>, len: usize| unsafe {
        std::slice::from_raw_parts(data.as_ptr().cast::<u8>().cast_const(), len)
    };
    // SAFETY: `seq_hdr` is alive as long as the picture.
    let full_range = picture
        .seq_hdr
        .is_some_and(|header| unsafe { header.as_ref() }.color_range != 0);
    Ok(Planes {
        y: plane(y, y_stride * height),
        u: plane(u, uv_stride * height.div_ceil(2)),
        v: plane(v, uv_stride * height.div_ceil(2)),
        y_stride: y_stride as u32,
        uv_stride: uv_stride as u32,
        size: (width as u32, height as u32),
        full_range,
        at: picture.m.timestamp,
    })
}

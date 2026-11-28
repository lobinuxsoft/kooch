//! The `.kfield` file: a 48-byte little-endian header, then the texels as `u16`.

use std::fmt;

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};

use super::{FIELD_DISTANCE, FIELD_HEIGHT, Field};

pub const FIELD_EXTENSION: &str = "kfield";

const MAGIC: &[u8; 4] = b"KFLD";
const VERSION: u32 = 1;
const HEADER: usize = 48;
/// A 4096² map, or a 256³ volume: past it a file is a mistake, not a shape.
pub(super) const MOST_TEXELS: u64 = 1 << 24;

impl Field {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.texels.len() * 2);
        out.extend_from_slice(MAGIC);
        for word in [VERSION, self.kind, self.size[0], self.size[1], self.size[2]] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        let floats = [
            self.extent[0],
            self.extent[1],
            self.extent[2],
            self.lo,
            self.hi,
            self.slope,
        ];
        for value in floats {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for texel in self.texels.iter() {
            out.extend_from_slice(&texel.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FieldParseError> {
        if bytes.get(..4) != Some(MAGIC.as_slice()) {
            return Err(FieldParseError::Magic);
        }
        let word = |at: usize| -> Result<[u8; 4], FieldParseError> {
            bytes
                .get(at..at + 4)
                .and_then(|b| b.try_into().ok())
                .ok_or(FieldParseError::Short)
        };
        let int = |at| word(at).map(u32::from_le_bytes);
        let float = |at| word(at).map(f32::from_le_bytes);
        if int(4)? != VERSION {
            return Err(FieldParseError::Version);
        }
        let kind = int(8)?;
        if kind != FIELD_HEIGHT && kind != FIELD_DISTANCE {
            return Err(FieldParseError::Kind(kind));
        }
        let size = [int(12)?, int(16)?, int(20)?];
        let count = size.iter().map(|&n| u64::from(n)).product::<u64>();
        if size.contains(&0) || count > MOST_TEXELS {
            return Err(FieldParseError::Size(size));
        }
        let extent = [float(24)?, float(28)?, float(32)?];
        let (lo, hi, slope) = (float(36)?, float(40)?, float(44)?);
        if ![lo, hi, slope].iter().chain(&extent).all(|v| v.is_finite()) {
            return Err(FieldParseError::NotFinite);
        }
        let data = bytes
            .get(HEADER..HEADER + count as usize * 2)
            .ok_or(FieldParseError::Short)?;
        let texels = data
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Ok(Self {
            kind,
            size,
            extent,
            lo,
            hi,
            slope,
            texels,
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FieldLoader;

impl AssetLoader<Field> for FieldLoader {
    fn extensions(&self) -> &[&'static str] {
        &[FIELD_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<Field> {
        Field::from_bytes(bytes).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

/// Why a `.kfield` did not load.
#[derive(Debug)]
pub enum FieldParseError {
    Magic,
    Version,
    Kind(u32),
    Size([u32; 3]),
    NotFinite,
    Short,
}

impl fmt::Display for FieldParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Magic => write!(f, "not a .kfield file"),
            Self::Version => write!(f, "a .kfield from another version"),
            Self::Kind(kind) => write!(f, "unknown field kind {kind}"),
            Self::Size(size) => write!(f, "a field of {size:?} texels is empty or too large"),
            Self::NotFinite => write!(f, "a field's header holds a NaN or an infinity"),
            Self::Short => write!(f, "the field ends before its texels do"),
        }
    }
}

impl std::error::Error for FieldParseError {}

kooch_core::register_asset!(Field, FieldLoader);

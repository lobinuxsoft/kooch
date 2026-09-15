//! A material's values for its shader's parameters (#1158), and how they pack for the GPU.

use std::collections::BTreeMap;

use kooch_core::Guid;
use serde::{Deserialize, Serialize};

use super::Material;
use super::shader::{
    MAX_PARAM_SCALARS, MAX_PARAM_TEXTURES, ParamKind, ShaderParam, TextureDefault,
};

/// One parameter's value, stored by name so a shader edit that reorders its params loses nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    /// Any numeric kind; components past its width are ignored.
    Number([f32; 4]),
    Texture(Option<Guid>),
}

/// A material's values by parameter name.
pub type ParamValues = BTreeMap<String, ParamValue>;

/// A texture a slot binds, and what it samples when none is assigned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureRef {
    pub guid: Option<Guid>,
    pub fallback: TextureDefault,
}

/// What one material slot uploads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackedParams {
    pub values: [f32; MAX_PARAM_SCALARS as usize],
    pub textures: [TextureRef; MAX_PARAM_TEXTURES as usize],
}

impl PackedParams {
    /// The engine's PBR surface: its three maps in their slots and no shader values.
    pub fn default_surface(material: &Material) -> Self {
        let map = |guid, fallback| TextureRef { guid, fallback };
        Self {
            values: [0.0; MAX_PARAM_SCALARS as usize],
            textures: [
                map(material.albedo, TextureDefault::White),
                map(material.normal, TextureDefault::Normal),
                map(material.metal_roughness, TextureDefault::White),
                TextureRef::default(),
            ],
        }
    }

    /// A custom shader: each declared parameter at its offset, the material's value or the default.
    pub fn for_shader(material: &Material, params: &[ShaderParam]) -> Self {
        let mut packed = Self {
            values: [0.0; MAX_PARAM_SCALARS as usize],
            textures: [TextureRef::default(); MAX_PARAM_TEXTURES as usize],
        };
        for param in params {
            let value = material.values.get(&param.name);
            if param.kind == ParamKind::Texture {
                let guid = match value {
                    Some(ParamValue::Texture(guid)) => *guid,
                    _ => None,
                };
                packed.textures[param.offset as usize] = TextureRef {
                    guid,
                    fallback: param.texture,
                };
                continue;
            }
            let number = match value {
                Some(ParamValue::Number(n)) => *n,
                _ => param.default,
            };
            let width = param.kind.width() as usize;
            let at = param.offset as usize;
            packed.values[at..at + width].copy_from_slice(&number[..width]);
        }
        packed
    }
}

/// Drops the values `params` does not declare with the same kind — what switching shader keeps.
pub fn retain_declared(values: &mut ParamValues, params: &[ShaderParam]) {
    values.retain(|name, value| {
        params.iter().any(|p| {
            p.name == *name
                && matches!(
                    (p.kind, *value),
                    (ParamKind::Texture, ParamValue::Texture(_))
                        | (
                            ParamKind::Float
                                | ParamKind::Vec2
                                | ParamKind::Vec3
                                | ParamKind::Vec4
                                | ParamKind::Color,
                            ParamValue::Number(_)
                        )
                )
        })
    });
}

#[cfg(test)]
mod tests;

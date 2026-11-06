//! A hand-written function called from a node: each wire narrowed to what it takes (#1807).

use egui_snarl::NodeId;

use super::Body;
use crate::shader_graph::FunctionPin;

/// `value`, of `width` components, as a wire.
pub(crate) fn widened(value: &str, width: u8) -> String {
    match width {
        1 => format!("vec4<f32>({value})"),
        2 => format!("vec4<f32>({value}, 0.0, 0.0)"),
        3 => format!("vec4<f32>({value}, 0.0)"),
        _ => value.to_owned(),
    }
}

impl Body<'_> {
    /// The call. Holds a struct when the function returns one, and a wire otherwise.
    pub(super) fn call(
        &mut self,
        id: NodeId,
        function: &str,
        inputs: &[FunctionPin],
        output: Option<u8>,
        reads: bool,
    ) -> Result<String, String> {
        let mut values: Vec<String> = reads.then(|| "input".to_owned()).into_iter().collect();
        for (input, pin) in inputs.iter().enumerate() {
            // Where every uv node starts: this pixel, until something else is wired.
            let fallback = match pin.name.as_str() {
                "uv" => "vec4<f32>(input.uv, 0.0, 0.0)".to_owned(),
                _ => super::vec4_literal(pin.default),
            };
            let value = self.input_or(id, input, &fallback)?;
            values.push(match pin.width {
                1 => format!("{value}.x"),
                2 => format!("{value}.xy"),
                3 => format!("{value}.xyz"),
                _ => value,
            });
        }
        let call = format!("{function}({})", values.join(", "));
        Ok(output.map_or_else(|| call.clone(), |width| widened(&call, width)))
    }
}

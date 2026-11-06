//! The Loop node: a subgraph's function called up to `rounds` times, carrying state (#1807).

use std::collections::HashSet;
use std::fmt::Write;

use egui_snarl::NodeId;

use super::Body;
use crate::shader_graph::subgraph::{INDEX, STOP, ident};

/// 🔴 Clamped: a wired count is whatever the graph computes, and a shader that never ends takes the
/// GPU down with it.
const MAX_ROUNDS: f32 = 256.0;

impl Body<'_> {
    /// Writes the loop and returns the struct holding its last round.
    pub(super) fn looped(
        &mut self,
        id: NodeId,
        function: &str,
        inputs: &[String],
        outputs: &[String],
        reads: bool,
    ) -> Result<String, String> {
        let rounds = self.input_or(id, 0, "vec4<f32>(8.0)")?;
        let pins: Vec<String> = inputs
            .iter()
            .map(|name| ident(name))
            .filter(|name| name != INDEX)
            .collect();
        let mut values = Vec::with_capacity(pins.len());
        for (pin, name) in pins.into_iter().enumerate() {
            let value = self.input_or(id, pin + 1, "vec4<f32>(0.0)")?;
            values.push((name, value));
        }
        let results: HashSet<String> = outputs.iter().map(|name| ident(name)).collect();

        let state = format!("l{}", self.locals);
        self.locals += 1;
        let _ = writeln!(self.lines, "    var {state}: {function}_out;");
        for (name, value) in values.iter().filter(|(name, _)| results.contains(name)) {
            let _ = writeln!(self.lines, "    {state}.out_{name} = {value};");
        }

        let round = format!("{state}_round");
        let mut arguments: Vec<String> = reads.then(|| "input".to_owned()).into_iter().collect();
        for name in inputs.iter().map(|name| ident(name)) {
            arguments.push(if name == INDEX {
                format!("vec4<f32>(f32({round}))")
            } else if results.contains(&name) {
                format!("{state}.out_{name}")
            } else {
                let value = values.iter().find(|(pin, _)| *pin == name);
                value.map_or_else(|| "vec4<f32>(0.0)".to_owned(), |(_, v)| v.clone())
            });
        }
        let _ = writeln!(
            self.lines,
            "    let {state}_rounds = u32(clamp({rounds}.x, 0.0, {MAX_ROUNDS:?}));\n    \
             for (var {round} = 0u; {round} < {state}_rounds; {round}++) {{\n        \
             {state} = {function}({});",
            arguments.join(", ")
        );
        if results.contains(STOP) {
            let _ = writeln!(
                self.lines,
                "        if {state}.out_{STOP}.x > 0.5 {{ break; }}"
            );
        }
        let _ = writeln!(self.lines, "    }}");
        Ok(state)
    }
}

//! Volume stamps (#1957): the `volume` shaders a terrain's stamps name, joined into its generator
//! as `fn volume_<i>`, with `fn stamp_volume` picking one by index. Adding a volume recompiles the
//! generator once; moving or copying its stamps does not.

use std::collections::HashMap;

/// What a generator joined with no volumes calls: nothing is inside.
pub(crate) const NO_VOLUMES: &str =
    "fn stamp_volume(index: u32, input: PlanetInput) -> f32 { return 1.0e30; }";

/// Whether a `volume` shader joins a bare generator and compiles, and if not, why.
pub fn validate_volume(volume: &str) -> Result<(), String> {
    crate::validate_generator(&join(crate::BARE_SPHERE, &[volume])?)
}

/// `generator` with each of `volumes` (a `volume` shader's `fn volume`) appended as `volume_<i>`. A
/// helper both define is kept once: the graph writes the same helper the same way. One name with two
/// bodies is an error, not a guess.
pub fn join(generator: &str, volumes: &[&str]) -> Result<String, String> {
    let mut held: HashMap<String, String> = items(generator)
        .into_iter()
        .map(|(name, text)| (name, normal(&text)))
        .collect();
    let mut out = generator.to_owned();
    let mut cases = String::new();
    for (index, volume) in volumes.iter().enumerate() {
        let mut found = false;
        for (name, text) in items(volume) {
            if name == "volume" {
                found = true;
                let renamed = text.replacen("fn volume(", &format!("fn volume_{index}("), 1);
                out.push('\n');
                out.push_str(&renamed);
                continue;
            }
            match held.get(&name) {
                Some(seen) if *seen == normal(&text) => {}
                Some(_) => {
                    return Err(format!(
                        "volume {index} defines `{name}` differently from the generator"
                    ));
                }
                None => {
                    held.insert(name, normal(&text));
                    out.push('\n');
                    out.push_str(&text);
                }
            }
        }
        if !found {
            return Err(format!("volume {index} has no `fn volume`"));
        }
        cases.push_str(&format!(
            "        case {index}u: {{ return volume_{index}(input); }}\n"
        ));
    }
    out.push_str(&format!(
        "\nfn stamp_volume(index: u32, input: PlanetInput) -> f32 {{\n    switch index {{\n{cases}        default: {{ return 1.0e30; }}\n    }}\n}}\n"
    ));
    Ok(out)
}

/// The top-level items of a WGSL source as `(name, text)`, comments between them dropped. An item
/// ends where its braces close, or at a `;` outside any.
fn items(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut text = String::new();
    let mut depth = 0i32;
    for line in source.lines() {
        let code = line.split("//").next().unwrap_or("");
        if depth == 0 && text.is_empty() && code.trim().is_empty() {
            continue;
        }
        text.push_str(line);
        text.push('\n');
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
        let closed = code.contains('}') || code.trim_end().ends_with(';');
        if depth <= 0 && closed {
            if let Some(name) = name_of(&text) {
                found.push((name, std::mem::take(&mut text)));
            }
            text.clear();
            depth = 0;
        }
    }
    found
}

/// What a top-level item declares: `fn x`, `struct x`, `const x`, `alias x`, `var<..> x`.
fn name_of(text: &str) -> Option<String> {
    let words = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let mut tokens = words.split_whitespace().skip_while(|t| t.starts_with('@'));
    let keyword = tokens.next()?;
    if !matches!(keyword, "fn" | "struct" | "const" | "alias" | "override")
        && !keyword.starts_with("var")
    {
        return None;
    }
    let rest = tokens.next()?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// An item's code with comments and spacing gone, so two copies of one helper compare equal.
fn normal(text: &str) -> String {
    text.lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A helper both sides wrote is kept once, or the joined module redefines it.
    #[test]
    fn a_shared_helper_joins_once() {
        let helper = "fn lib_sphere(p: vec3<f32>, r: f32) -> f32 {\n    return length(p) - r;\n}\n";
        let generator =
            format!("{helper}fn planet(input: PlanetInput) -> f32 {{ return input.altitude; }}\n");
        let volume = format!(
            "// kind: volume\n{helper}fn volume(input: PlanetInput) -> f32 {{\n    return lib_sphere(input.position, 2.0);\n}}\n"
        );
        let joined = join(&generator, &[&volume, &volume]).unwrap();
        assert_eq!(joined.matches("fn lib_sphere(").count(), 1);
        assert!(joined.contains("fn volume_1("));
        crate::validate_generator(&joined).unwrap();
    }
}

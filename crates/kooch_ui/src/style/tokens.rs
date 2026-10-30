//! Token-level rewrites of a declaration's value, before it is parsed.

use std::collections::HashMap;

use cssparser::{ParseError, Parser, ParserInput, Token};

/// Deep enough for a theme built on a theme; a cycle stops here instead of overflowing.
const MAX_DEPTH: u32 = 8;

/// `text` with every `var(--name)` replaced by its `:root` value.
pub(super) fn substitute(text: &str, vars: &HashMap<String, String>) -> Result<String, String> {
    substitute_at(text, vars, 0)
}

fn substitute_at(text: &str, vars: &HashMap<String, String>, depth: u32) -> Result<String, String> {
    if !text.contains("var(") {
        return Ok(text.to_owned());
    }
    if depth > MAX_DEPTH {
        return Err("var() nests too deep, or refers to itself".to_owned());
    }
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut out = String::with_capacity(text.len());
    write_tokens(&mut parser, vars, depth, &mut out)?;
    Ok(out)
}

fn write_tokens<'i>(
    parser: &mut Parser<'i, '_>,
    vars: &HashMap<String, String>,
    depth: u32,
    out: &mut String,
) -> Result<(), String> {
    loop {
        let start = parser.position();
        let Ok(token) = parser.next_including_whitespace().cloned() else {
            return Ok(());
        };
        match token {
            Token::Function(name) if name.eq_ignore_ascii_case("var") => {
                let var: Result<String, ParseError<'i, ()>> =
                    parser.parse_nested_block(|inner| Ok(inner.expect_ident()?.to_string()));
                let var = var.map_err(|_| "var() takes one --name".to_owned())?;
                let value = vars
                    .get(&var)
                    .ok_or_else(|| format!("`{var}` is not set in :root"))?;
                out.push_str(&substitute_at(value, vars, depth + 1)?);
            }
            Token::Function(name) => {
                out.push_str(&name);
                out.push('(');
                let inner: Result<(), ParseError<'i, String>> =
                    parser.parse_nested_block(|inner| {
                        write_tokens(inner, vars, depth, out).map_err(|m| inner.new_custom_error(m))
                    });
                inner.map_err(message)?;
                out.push(')');
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                skip_block(parser);
                out.push_str(parser.slice_from(start));
            }
            _ => out.push_str(parser.slice_from(start)),
        }
    }
}

/// The space-separated parts of a value; a function call is one part.
pub(super) fn components(text: &str) -> Vec<String> {
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut parts = Vec::new();
    loop {
        let start = parser.position();
        let Ok(token) = parser.next().cloned() else {
            return parts;
        };
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            skip_block(&mut parser);
        }
        parts.push(parser.slice_from(start).trim().to_owned());
    }
}

fn skip_block(parser: &mut Parser<'_, '_>) {
    let _: Result<(), ParseError<'_, ()>> = parser.parse_nested_block(|inner| {
        while inner.next().is_ok() {}
        Ok(())
    });
}

pub(super) fn message(error: ParseError<'_, String>) -> String {
    use cssparser::{BasicParseErrorKind, ParseErrorKind, ToCss};
    match error.kind {
        ParseErrorKind::Custom(message) => message,
        ParseErrorKind::Basic(BasicParseErrorKind::UnexpectedToken(token)) => {
            format!("unexpected `{}`", token.to_css_string())
        }
        ParseErrorKind::Basic(BasicParseErrorKind::EndOfInput) => "unexpected end".to_owned(),
        ParseErrorKind::Basic(BasicParseErrorKind::AtRuleInvalid(name)) => {
            format!("`@{name}` is not supported")
        }
        ParseErrorKind::Basic(_) => "invalid rule".to_owned(),
    }
}

//! The rule grammar: selectors before a block, declarations inside it.

use std::collections::HashMap;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, SourceLocation, StyleSheetParser,
    Token,
};

use super::declaration::{self, Declaration};
use super::tokens::{message, substitute};

pub const STATE_HOVER: u8 = 1;
pub const STATE_ACTIVE: u8 = 1 << 1;
pub const STATE_FOCUS: u8 = 1 << 2;
pub const STATE_CHECKED: u8 = 1 << 3;
pub const STATE_DISABLED: u8 = 1 << 4;
/// `:root`: the document's root node, and where `--variables` live.
pub const STATE_ROOT: u8 = 1 << 5;

const ELEMENTS: [&str; 7] = ["ui", "panel", "label", "button", "toggle", "slider", "use"];

/// What one node must be, written without spaces: `button.menu-button:hover`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compound {
    pub element: Option<Box<str>>,
    pub name: Option<Box<str>>,
    pub classes: Vec<Box<str>>,
    /// `STATE_*` bits.
    pub states: u8,
}

impl Compound {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Compounds left to right, each a descendant of the one before; the last is the node styled.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector(pub Vec<Compound>);

#[derive(Debug, Clone, PartialEq)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// 1-based, for the warnings of later phases.
    pub line: u32,
}

/// What was skipped and where. The rest of the sheet still applies, as in CSS.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleWarning {
    /// 1-based.
    pub line: u32,
    pub column: u32,
    pub message: String,
}

/// A parsed `.kcss`, rules in source order.
#[derive(Debug, Clone, Default)]
pub struct UiStyleSheet {
    pub rules: Vec<StyleRule>,
    pub warnings: Vec<StyleWarning>,
}

// SAFETY: as for `UiTree`: taffy's lengths are `!Send` only for a `calc()` pointer, and taffy is
// built without `calc`, so no declaration holds one.
unsafe impl Send for UiStyleSheet {}
unsafe impl Sync for UiStyleSheet {}

impl UiStyleSheet {
    /// Gives every `url()` the asset it names; the loader knows where the sheet lives, the parser
    /// does not.
    pub fn resolve_images(&mut self, mut guid_of: impl FnMut(&str) -> Option<kooch_core::Guid>) {
        for declaration in self
            .rules
            .iter_mut()
            .flat_map(|rule| &mut rule.declarations)
        {
            if let Declaration::BackgroundImage(Some(image)) = declaration {
                image.guid = guid_of(&image.url);
            }
        }
    }

    pub fn parse(source: &str) -> Self {
        // Two passes: a `var()` may come before the `:root` that sets it.
        let vars = variables(source);
        let mut sheet = Self::default();
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rules = Rules {
            vars: &vars,
            warnings: Vec::new(),
        };
        let mut skipped = Vec::new();
        for item in StyleSheetParser::new(&mut parser, &mut rules) {
            match item {
                Ok(rule) if !rule.declarations.is_empty() => sheet.rules.push(rule),
                Ok(_) => {}
                Err((error, _)) => skipped.push(warning(error)),
            }
        }
        sheet.warnings = rules.warnings;
        sheet.warnings.extend(skipped);
        sheet.warnings.sort_by_key(|w| (w.line, w.column));
        sheet
    }
}

fn warning(error: ParseError<'_, String>) -> StyleWarning {
    let SourceLocation { line, column } = error.location;
    StyleWarning {
        line: line + 1,
        column,
        message: message(error),
    }
}

struct Rules<'a> {
    vars: &'a HashMap<String, String>,
    warnings: Vec<StyleWarning>,
}

impl<'i> QualifiedRuleParser<'i> for Rules<'_> {
    type Prelude = Vec<Selector>;
    type QualifiedRule = StyleRule;
    type Error = String;

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Vec<Selector>, ParseError<'i, String>> {
        input.parse_comma_separated(selector)
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Vec<Selector>,
        start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<StyleRule, ParseError<'i, String>> {
        let mut body = Body { vars: self.vars };
        let mut declarations = Vec::new();
        for item in RuleBodyParser::new(input, &mut body) {
            match item {
                Ok(parsed) => declarations.extend(parsed),
                Err((error, _)) => self.warnings.push(warning(error)),
            }
        }
        Ok(StyleRule {
            selectors,
            declarations,
            line: start.source_location().line + 1,
        })
    }
}

impl<'i> AtRuleParser<'i> for Rules<'_> {
    type Prelude = ();
    type AtRule = StyleRule;
    type Error = String;
}

fn selector<'i>(input: &mut Parser<'i, '_>) -> Result<Selector, ParseError<'i, String>> {
    let mut compounds = vec![Compound::default()];
    let mut spaced = false;
    while let Ok(token) = input.next_including_whitespace().cloned() {
        if matches!(token, Token::WhiteSpace(_)) {
            spaced = true;
            continue;
        }
        if std::mem::take(&mut spaced) && !compounds.last().is_some_and(Compound::is_empty) {
            compounds.push(Compound::default());
        }
        let Some(last) = compounds.last_mut() else {
            break;
        };
        match token {
            Token::Ident(element) if last.is_empty() => {
                if !ELEMENTS.contains(&element.as_ref()) {
                    return Err(input.new_custom_error(format!("`{element}` is not an element")));
                }
                last.element = Some(element.as_ref().into());
            }
            Token::IDHash(name) => last.name = Some(name.as_ref().into()),
            Token::Delim('.') => last.classes.push(input.expect_ident()?.as_ref().into()),
            Token::Colon => {
                let state = input.expect_ident()?.clone();
                last.states |= state_bit(&state).ok_or_else(|| {
                    input.new_custom_error(format!("`:{state}` is not a supported state"))
                })?;
            }
            token => return Err(input.new_unexpected_token_error(token)),
        }
    }
    if compounds.last().is_none_or(Compound::is_empty) {
        return Err(input.new_custom_error("empty selector".to_owned()));
    }
    Ok(Selector(compounds))
}

fn state_bit(state: &str) -> Option<u8> {
    Some(match state {
        "hover" => STATE_HOVER,
        "active" => STATE_ACTIVE,
        "focus" => STATE_FOCUS,
        "checked" => STATE_CHECKED,
        "disabled" => STATE_DISABLED,
        "root" => STATE_ROOT,
        _ => return None,
    })
}

/// The declarations of one block.
struct Body<'a> {
    vars: &'a HashMap<String, String>,
}

impl<'i> DeclarationParser<'i> for Body<'_> {
    type Declaration = Vec<Declaration>;
    type Error = String;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<Vec<Declaration>, ParseError<'i, String>> {
        let start = input.position();
        while input.next().is_ok() {}
        // Read by `variables` in the first pass.
        if name.starts_with("--") {
            return Ok(Vec::new());
        }
        let text = input.slice_from(start).trim();
        let text = substitute(text, self.vars).map_err(|m| input.new_custom_error(m))?;
        declaration::parse(&name.to_ascii_lowercase(), &text).map_err(|m| input.new_custom_error(m))
    }
}

impl<'i> QualifiedRuleParser<'i> for Body<'_> {
    type Prelude = ();
    type QualifiedRule = Vec<Declaration>;
    type Error = String;
}

impl<'i> AtRuleParser<'i> for Body<'_> {
    type Prelude = ();
    type AtRule = Vec<Declaration>;
    type Error = String;
}

impl<'i> RuleBodyItemParser<'i, Vec<Declaration>, String> for Body<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// `--name: value` from every `:root` rule, the value as written.
fn variables(source: &str) -> HashMap<String, String> {
    let mut vars = Variables::default();
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    for _ in StyleSheetParser::new(&mut parser, &mut vars) {}
    vars.0
}

#[derive(Default)]
struct Variables(HashMap<String, String>);

impl<'i> QualifiedRuleParser<'i> for Variables {
    type Prelude = bool;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<bool, ParseError<'i, ()>> {
        let root = input
            .try_parse(|input| -> Result<(), ParseError<'i, ()>> {
                input.expect_colon()?;
                input.expect_ident_matching("root")?;
                Ok(input.expect_exhausted()?)
            })
            .is_ok();
        while input.next().is_ok() {}
        Ok(root)
    }

    fn parse_block<'t>(
        &mut self,
        root: bool,
        _start: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<(), ParseError<'i, ()>> {
        if root {
            for _ in RuleBodyParser::new(input, &mut VariableBody(&mut self.0)) {}
        }
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Variables {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

struct VariableBody<'a>(&'a mut HashMap<String, String>);

impl<'i> DeclarationParser<'i> for VariableBody<'_> {
    type Declaration = ();
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _start: &ParserState,
    ) -> Result<(), ParseError<'i, ()>> {
        let start = input.position();
        while input.next().is_ok() {}
        if name.starts_with("--") {
            self.0
                .insert(name.to_string(), input.slice_from(start).trim().to_owned());
        }
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for VariableBody<'_> {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

impl<'i> AtRuleParser<'i> for VariableBody<'_> {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, (), ()> for VariableBody<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

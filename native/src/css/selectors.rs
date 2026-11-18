//! Classify selector branches, preserving class/id/attribute boundaries and
//! selector-list pseudos. Root modifiers such as `body.dark` still affect the
//! document; `.panel *` and `body .panel` are component selectors.

use super::{Analyzer, Error};
use cssparser::{ParseError, Parser, Token};

#[derive(Default)]
pub(super) struct Scope {
    pub global: bool,
    root: bool,
}

pub(super) fn classify(
    input: &mut Parser<'_>,
    analyzer: &mut Analyzer<'_>,
    depth: usize,
) -> Result<Scope, Error> {
    let mut result = Scope::default();
    let mut root = false;
    let mut restricted = false;
    let mut anchored = false;
    let mut pseudo = false;
    let mut class = false;
    let mut seen = false;
    let mut whitespace = false;
    while !input.is_exhausted() {
        analyzer.step(input, depth)?;
        let token = input.next_including_whitespace_and_comments()?.clone();
        if matches!(token, Token::Comment(_)) {
            continue;
        }
        if matches!(token, Token::WhiteSpace(_)) {
            whitespace = true;
            continue;
        }
        // Spaces before a comma do not start a descendant compound.
        if whitespace && !matches!(token, Token::Comma) {
            anchored |= restricted && !root;
            root = false;
            restricted = false;
            pseudo = false;
            class = false;
        }
        whitespace = false;
        match token {
            Token::Delim('>' | '+' | '~') => {
                anchored |= restricted && !root;
                root = false;
                restricted = false;
                pseudo = false;
                class = false;
                continue;
            }
            Token::Comma => {
                result.global |= seen && !anchored && (!restricted || root);
                result.root |= seen && !anchored && root;
                root = false;
                restricted = false;
                anchored = false;
                pseudo = false;
                class = false;
                seen = false;
                continue;
            }
            Token::Delim('.') => {
                restricted = true;
                class = true;
            }
            Token::IDHash(_) | Token::Hash(_) => restricted = true,
            Token::SquareBracketBlock => {
                restricted = true;
                input.parse_nested_block(|input| analyzer.consume(input, depth + 1, true))?;
            }
            Token::Colon => pseudo = true,
            Token::Ident(name) => {
                if !class
                    && ((!pseudo
                        && (name.eq_ignore_ascii_case("html")
                            || name.eq_ignore_ascii_case("body")))
                        || (pseudo && name.eq_ignore_ascii_case("root")))
                {
                    root = true;
                }
                if pseudo
                    && (name.eq_ignore_ascii_case("scope") || name.eq_ignore_ascii_case("host"))
                {
                    restricted = true;
                }
                class = false;
                pseudo = false;
            }
            Token::Function(name) => {
                if pseudo
                    && (name.eq_ignore_ascii_case("is")
                        || name.eq_ignore_ascii_case("where")
                        || name.eq_ignore_ascii_case("matches"))
                {
                    let nested =
                        input.parse_nested_block(|input| classify(input, analyzer, depth + 1))?;
                    restricted |= !nested.global;
                    root |= nested.root;
                } else {
                    if name.eq_ignore_ascii_case("host")
                        || name.eq_ignore_ascii_case("host-context")
                    {
                        restricted = true;
                    }
                    input.parse_nested_block(|input| analyzer.consume(input, depth + 1, true))?;
                }
                pseudo = false;
            }
            token if token.is_parse_error() => return Err(ParseError::unexpected_token()),
            _ => {}
        }
        seen = true;
    }
    result.global |= seen && !anchored && (!restricted || root);
    result.root |= seen && !anchored && root;
    Ok(result)
}

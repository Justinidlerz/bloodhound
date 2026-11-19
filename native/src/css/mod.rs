//! Stylesheet analysis. Only declarations with a document-wide selector are
//! findings; component variables, animation uses and named at-rules are not.

mod selectors;

use crate::model::{Analysis, Confidence, Dependency, DependencyKind, Finding, Gap, Rule, Span};
use crate::scheduler::Cancellation;
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, Token,
};

type Error = ParseError<&'static str>;

pub fn analyze(source: &str, max_steps: usize, cancel: &Cancellation) -> Analysis {
    let mut analyzer = Analyzer {
        source,
        out: Analysis::default(),
        remaining: max_steps,
        stopped: false,
        cancel,
    };
    analyzer.walk(&mut Parser::new(source), false, true, 0);
    analyzer.out
}

struct Analyzer<'a> {
    source: &'a str,
    out: Analysis,
    remaining: usize,
    stopped: bool,
    cancel: &'a Cancellation,
}

impl Analyzer<'_> {
    fn step(&mut self, input: &Parser<'_>, depth: usize) -> Result<(), Error> {
        let reason = self.cancel.check().err().or_else(|| {
            if depth > 128 {
                Some("css-depth-budget")
            } else if self.remaining == 0 {
                Some("css-step-budget")
            } else {
                None
            }
        });
        if let Some(reason) = reason {
            if !self.stopped {
                self.gap(
                    reason,
                    input.position().byte_index(),
                    input.position().byte_index(),
                );
            }
            self.stopped = true;
            return Err(ParseError::custom(reason));
        }
        self.remaining -= 1;
        Ok(())
    }

    fn gap(&mut self, reason: &str, start: usize, end: usize) {
        self.out.gaps.push(Gap {
            reason: reason.into(),
            span: Span::new(start as u32, end as u32),
            detail: None,
        });
    }

    fn consume(
        &mut self,
        input: &mut Parser<'_>,
        depth: usize,
        allow_block: bool,
    ) -> Result<(), Error> {
        while !input.is_exhausted() {
            self.step(input, depth)?;
            match input.next()?.clone() {
                Token::CurlyBracketBlock if !allow_block => {
                    return Err(ParseError::unexpected_token());
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    input.parse_nested_block(|input| self.consume(input, depth + 1, true))?;
                }
                token if token.is_parse_error() => return Err(ParseError::unexpected_token()),
                _ => {}
            }
        }
        Ok(())
    }

    fn walk(
        &mut self,
        input: &mut Parser<'_>,
        declarations: bool,
        global: bool,
        depth: usize,
    ) -> bool {
        let mut rules = Rules {
            analyzer: self,
            declarations,
            global,
            depth,
        };
        let mut parser = RuleBodyParser::new(input, &mut rules);
        let mut has_declarations = false;
        while !parser.input.is_exhausted() && !parser.parser.analyzer.stopped {
            let start = parser.input.position().byte_index();
            if parser.parser.analyzer.step(parser.input, depth).is_err() {
                break;
            }
            match parser.next() {
                Some(Ok(declaration)) => has_declarations |= declaration,
                Some(Err(_)) if !parser.parser.analyzer.stopped => {
                    parser.parser.analyzer.gap(
                        "css-parse-error",
                        start,
                        parser.input.position().byte_index(),
                    );
                }
                _ => {}
            }
        }
        has_declarations
    }
}

struct Rules<'a, 'b> {
    analyzer: &'a mut Analyzer<'b>,
    declarations: bool,
    global: bool,
    depth: usize,
}

struct Selector<'i> {
    text: &'i str,
    global: bool,
}

impl<'i> QualifiedRuleParser<'i> for Rules<'_, '_> {
    type Prelude = Selector<'i>;
    type QualifiedRule = bool;
    type Error = &'static str;

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<Selector<'i>, Error> {
        let start = input.position();
        let global = selectors::classify(input, self.analyzer, self.depth)?.global;
        Ok(Selector {
            text: input.slice_from(start).trim(),
            global: self.global && global,
        })
    }

    fn parse_block(
        &mut self,
        selector: Selector<'i>,
        start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<bool, Error> {
        let declarations = self
            .analyzer
            .walk(input, true, selector.global, self.depth + 1);
        if declarations && selector.global {
            input.skip_whitespace();
            let end = input.position().byte_index();
            let closed = self.analyzer.source.as_bytes().get(end) == Some(&b'}');
            self.analyzer.out.findings.push(Finding {
                rule: Rule::CssGlobalStyle,
                span: Span::new(
                    start.position().byte_index() as u32,
                    (end + usize::from(closed)) as u32,
                ),
                target: selector.text.into(),
                operation: "style".into(),
                confidence: Confidence::High,
                detail: None,
                polyfill: None,
                declaration: None,
            });
        }
        // Nested selectors own their declarations; do not attribute them to the parent.
        Ok(false)
    }
}

struct AtRule {
    name: String,
    import: Option<String>,
    span: Span,
    scope_global: bool,
}

impl<'i> AtRuleParser<'i> for Rules<'_, '_> {
    type Prelude = AtRule;
    type AtRule = bool;
    type Error = &'static str;

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<AtRule, Error> {
        let start = input.position();
        let name = name.to_ascii_lowercase();
        let import = if name == "import" {
            Some(input.expect_url_or_string()?.to_string())
        } else {
            None
        };
        let scope_global = if name == "scope" {
            if input
                .try_parse(|input| input.expect_parenthesis_block())
                .is_ok()
            {
                input
                    .parse_nested_block(|input| {
                        selectors::classify(input, self.analyzer, self.depth + 1)
                    })?
                    .global
            } else {
                false
            }
        } else {
            true
        };
        self.analyzer.consume(input, self.depth, true)?;
        Ok(AtRule {
            name,
            import,
            scope_global,
            span: Span::new(
                start.byte_index() as u32,
                input.position().byte_index() as u32,
            ),
        })
    }

    fn rule_without_block(&mut self, rule: AtRule, _: &ParserState) -> Result<bool, ()> {
        if let Some(url) = rule.import {
            self.analyzer.out.dependencies.push(Dependency {
                kind: DependencyKind::CssImport,
                span: rule.span,
                specifiers: vec![url],
                content: None,
                content_offset: None,
                script_mode: None,
                base_url: None,
            });
        }
        Ok(false)
    }

    fn parse_block(
        &mut self,
        rule: AtRule,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<bool, Error> {
        // These grouping rules contain selectors (and, when nested, declarations).
        // Keyframes, font descriptors and property registrations do not.
        if matches!(
            rule.name.as_str(),
            "media" | "supports" | "layer" | "container" | "scope" | "starting-style"
        ) {
            let global = self.global && rule.scope_global;
            let declarations = self
                .analyzer
                .walk(input, self.declarations, global, self.depth + 1);
            Ok(declarations && global)
        } else {
            self.analyzer.consume(input, self.depth + 1, true)?;
            Ok(false)
        }
    }
}

impl<'i> DeclarationParser<'i> for Rules<'_, '_> {
    type Declaration = bool;
    type Error = &'static str;

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Result<bool, Error> {
        let nonempty = !input.is_exhausted();
        // A block in a non-custom value is a nested selector (e.g. a:hover),
        // not a declaration. Let RuleBodyParser retry it as a qualified rule.
        self.analyzer
            .consume(input, self.depth, name.starts_with("--"))?;
        Ok(nonempty)
    }
}

impl<'i> RuleBodyItemParser<'i, bool, &'static str> for Rules<'_, '_> {
    fn parse_declarations(&self) -> bool {
        self.declarations
    }
    fn parse_qualified(&self) -> bool {
        true
    }
}

/// Read only actual top-level CSS comments, never strings or declaration values.
pub fn source_map_reference(source: &str) -> Option<&str> {
    let mut parser = Parser::new(source);
    let mut reference = None;
    while let Ok(token) = parser.next_including_whitespace_and_comments() {
        if let Token::Comment(comment) = token {
            if let Some(value) = comment
                .trim()
                .strip_prefix('#')
                .or_else(|| comment.trim().strip_prefix('@'))
            {
                if let Some(value) = value.trim_start().strip_prefix("sourceMappingURL=") {
                    reference = value.split_whitespace().next();
                }
            }
        }
    }
    reference
}

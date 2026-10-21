use super::*;

pub fn css(source: &str, max_steps: usize) -> Value {
    let mut out = raw();
    let mut parser = Parser::new(source);
    let mut steps = 0;
    fn walk<'i>(
        p: &mut Parser<'i>,
        out: &mut Value,
        declarations: bool,
        steps: &mut usize,
        max: usize,
        depth: usize,
    ) -> Result<(), cssparser::ParseError<()>> {
        if depth > 128 {
            gap(
                out,
                p.position().byte_index(),
                p.position().byte_index(),
                "css-depth-budget",
                "css",
            );
            return Ok(());
        }
        let mut start = p.position();
        let mut at_rule = String::new();
        let mut property = String::new();
        let mut value = false;
        let mut global = false;
        let mut class_name = false;
        let mut pseudo = false;
        let mut import = false;
        while !p.is_exhausted() {
            *steps += 1;
            if *steps > max {
                gap(
                    out,
                    p.position().byte_index(),
                    p.position().byte_index(),
                    "css-step-budget",
                    "css",
                );
                return Ok(());
            }
            let before = p.position().byte_index();
            let token = p.next_including_whitespace_and_comments()?.clone();
            let after = p.position().byte_index();
            match token {
                CssToken::AtKeyword(n) => {
                    at_rule = n.to_string().to_ascii_lowercase();
                    import = at_rule == "import";
                }
                CssToken::Ident(n) => {
                    if declarations && !value && property.is_empty() {
                        property = n.to_string();
                    }
                    if !declarations
                        && !class_name
                        && ((!pseudo && ["html", "body"].contains(&n.as_ref()))
                            || (pseudo && n.as_ref() == "root"))
                    {
                        global = true;
                    }
                    class_name = false;
                    pseudo = false;
                }
                CssToken::Delim('*') if !declarations => global = true,
                CssToken::Delim('.') if !declarations => class_name = true,
                CssToken::Colon => {
                    if !declarations {
                        pseudo = true;
                    }
                    value = true;
                }
                CssToken::QuotedString(s) if import => {
                    push(
                        out,
                        "dependencies",
                        json!({"start":before,"end":after,"specifiers":[s.as_ref()],"kind":"css-import"}),
                    );
                    import = false;
                }
                CssToken::UnquotedUrl(s) => {
                    push(
                        out,
                        "dependencies",
                        json!({"start":before,"end":after,"specifiers":[s.as_ref()],"kind":if import{"css-import"}else{"css-url"}}),
                    );
                    import = false;
                }
                CssToken::Function(name) if name.eq_ignore_ascii_case("url") => {
                    let url: Result<String, cssparser::ParseError<()>> =
                        p.parse_nested_block(|p| Ok(p.expect_string()?.to_string()));
                    match url {
                        Ok(url) => push(
                            out,
                            "dependencies",
                            json!({"start":before,"end":p.position().byte_index(),"specifiers":[url],"kind":if import{"css-import"}else{"css-url"}}),
                        ),
                        Err(_) => gap(
                            out,
                            before,
                            p.position().byte_index(),
                            "css-url-parse-error",
                            "dependencies",
                        ),
                    };
                    import = false;
                }
                CssToken::CurlyBracketBlock => {
                    let end = p.position().byte_index();
                    let prelude = p.slice_from(start).trim_end_matches('{').trim().to_string();
                    if !declarations && global {
                        push(
                            out,
                            "findings",
                            json!({"start":start.byte_index(),"end":end,"kind":"css","ruleId":"global-selector","details":{"selector":prelude,"certainty":"static-candidate"}}),
                        );
                    }
                    if at_rule.ends_with("keyframes") {
                        push(
                            out,
                            "findings",
                            json!({"start":start.byte_index(),"end":end,"kind":"css","ruleId":"keyframes-name","details":{"prelude":prelude,"certainty":"static-candidate"}}),
                        );
                    }
                    let child_declarations = at_rule.is_empty()
                        || ["font-face", "page", "property"].contains(&at_rule.as_str());
                    p.parse_nested_block(|p| {
                        walk(p, out, child_declarations, steps, max, depth + 1)
                    })?;
                    start = p.position();
                    at_rule.clear();
                    property.clear();
                    value = false;
                    global = false;
                    import = false;
                }
                CssToken::Function(_)
                | CssToken::ParenthesisBlock
                | CssToken::SquareBracketBlock => {
                    p.parse_nested_block(|p| walk(p, out, true, steps, max, depth + 1))?;
                }
                CssToken::Semicolon => {
                    if declarations {
                        declaration(
                            out,
                            start.byte_index(),
                            after,
                            &property,
                            p.slice_from(start),
                        );
                    }
                    if import {
                        gap(
                            out,
                            start.byte_index(),
                            after,
                            "unknown-css-import",
                            "dependencies",
                        );
                    }
                    start = p.position();
                    property.clear();
                    value = false;
                    global = false;
                    at_rule.clear();
                    import = false;
                }
                CssToken::BadUrl(_)
                | CssToken::BadString(_)
                | CssToken::CloseCurlyBracket
                | CssToken::CloseParenthesis
                | CssToken::CloseSquareBracket => {
                    gap(out, before, after, "css-token-recovery", "css")
                }
                _ => {}
            }
        }
        if declarations {
            declaration(
                out,
                start.byte_index(),
                p.position().byte_index(),
                &property,
                p.slice_from(start),
            );
        }
        Ok(())
    }
    fn declaration(out: &mut Value, start: usize, end: usize, property: &str, text: &str) {
        if property.starts_with("--") || ["all", "animation", "animation-name"].contains(&property)
        {
            push(
                out,
                "findings",
                json!({"start":start,"end":end,"kind":"css","ruleId":if property.starts_with("--"){"custom-property"}else if property=="all"{"reset-declaration"}else{"animation-name"},"details":{"declaration":text,"certainty":"static-candidate"}}),
            );
        }
    }
    if walk(&mut parser, &mut out, false, &mut steps, max_steps, 0).is_err() {
        gap(&mut out, 0, source.len(), "css-parse-error", "css");
    }
    out
}

use crate::{
    model::{Context, gap, push, raw},
    resolve,
};
use cssparser::{Parser, Token as CssToken};
use html5gum::{DefaultEmitter, Token, Tokenizer};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn html_dependency(
    result: &mut Value,
    tag: &str,
    attrs: &BTreeMap<String, String>,
    source: &str,
    start: usize,
    end: usize,
    base: &str,
    map: &Value,
) {
    let mut d = json!({"start":start,"end":end,"baseUrl":base,"importMap":map,"attributes":attrs,"specifiers":[]});
    match tag {
        "script" => {
            let ty = attrs
                .get("type")
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if ![
                "",
                "module",
                "text/javascript",
                "application/javascript",
                "text/ecmascript",
                "application/ecmascript",
            ]
            .contains(&ty.as_str())
            {
                return;
            }
            d["kind"] = json!("script");
            d["scriptMode"] = json!(if ty == "module" { "module" } else { "classic" });
            if let Some(src) = attrs.get("src") {
                d["specifiers"] = json!([src]);
            } else {
                d["content"] = json!(&source[start..end]);
            }
            if attrs.contains_key("nomodule") {
                d["condition"] = json!("nomodule");
            }
        }
        "style" => {
            d["kind"] = json!("style");
            d["content"] = json!(&source[start..end]);
        }
        "link" => {
            let rel = attrs
                .get("rel")
                .map(|s| s.split_whitespace().collect::<Vec<_>>())
                .unwrap_or_default();
            let Some(href) = attrs.get("href") else {
                return;
            };
            if rel.contains(&"stylesheet") {
                d["kind"] = json!("style");
            } else if rel.contains(&"modulepreload") {
                d["kind"] = json!("preload");
                d["scriptMode"] = json!("module");
            } else {
                return;
            }
            d["specifiers"] = json!([href]);
        }
        "iframe" => {
            d["kind"] = json!("iframe");
            d["environment"] = json!("window");
            if let Some(content) = attrs.get("srcdoc") {
                d["content"] = json!(content);
                d["sourceLocationApproximate"] = json!(true);
            } else if let Some(src) = attrs.get("src") {
                d["specifiers"] = json!([src]);
            } else {
                return;
            }
        }
        _ => return,
    }
    push(result, "dependencies", d);
}
pub fn html(source: &str, url: &str, context: &Context, max_steps: usize) -> Value {
    let mut out = raw();
    let mut emitter = DefaultEmitter::<usize>::new_with_span();
    emitter.naively_switch_states(true);
    let mut base = context.base_url.clone().unwrap_or(url.into());
    let mut base_seen = false;
    let mut module_seen = false;
    let mut inert = 0usize;
    let mut foreign = 0usize;
    let mut map = match resolve::normalize(
        context.import_map.as_ref().unwrap_or(&json!({})),
        context.import_map_base_url.as_deref().unwrap_or(&base),
    ) {
        Ok(m) => m,
        Err(_) => {
            gap(&mut out, 0, 0, "invalid-import-map", "dependencies");
            json!({"imports":{},"scopes":{}})
        }
    };
    let mut pending: Option<(String, BTreeMap<String, String>, usize)> = None;
    for (steps, token) in Tokenizer::new_with_emitter(source, emitter).enumerate() {
        if steps > max_steps {
            gap(&mut out, 0, source.len(), "html-step-budget", "html");
            break;
        }
        let token = token.unwrap();
        match token {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_string();
                let attrs: BTreeMap<_, _> = tag
                    .attributes
                    .iter()
                    .map(|(k, v)| {
                        (
                            String::from_utf8_lossy(k).to_string(),
                            String::from_utf8_lossy(&v.value).to_string(),
                        )
                    })
                    .collect();
                if name == "template" {
                    inert += 1;
                }
                if inert > 0 {
                    continue;
                }
                if ["svg", "math"].contains(&name.as_str()) {
                    foreign += 1;
                    gap(
                        &mut out,
                        tag.span.start,
                        tag.span.end,
                        "html-foreign-tree-context",
                        "html",
                    );
                }
                if foreign > 0 {
                    continue;
                }
                if name == "noscript" {
                    gap(
                        &mut out,
                        tag.span.start,
                        tag.span.end,
                        "html-scripting-context",
                        "html",
                    );
                }
                if name == "base" && !base_seen {
                    if let Some(href) = attrs.get("href") {
                        match resolve::join(url, href) {
                            Ok(v) => {
                                base = v;
                                base_seen = true
                            }
                            Err(_) => gap(
                                &mut out,
                                tag.span.start,
                                tag.span.end,
                                "invalid-html-base",
                                "dependencies",
                            ),
                        }
                    }
                }
                if name == "script" || name == "style" {
                    pending = Some((name.clone(), attrs.clone(), tag.span.end));
                } else {
                    html_dependency(
                        &mut out,
                        &name,
                        &attrs,
                        source,
                        tag.span.start,
                        tag.span.end,
                        &base,
                        &map,
                    );
                }
                for (key, value) in &attrs {
                    if key.starts_with("on") {
                        gap(
                            &mut out,
                            tag.span.start,
                            tag.span.end,
                            "html-event-handler-special-scope",
                            "globals",
                        );
                    }
                    if key == "style" {
                        push(
                            &mut out,
                            "findings",
                            json!({"start":tag.span.start,"end":tag.span.end,"kind":"css","ruleId":"inline-style","details":{"value":value,"certainty":"static-candidate"}}),
                        );
                    }
                }
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name);
                if name == "template" {
                    inert = inert.saturating_sub(1);
                    continue;
                }
                if inert > 0 {
                    continue;
                }
                if ["svg", "math"].contains(&name.as_ref()) {
                    foreign = foreign.saturating_sub(1);
                    continue;
                }
                if foreign > 0 {
                    continue;
                }
                if pending.as_ref().is_some_and(|(n, _, _)| n == &name) {
                    let (n, attrs, start) = pending.take().unwrap();
                    let end = tag.span.start;
                    if n == "script"
                        && attrs
                            .get("type")
                            .is_some_and(|t| t.trim().eq_ignore_ascii_case("importmap"))
                    {
                        if module_seen {
                            gap(
                                &mut out,
                                start,
                                end,
                                "import-map-after-module-resolution",
                                "dependencies",
                            );
                        }
                        match serde_json::from_str::<Value>(&source[start..end])
                            .map_err(|e| e.to_string())
                            .and_then(|v| resolve::normalize(&v, &base))
                        {
                            Ok(next) => map = resolve::merge_maps(&map, &next),
                            Err(_) => {
                                gap(&mut out, start, end, "invalid-import-map", "dependencies")
                            }
                        }
                    } else {
                        if attrs.get("type").is_some_and(|t| t == "module") {
                            module_seen = true;
                        }
                        html_dependency(&mut out, &n, &attrs, source, start, end, &base, &map);
                    }
                }
            }
            Token::Error(error) => gap(
                &mut out,
                error.span.start,
                error.span.end,
                "html-token-recovery",
                "html",
            ),
            _ => {}
        }
    }
    if let Some((name, attrs, start)) = pending {
        gap(
            &mut out,
            start,
            source.len(),
            "unclosed-html-rawtext",
            "html",
        );
        html_dependency(
            &mut out,
            &name,
            &attrs,
            source,
            start,
            source.len(),
            &base,
            &map,
        );
    }
    out
}
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

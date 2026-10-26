//! HTML entry extraction.
//!
//! HTML is not analyzed for pollution itself; it is the index that tells the
//! scheduler which scripts, styles and frames a page loads. The extractor
//! walks the token stream once and records:
//!
//! * `<script src>` / inline `<script>` with their script mode,
//! * `<link rel=stylesheet>` / `<link rel=modulepreload>` / `<style>`,
//! * `<iframe src>`,
//! * `<base href>` and `<script type=importmap>` so children resolve
//!   relative URLs the way the browser would,
//! * a `style=` attribute on `<html>`/`<body>` as the only
//!   HTML-level pollution finding (styles scoped to the whole document).
//!
//! CSS resources are reported as dependencies and are never analyzed.

use crate::model::{
    Analysis, Confidence, Dependency, DependencyKind, Finding, Gap, ImportMap, Rule, ScriptMode,
    Span,
};
use crate::resolve;
use html5gum::{DefaultEmitter, Token, Tokenizer};
use std::collections::BTreeMap;

/// Script `type` values treated as executable JavaScript. Anything else
/// (`application/json`, `importmap`, `text/template`, …) is data.
fn script_mode(attrs: &BTreeMap<String, String>) -> Option<ScriptMode> {
    let ty = attrs
        .get("type")
        .map(|t| t.trim().to_ascii_lowercase())
        .unwrap_or_default();
    match ty.as_str() {
        "module" => Some(ScriptMode::Module),
        ""
        | "text/javascript"
        | "application/javascript"
        | "text/ecmascript"
        | "application/ecmascript" => Some(ScriptMode::Classic),
        _ => None,
    }
}

/// A raw-text element whose body we are waiting to close.
struct Pending {
    name: String,
    attrs: BTreeMap<String, String>,
    tag_span: Span,
    body_start: u32,
}

/// Extracts dependencies and global-style findings from an HTML document.
///
/// `url` is the document's own URL and the default base for relative links.
/// `max_steps` bounds the number of tokens processed.
pub fn extract(source: &str, url: &str, max_steps: usize) -> Analysis {
    let mut out = Analysis::default();
    let mut base = url.to_string();
    let mut base_seen = false;
    let mut import_map: Option<ImportMap> = None;
    let mut pending: Option<Pending> = None;
    // Depth counters for contexts whose content is inert or foreign.
    let mut template_depth = 0usize;
    let mut foreign_depth = 0usize;

    let mut emitter = DefaultEmitter::<usize>::new_with_span();
    emitter.naively_switch_states(true);

    for (step, token) in Tokenizer::new_with_emitter(source, emitter).enumerate() {
        if step >= max_steps {
            out.gaps.push(Gap {
                reason: "html-step-budget".into(),
                span: Span::new(0, source.len() as u32),
                detail: None,
            });
            break;
        }
        let Ok(token) = token;
        match token {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
                let attrs: BTreeMap<String, String> = tag
                    .attributes
                    .iter()
                    .map(|(k, v)| {
                        (
                            String::from_utf8_lossy(k).to_ascii_lowercase(),
                            String::from_utf8_lossy(&v.value).to_string(),
                        )
                    })
                    .collect();
                let span = Span::new(tag.span.start as u32, tag.span.end as u32);

                if name == "template" {
                    template_depth += 1;
                }
                if template_depth > 0 {
                    continue;
                }
                if name == "svg" || name == "math" {
                    out.gaps.push(Gap {
                        reason: "foreign-html-content".into(),
                        span,
                        detail: None,
                    });
                    if !tag.self_closing {
                        foreign_depth += 1;
                    }
                    continue;
                }
                if foreign_depth > 0 {
                    continue;
                }

                match name.as_str() {
                    "base" if !base_seen => {
                        if let Some(href) = attrs.get("href") {
                            match resolve::join(url, href) {
                                Ok(resolved) => {
                                    base = resolved;
                                    base_seen = true;
                                }
                                Err(_) => out.gaps.push(Gap {
                                    reason: "invalid-html-base".into(),
                                    span,
                                    detail: None,
                                }),
                            }
                        }
                    }
                    "html" | "body" => {
                        if let Some(style) = attrs.get("style") {
                            if !style.trim().is_empty() {
                                out.findings.push(Finding {
                                    rule: Rule::HtmlGlobalStyle,
                                    span,
                                    target: format!("<{name} style>"),
                                    operation: "style".into(),
                                    confidence: Confidence::High,
                                    detail: Some(style.trim().to_string()),
                                });
                            }
                        }
                    }
                    "script" | "style" => {
                        // Body is captured when the end tag arrives; `tag.span.end`
                        // is the byte after `>` of the start tag.
                        pending = Some(Pending {
                            name: name.clone(),
                            attrs,
                            tag_span: span,
                            body_start: span.end,
                        });
                    }
                    "link" => {
                        let rel: Vec<String> = attrs
                            .get("rel")
                            .map(|r| {
                                r.split_whitespace()
                                    .map(|s| s.to_ascii_lowercase())
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let Some(href) = attrs.get("href") {
                            if rel.iter().any(|r| r == "stylesheet") {
                                out.dependencies.push(dependency(
                                    DependencyKind::Style,
                                    span,
                                    href,
                                    &base,
                                    None,
                                ));
                            } else if rel.iter().any(|r| r == "modulepreload") {
                                out.dependencies.push(dependency(
                                    DependencyKind::ModulePreload,
                                    span,
                                    href,
                                    &base,
                                    Some(ScriptMode::Module),
                                ));
                            }
                        }
                    }
                    "iframe" => {
                        if let Some(srcdoc) = attrs.get("srcdoc") {
                            out.dependencies.push(Dependency {
                                kind: DependencyKind::Iframe,
                                span,
                                specifiers: vec![],
                                content: Some(srcdoc.clone()),
                                content_offset: Some(span.start),
                                script_mode: None,
                                base_url: Some(base.clone()),
                            });
                        } else if let Some(src) = attrs.get("src") {
                            if !src.trim().is_empty() && !src.trim_start().starts_with("about:") {
                                out.dependencies.push(dependency(
                                    DependencyKind::Iframe,
                                    span,
                                    src,
                                    &base,
                                    None,
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
                if name == "template" {
                    template_depth = template_depth.saturating_sub(1);
                    continue;
                }
                if template_depth > 0 {
                    continue;
                }
                if name == "svg" || name == "math" {
                    foreign_depth = foreign_depth.saturating_sub(1);
                    continue;
                }
                if foreign_depth > 0 {
                    continue;
                }
                if pending.as_ref().is_some_and(|p| p.name == name) {
                    let p = pending.take().unwrap();
                    let body_end = tag.span.start as u32;
                    finish_raw_text(&mut out, &p, source, body_end, &base, &mut import_map);
                }
            }
            _ => {}
        }
    }

    if let Some(p) = pending.take() {
        out.gaps.push(Gap {
            reason: "unclosed-html-rawtext".into(),
            span: p.tag_span,
            detail: None,
        });
        finish_raw_text(
            &mut out,
            &p,
            source,
            source.len() as u32,
            &base,
            &mut import_map,
        );
    }

    out.base_url = Some(base);
    out.import_map = import_map;
    out
}

/// Emits the dependency for a closed `<script>` or `<style>` element.
fn finish_raw_text(
    out: &mut Analysis,
    p: &Pending,
    source: &str,
    body_end: u32,
    base: &str,
    import_map: &mut Option<ImportMap>,
) {
    let body_start = p.body_start.min(body_end);
    let body = &source[body_start as usize..body_end as usize];
    match p.name.as_str() {
        "style" => {
            // A `<style>` block applies to the whole document, but that is the
            // normal way to ship page CSS. It is a dependency, not a finding.
            out.dependencies.push(Dependency {
                kind: DependencyKind::Style,
                span: p.tag_span,
                specifiers: vec![],
                content: Some(body.to_string()),
                content_offset: Some(body_start),
                script_mode: None,
                base_url: Some(base.to_string()),
            });
        }
        "script" => {
            let ty = p
                .attrs
                .get("type")
                .map(|t| t.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if ty == "importmap" {
                if out
                    .dependencies
                    .iter()
                    .any(|d| d.script_mode == Some(ScriptMode::Module))
                {
                    out.gaps.push(Gap {
                        reason: "late-import-map".into(),
                        span: p.tag_span,
                        detail: None,
                    });
                    return;
                }
                match serde_json::from_str::<serde_json::Value>(body)
                    .map_err(|e| e.to_string())
                    .and_then(|v| resolve::parse_import_map(&v, base))
                {
                    Ok(map) => {
                        *import_map = Some(match import_map.take() {
                            Some(prior) => resolve::merge_import_maps(&prior, &map),
                            None => map,
                        });
                    }
                    Err(detail) => out.gaps.push(Gap {
                        reason: "invalid-import-map".into(),
                        span: p.tag_span,
                        detail: Some(detail),
                    }),
                }
                return;
            }
            let Some(mode) = script_mode(&p.attrs) else {
                return;
            };
            if let Some(src) = p.attrs.get("src") {
                out.dependencies.push(dependency(
                    DependencyKind::Script,
                    p.tag_span,
                    src,
                    base,
                    Some(mode),
                ));
            } else if !body.trim().is_empty() {
                out.dependencies.push(Dependency {
                    kind: DependencyKind::InlineScript,
                    span: p.tag_span,
                    specifiers: vec![],
                    content: Some(body.to_string()),
                    content_offset: Some(body_start),
                    script_mode: Some(mode),
                    base_url: Some(base.to_string()),
                });
            }
        }
        _ => {}
    }
}

fn dependency(
    kind: DependencyKind,
    span: Span,
    specifier: &str,
    base: &str,
    mode: Option<ScriptMode>,
) -> Dependency {
    Dependency {
        kind,
        span,
        specifiers: vec![specifier.trim().to_string()],
        content: None,
        content_offset: None,
        script_mode: mode,
        base_url: Some(base.to_string()),
    }
}

#[cfg(test)]
#[path = "../tests/html.rs"]
mod tests;

//! Data model shared by the scheduler, analyzers, and the manifest.
//!
//! Everything in this module is plain data. Input types (`Source`, `Entry`,
//! `Options`) arrive from Node.js as JSON; output types (`Manifest` and the
//! records inside it) are serialized back to Node.js as JSON. The analyzers
//! produce `Finding`, `Dependency` and `Gap` records with byte spans, and the
//! report builder converts those spans into line/column locations.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use url::Url;

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Where a resource's bytes come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    /// A file on the local disk. Relative paths resolve against the process
    /// working directory.
    File { path: String },
    /// An `http:`, `https:` or `file:` URL.
    Url { url: String },
    /// Source text supplied directly. `resource_url` is the logical address
    /// used to resolve relative dependencies and to label the resource.
    Content {
        content: String,
        #[serde(
            rename = "resourceUrl",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        resource_url: Option<String>,
    },
}

impl Source {
    /// The logical URL of this source. Anonymous content gets an opaque
    /// `memory:` identity derived from its hash so relative dependencies
    /// cannot accidentally resolve against a fabricated base.
    pub fn url(&self) -> Result<String, String> {
        match self {
            Self::File { path } => {
                let p = PathBuf::from(path);
                let absolute = if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir().map_err(|e| e.to_string())?.join(p)
                };
                Url::from_file_path(absolute)
                    .map(|u| u.to_string())
                    .map_err(|_| "invalid-file-path".into())
            }
            Self::Url { url } => Ok(url.clone()),
            Self::Content {
                resource_url: Some(url),
                ..
            } => Ok(url.clone()),
            Self::Content { content, .. } => Ok(format!("memory:{}", content_hash(content))),
        }
    }
}

/// The kind of resource being analyzed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Js,
    Html,
    Css,
}

/// How a JavaScript resource is executed. This changes scope semantics:
/// top-level `var`/`function` in a classic script become `window`
/// properties; in a module they stay module-local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptMode {
    Classic,
    Module,
}

/// A scan entry point supplied by the caller.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub kind: Kind,
    pub source: Source,
    /// Overrides the logical URL of the source (for example when scanning a
    /// local mirror of a deployed file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_url: Option<String>,
    /// Script mode for JS entries. Defaults to `Options::script_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_mode: Option<ScriptMode>,
}

/// Limits that bound the work a scan performs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Budgets {
    /// Maximum queued/in-flight load-and-analysis jobs.
    pub concurrency: usize,
    /// Worker count, capped by concurrency. Each worker loads then analyzes.
    pub analysis_concurrency: usize,
    /// Wall clock limit for the whole scan.
    pub timeout_ms: u64,
    /// Per-request network timeout.
    pub resource_timeout_ms: u64,
    /// Maximum number of resources loaded.
    pub max_resources: usize,
    /// Maximum size of a single resource.
    pub max_resource_bytes: usize,
    /// Maximum total bytes loaded across the scan.
    pub max_bytes: usize,
    /// Maximum dependency depth from an entry.
    pub max_depth: usize,
    /// Maximum AST nodes visited per JavaScript resource.
    pub max_steps: usize,
    /// Maximum string candidates tracked for one value.
    pub max_candidates: usize,
    /// HTTP redirects followed per request.
    pub max_redirects: usize,
    /// HTTP retries on transport failure.
    pub retries: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            concurrency: 8,
            analysis_concurrency: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            timeout_ms: 120_000,
            resource_timeout_ms: 30_000,
            max_resources: 500,
            max_resource_bytes: 16 * 1024 * 1024,
            max_bytes: 256 * 1024 * 1024,
            max_depth: 16,
            max_steps: 2_000_000,
            max_candidates: 32,
            max_redirects: 5,
            retries: 1,
        }
    }
}

/// Restrictions on what the loader may fetch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Access {
    /// Allowed URL schemes including the trailing colon, e.g. `"https:"`.
    /// `None` allows `file:`, `http:` and `https:`.
    pub protocols: Option<Vec<String>>,
    /// Allowed `host[:port]` values for network requests.
    pub hosts: Option<Vec<String>>,
    /// Directories that `file:` URLs must live inside.
    pub file_roots: Option<Vec<String>>,
}

/// Scan configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Script mode for JavaScript entries and for scripts whose mode cannot
    /// be determined from HTML. Defaults to `classic`.
    pub script_mode: ScriptMode,
    /// `single` analyzes only the entries; `recursive` follows dependencies.
    pub dependency_mode: DependencyMode,
    /// Document base used when a resource has no usable URL of its own.
    pub base_url: Option<String>,
    /// Import map applied to module specifiers in standalone JS entries. HTML
    /// entries read their own import maps.
    pub import_map: Option<serde_json::Value>,
    pub budgets: Budgets,
    pub access: Access,
    /// Regex (Rust syntax) matched against the resolved logical URL of
    /// recursively discovered dependencies. Matching dependencies are not
    /// loaded or analyzed; a matching entry fails the scan.
    pub exclude_urls: Vec<String>,
    /// Substitute sources for resolved URLs. Used by bundler plugins to scan
    /// in-memory output without writing files or hitting the network.
    pub resource_mappings: BTreeMap<String, Source>,
    /// Extra request headers keyed by `host[:port]`.
    pub request_headers: BTreeMap<String, BTreeMap<String, String>>,
    pub proxy: Option<String>,
    /// Decode `sourceMappingURL` comments and attach original locations.
    pub source_maps: bool,
    /// Internal: identifies the run for cancellation lookups.
    #[serde(skip_serializing)]
    pub run_id: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            script_mode: ScriptMode::Classic,
            dependency_mode: DependencyMode::Recursive,
            base_url: None,
            import_map: None,
            budgets: Budgets::default(),
            access: Access::default(),
            exclude_urls: vec![],
            resource_mappings: BTreeMap::new(),
            request_headers: BTreeMap::new(),
            proxy: None,
            source_maps: false,
            run_id: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyMode {
    Single,
    Recursive,
}

/// The top-level request received from Node.js.
#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub options: Options,
}

// ---------------------------------------------------------------------------
// Analyzer output (byte spans, before location conversion)
// ---------------------------------------------------------------------------

/// A half-open byte range inside a resource.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

impl From<oxc_span::Span> for Span {
    fn from(s: oxc_span::Span) -> Self {
        Self {
            start: s.start,
            end: s.end,
        }
    }
}

/// Pollution categories. These are the only things the scanner reports as
/// findings; everything else is either a dependency edge or a coverage gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    /// A write to a property of the global object: `window.foo = ...`,
    /// `globalThis.foo = ...`, `Object.defineProperty(window, ...)`,
    /// or a top-level `var`/`function` declaration in a classic script.
    GlobalWrite,
    /// A write to a property of `document` or a well-known document node
    /// such as `document.title = ...` or `document.body.className = ...`.
    DocumentWrite,
    /// Mutation of a built-in prototype or constructor:
    /// `Array.prototype.foo = ...`, `Promise.allSettled = ...`.
    BuiltinMutation,
    /// Replacement of a host API: `window.fetch = ...`,
    /// `XMLHttpRequest.prototype.open = ...`, `history.pushState = ...`.
    HostApiOverride,
    /// A DOM node inserted imperatively under `document`, `document.body`
    /// or `document.head`: `document.body.appendChild(el)`,
    /// `document.head.insertAdjacentHTML(...)`, `document.write(...)`.
    DomInsertion,
    /// A `<style>` element or stylesheet `<link>` created by script and
    /// inserted into the document, or `CSSStyleSheet.insertRule` on a
    /// document stylesheet, without a matching removal in the same resource.
    StyleInjection,
    /// An inline style applied to a document-level element:
    /// `document.body.style.overflow = 'hidden'`, `document.documentElement.style.setProperty(...)`.
    DocumentStyleWrite,
    /// An event listener registered on `window` or `document` without a
    /// same-event, same-handler removal in the same resource.
    GlobalListener,
    /// `style=` attribute on `<html>`/`<body>` in HTML.
    HtmlGlobalStyle,
}

impl Rule {
    /// Default severity for consumers that block releases on severe
    /// pollution. Consumers may override this per rule.
    pub fn severity(self) -> Severity {
        match self {
            Self::BuiltinMutation | Self::HostApiOverride => Severity::Error,
            Self::GlobalWrite
            | Self::DocumentWrite
            | Self::DomInsertion
            | Self::StyleInjection
            | Self::DocumentStyleWrite
            | Self::GlobalListener => Severity::Warning,
            Self::HtmlGlobalStyle => Severity::Info,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// How confident the analyzer is that the finding targets a global.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// The receiver is provably the global object, `document`, or a builtin
    /// (direct reference or a const alias).
    High,
    /// The receiver was tracked through a mutable alias or a conditional
    /// branch, so a runtime reassignment could change the target.
    Medium,
}

/// A pollution finding produced by an analyzer, located by byte span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub rule: Rule,
    pub span: Span,
    /// Dotted path of the polluted target, e.g. `window.sharedConfig`,
    /// `Array.prototype.flatMap`, `document.body`.
    pub target: String,
    /// What was done: `write`, `define`, `delete`, `append`, `insert`,
    /// `listen`, `style`, `replace`.
    pub operation: String,
    pub confidence: Confidence,
    /// Optional extra context such as the method name or value type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Why a dependency was discovered. Determines the child resource kind and
/// script mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    /// `<script src>` in HTML.
    Script,
    /// Inline `<script>` content in HTML.
    InlineScript,
    /// `<link rel=stylesheet>` or `<style>` in HTML.
    Style,
    /// `<link rel=modulepreload>` in HTML.
    ModulePreload,
    /// `<iframe src>` / `srcdoc` in HTML; discovered but never followed.
    Iframe,
    /// Static `import`/`export ... from` in JavaScript.
    Import,
    /// `import()` in JavaScript.
    DynamicImport,
    /// `new Worker(url)` or `importScripts(url)`.
    Worker,
    /// `script.src = ...` on a created script element.
    DynamicScript,
    /// `link.href = ...` on a created stylesheet link.
    DynamicStyle,
}

impl DependencyKind {
    /// The resource kind this dependency loads.
    pub fn child_kind(self) -> Kind {
        match self {
            Self::Style | Self::DynamicStyle => Kind::Css,
            Self::Iframe => Kind::Html,
            _ => Kind::Js,
        }
    }

    /// Whether the specifier resolves through import maps and rejects bare
    /// specifiers without a mapping (ESM semantics).
    pub fn is_module_specifier(self) -> bool {
        matches!(self, Self::Import | Self::DynamicImport)
    }
}

/// A dependency discovered by an analyzer. Either `specifier` (URL-like) or
/// `content` (inline code) is present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependency {
    pub kind: DependencyKind,
    pub span: Span,
    /// Candidate specifiers. Empty when the value could not be determined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specifiers: Vec<String>,
    /// Inline content for `InlineScript`/`Style` dependencies from HTML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Byte offset of `content` inside the parent resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_offset: Option<u32>,
    /// Script mode for JS children when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_mode: Option<ScriptMode>,
    /// Base URL override for resolution (HTML `<base>` or document base for
    /// workers created from inline scripts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// A coverage gap: something the analyzer could not determine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub reason: String,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The import map snapshot an HTML analyzer discovered, carried to children.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportMap {
    #[serde(default)]
    pub imports: BTreeMap<String, Option<String>>,
    #[serde(default)]
    pub scopes: BTreeMap<String, BTreeMap<String, Option<String>>>,
}

/// Everything one analyzer run produces for one resource.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub findings: Vec<Finding>,
    pub dependencies: Vec<Dependency>,
    pub gaps: Vec<Gap>,
    /// Effective document base after `<base href>` (HTML only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Import map after all `<script type=importmap>` blocks (HTML only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_map: Option<ImportMap>,
}

// ---------------------------------------------------------------------------
// Manifest (output)
// ---------------------------------------------------------------------------

/// Line/column location. Lines are 1-based; columns are 0-based UTF-16 code
/// units, matching editors and source maps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    /// Original position from a source map when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<OriginalLocation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalLocation {
    pub source: String,
    pub line: u32,
    pub column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A resource that was analyzed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSource {
    /// Manifest-local id such as `s1`.
    pub id: String,
    pub kind: Kind,
    /// Logical URL. Absent for anonymous inline content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Present for inline scripts and styles extracted from an HTML parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<InlineOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_mode: Option<ScriptMode>,
    pub status: SourceStatus,
    pub bytes: usize,
    pub content_hash: String,
    /// Ids of the resources that referenced this one. Empty for entries.
    pub referrers: Vec<String>,
    /// Shortest chain of source ids from an entry to this resource,
    /// inclusive of both ends. Lets consumers explain *why* a file was
    /// scanned.
    pub chain: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineOrigin {
    /// Source id of the containing document.
    pub parent: String,
    /// Line/column of the inline block inside the parent.
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceStatus {
    /// Analyzed without gaps.
    Completed,
    /// Analyzed, but some constructs could not be followed.
    Partial,
    /// Could not be loaded or parsed.
    Failed,
}

/// A pollution finding in the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestFinding {
    /// Source id that contains the pollution.
    pub source: String,
    pub rule: Rule,
    pub severity: Severity,
    pub target: String,
    pub operation: String,
    pub confidence: Confidence,
    pub location: Location,
    /// Source code excerpt of the offending expression (trimmed).
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A dependency edge in the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDependency {
    pub from: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub kind: DependencyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub status: DependencyStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub location: Location,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyStatus {
    /// Resolved and analyzed; `to` is set.
    Resolved,
    /// Resolved but intentionally not followed (single mode, or a CSS
    /// resource, which is reported as a dependency only).
    NotFollowed,
    /// Could not be resolved or loaded; `reason` explains.
    Unresolved,
}

/// A coverage gap in the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestGap {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub reason: String,
    pub count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Completed,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub sources: usize,
    pub findings: usize,
    pub errors: usize,
    pub warnings: usize,
    pub dependencies: usize,
    pub unresolved: usize,
    pub gaps: usize,
    pub bytes: usize,
    pub elapsed_ms: u64,
}

/// The scan result consumed by platforms and the bundler plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub scanner_version: String,
    pub status: ScanStatus,
    pub dependency_mode: DependencyMode,
    pub summary: Summary,
    /// Source ids of the entries, in request order.
    pub entries: Vec<String>,
    pub sources: Vec<ManifestSource>,
    pub findings: Vec<ManifestFinding>,
    pub dependencies: Vec<ManifestDependency>,
    pub gaps: Vec<ManifestGap>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// SHA-256 hex digest of a string.
pub fn content_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;

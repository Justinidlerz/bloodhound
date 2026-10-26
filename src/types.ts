/**
 * Public types of `@bloodhound-rs/core`.
 *
 * These mirror the Rust data model in `native/src/model.rs`. The Rust side is
 * the source of truth; keep field names and enum values in sync.
 */

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/** Where a resource's bytes come from. */
export type Source =
  | { type: 'file'; path: string }
  | { type: 'url'; url: string }
  | {
      type: 'content';
      content: string;
      /** Logical URL used to resolve relative dependencies and to label the resource. */
      resourceUrl?: string;
    };

export type ResourceKind = 'js' | 'html' | 'css';

/**
 * How a JavaScript resource executes. Top-level `var`/`function` in a classic
 * script become `window` properties; in a module they stay module-local.
 */
export type ScriptMode = 'classic' | 'module';

export type DependencyMode = 'single' | 'recursive';

export interface ScanEntry {
  kind: ResourceKind;
  source: Source;
  /** Overrides the logical URL of the source. */
  resourceUrl?: string;
  /** Script mode for JS entries. Defaults to `ScanOptions.scriptMode`. */
  scriptMode?: ScriptMode;
}

export interface Budgets {
  /** Maximum in-flight load-and-analysis jobs. Default 8, at most 256. */
  concurrency: number;
  /** Worker count, capped by concurrency. Default: CPU count, at most 256. */
  analysisConcurrency: number;
  /** Wall clock limit for the whole scan. Default 120 000. */
  timeoutMs: number;
  /** Per-request network timeout. Default 30 000. */
  resourceTimeoutMs: number;
  /** Maximum resources loaded. Default 500. */
  maxResources: number;
  /** Maximum size of one resource. Default 16 MiB. */
  maxResourceBytes: number;
  /** Maximum total bytes loaded. Default 256 MiB. */
  maxBytes: number;
  /** Maximum dependency depth from an entry. Default 16. */
  maxDepth: number;
  /** Maximum AST nodes visited per JavaScript resource. Default 2 000 000. */
  maxSteps: number;
  /** Maximum string candidates tracked per value. Default 32. */
  maxCandidates: number;
  /** HTTP redirects followed per request. Default 5. */
  maxRedirects: number;
  /** HTTP retries on transport failure. Default 1. */
  retries: number;
}

export interface Access {
  /** Allowed URL schemes with trailing colon, e.g. `['https:']`. Default: file, http, https. */
  protocols?: string[];
  /** Allowed `host[:port]` values for network requests. */
  hosts?: string[];
  /** Directories that `file:` URLs must live inside. */
  fileRoots?: string[];
}

export interface ImportMap {
  imports?: Record<string, string | null>;
  scopes?: Record<string, Record<string, string | null>>;
}

export interface ScanOptions {
  /** Script mode for JS entries and for scripts whose mode is unknown. Default `classic`. */
  scriptMode?: ScriptMode;
  /** `single` analyzes only the entries; `recursive` follows dependencies. Default `recursive`. */
  dependencyMode?: DependencyMode;
  /** Logical URL for a string or content input (used by `scanHtml`/`scanJs`). */
  resourceUrl?: string;
  /** Document base when a resource has no usable URL of its own. */
  baseUrl?: string;
  /** Import map applied to standalone JS entries. HTML entries read their own. */
  importMap?: ImportMap;
  budgets?: Partial<Budgets>;
  access?: Access;
  /**
   * Regex source strings (Rust regex syntax: no lookaround or backreferences) matched against
   * the resolved logical URL of recursively discovered dependencies, e.g. `'/vendor/'` or
   * `'\\.min\\.js($|\\?)'`. Matching dependencies are not loaded or analyzed and appear in
   * `dependencies` as `not-followed` with `reason: 'excluded'`. Inline scripts are not matched.
   * A matching entry, or an invalid pattern, rejects the scan (throws).
   */
  excludeUrls?: string[];
  /** Substitute sources for resolved URLs (offline scanning of bundler output). */
  resourceMappings?: Record<string, Source>;
  /** Extra request headers keyed by `host[:port]`. */
  requestHeaders?: Record<string, Record<string, string>>;
  proxy?: string;
  /** Decode `sourceMappingURL` comments and attach original locations. */
  sourceMaps?: boolean;
  /** Cancels a running scan. The result is a manifest with `status: 'cancelled'`. */
  signal?: AbortSignal;
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/**
 * Pollution categories reported by the scanner.
 *
 * | rule                   | meaning                                                             |
 * |------------------------|---------------------------------------------------------------------|
 * | `global-write`         | `window.x = …`, top-level `var`/`function` in classic scripts       |
 * | `document-write`       | `document.body.className = …`, `document.body.setAttribute(…)`      |
 * | `builtin-mutation`     | `Array.prototype.x = …`, `Promise.allSettled = …`                   |
 * | `host-api-override`    | `window.fetch = …`, `XMLHttpRequest.prototype.open = …`             |
 * | `dom-insertion`        | `document.body.appendChild(…)`, `document.write(…)`                 |
 * | `style-injection`      | `<style>`/`<link>` inserted by script without removal               |
 * | `document-style-write` | `document.body.style.x = …`, `documentElement.style.setProperty(…)` |
 * | `global-listener`      | `window.addEventListener(…)` without `removeEventListener`          |
 * | `html-global-style`    | `style=` attribute on `<html>`/`<body>`                             |
 */
export type Rule =
  | 'global-write'
  | 'document-write'
  | 'builtin-mutation'
  | 'host-api-override'
  | 'dom-insertion'
  | 'style-injection'
  | 'document-style-write'
  | 'global-listener'
  | 'html-global-style';

export type Severity = 'info' | 'warning' | 'error';

/**
 * `high`: the receiver is provably the global object, `document`, or a builtin.
 * `medium`: the receiver was tracked through a reassignable alias or a branch.
 */
export type Confidence = 'high' | 'medium';

export type ScanStatus = 'completed' | 'partial' | 'failed' | 'cancelled';
export type SourceStatus = 'completed' | 'partial' | 'failed';
export type DependencyStatus = 'resolved' | 'not-followed' | 'unresolved';

export type DependencyKind =
  | 'script'
  | 'inline-script'
  | 'style'
  | 'module-preload'
  | 'iframe'
  | 'import'
  | 'dynamic-import'
  | 'worker'
  | 'dynamic-script'
  | 'dynamic-style';

/** Lines are 1-based; columns are 0-based UTF-16 code units. */
export interface Location {
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  /** Original position from a source map when `sourceMaps` was enabled. */
  original?: { source: string; line: number; column: number; name?: string };
}

export interface ManifestSource {
  /** Manifest-local id such as `s1`. */
  id: string;
  kind: ResourceKind;
  /** Logical URL. Absent for anonymous inline content. */
  url?: string;
  /** Present for inline scripts extracted from an HTML parent. */
  inline?: { parent: string; line: number; column: number };
  scriptMode?: ScriptMode;
  status: SourceStatus;
  bytes: number;
  contentHash: string;
  /** Ids of the sources that referenced this one. Empty for entries. */
  referrers: string[];
  /** Shortest chain of source ids from an entry to this source, inclusive. */
  chain: string[];
}

export interface ManifestFinding {
  /** Source id containing the pollution. */
  source: string;
  rule: Rule;
  severity: Severity;
  /** Dotted path of the polluted target, e.g. `window.sharedConfig`. */
  target: string;
  /** `write` | `update` | `define` | `delete` | `declare` | `insert` | `listen` | `style`. */
  operation: string;
  confidence: Confidence;
  location: Location;
  /** Trimmed excerpt of the offending expression. */
  snippet: string;
  detail?: string;
}

export interface ManifestDependency {
  from: string;
  /** Set when the dependency was resolved and analyzed. */
  to?: string;
  kind: DependencyKind;
  specifier?: string;
  url?: string;
  status: DependencyStatus;
  reason?: string;
  location: Location;
}

export interface ManifestGap {
  source?: string;
  reason: string;
  count: number;
  detail?: string;
  location?: Location;
}

export interface Summary {
  sources: number;
  findings: number;
  errors: number;
  warnings: number;
  dependencies: number;
  unresolved: number;
  gaps: number;
  bytes: number;
  elapsedMs: number;
}

/** The scan result. */
export interface Manifest {
  schemaVersion: 2;
  scannerVersion: string;
  status: ScanStatus;
  dependencyMode: DependencyMode;
  summary: Summary;
  /** Source ids of the entries, in request order. */
  entries: string[];
  sources: ManifestSource[];
  findings: ManifestFinding[];
  dependencies: ManifestDependency[];
  gaps: ManifestGap[];
}

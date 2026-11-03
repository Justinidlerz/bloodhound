/** Manifest v2 types, shared by the scanner and browser consumers.
 * Keep in sync with native/src/model.rs. */
export type ResourceKind = 'js' | 'html' | 'css';
/** Classic top-level var/function declarations become globals; module declarations stay local. */
export type ScriptMode = 'classic' | 'module';
export type DependencyMode = 'single' | 'recursive';

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
 * | `module-state-write`   | function/callback writes to cached ESM module state                |
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
  | 'module-state-write'
  | 'html-global-style';

export type Severity = 'info' | 'warning' | 'error';

/**
 * `high`: the target resolves to a global, `document`, builtin, or module binding.
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
  original?: {
    source: string;
    line: number;
    column: number;
    name?: string;
    endLine?: number;
    endColumn?: number;
  };
}

/** Optional bundler/consumer association. URLs are relative to the manifest unless absolute. */
export interface SourceMapReference {
  url: string;
  /** Map file relative to the project root, for local previews. */
  file?: string;
  /** Project-relative paths in the flattened map's source order; null for external/virtual files. */
  sourcePaths?: (string | null)[];
}

export interface ManifestSource {
  /** Manifest-local id such as `s1`. */
  id: string;
  kind: ResourceKind;
  /** Logical URL. Absent for anonymous inline content. */
  url?: string;
  /** Emitted file relative to the project root, supplied by the bundler plugin. */
  file?: string;
  /** Source map used to preview original code. Absent when no map is associated. */
  sourceMap?: SourceMapReference;
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
  /** Target path, e.g. `window.sharedConfig` or the ESM binding `ready`. */
  target: string;
  /** `write` | `update` | `define` | `delete` | `declare` | `insert` | `listen` | `style`. */
  operation: string;
  confidence: Confidence;
  location: Location;
  /** Trimmed excerpt of the offending expression. */
  snippet: string;
  detail?: string;
  /** Definition of the lexical module binding modified by this record. */
  declaration?: { name: string; location: Location };
  /** All operations (including the first) grouped by declaration or rule/source line within one resource. */
  writes?: ManifestWrite[];
  /** API fallback identified by the scanner, independent of browser targets. */
  polyfill?: {
    guard: 'missing' | 'not-function';
    /** Native API checked before installation; e.g. fetch guards a fetch/Request/Response bundle. */
    guardTarget?: string;
  };
}

export interface ManifestWrite {
  target: string;
  operation: string;
  confidence: Confidence;
  location: Location;
  snippet: string;
  /** Detail for this operation, such as the inserted node's tag. */
  detail?: string;
  polyfill?: ManifestFinding['polyfill'];
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

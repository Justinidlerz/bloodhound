import type { ResourceKind, ScriptMode, DependencyMode } from '@bloodhound-rs/manifest/types';
export type * from '@bloodhound-rs/manifest/types';

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
  /** Decode source maps, attach original locations and verify module-state writes against embedded original sources. */
  sourceMaps?: boolean;
  /** Cancels a running scan. The result is a manifest with `status: 'cancelled'`. */
  signal?: AbortSignal;
}

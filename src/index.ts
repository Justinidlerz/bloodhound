/**
 * `@bloodhound-rs/core` — static scanner for global-environment pollution.
 *
 * Give it an HTML page or a JavaScript file (as text, a file path, or a URL)
 * and it returns a `Manifest` describing every write to the global object,
 * `document`, builtin prototypes and host APIs, every imperative DOM or
 * style insertion, and the resource chain that led to each one.
 *
 * All scanning, loading and resolution run in Rust on a worker thread. The
 * functions here only shape arguments and parse the returned JSON.
 */

import {
  scanRaw,
  scanHtmlRaw,
  scanJsRaw,
  version as nativeVersion,
  defaultBudgets,
} from './native.js';
import type { Budgets, Manifest, ScanEntry, ScanOptions, Source } from './types.js';

export type * from './types.js';

/** Scans any number of entries and returns one manifest. */
export async function scan(entries: ScanEntry[], options: ScanOptions = {}): Promise<Manifest> {
  return JSON.parse(await scanRaw(entries, options)) as Manifest;
}

/**
 * Scans an HTML document. A string is treated as page source; supply
 * `options.resourceUrl` so relative script URLs resolve.
 */
export async function scanHtml(
  input: Source | string,
  options: ScanOptions = {},
): Promise<Manifest> {
  return JSON.parse(await scanHtmlRaw(input, options)) as Manifest;
}

/**
 * Scans a JavaScript resource. A string is treated as source text. Set
 * `options.scriptMode` to `'module'` for ESM; the default is `'classic'`.
 */
export async function scanJs(input: Source | string, options: ScanOptions = {}): Promise<Manifest> {
  return JSON.parse(await scanJsRaw(input, options)) as Manifest;
}

/** Scanner, parser and manifest schema versions. */
export function version(): { scanner: string; parser: string; schema: number } {
  return nativeVersion();
}

/** The budgets applied when `options.budgets` is omitted. */
export function defaults(): { budgets: Budgets } {
  return { budgets: defaultBudgets() };
}

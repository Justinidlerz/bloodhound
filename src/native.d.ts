/**
 * Declarations for the napi-rs generated loader (`dist/native.js`).
 *
 * The build overwrites `dist/native.d.ts` with the generated version; this
 * copy exists so `src/index.ts` type-checks before the native build runs.
 * Keep the signatures in sync with the `#[napi]` exports in
 * `native/src/lib.rs`.
 */
import type { Budgets, ScanEntry, ScanOptions, Source } from './types.js';

export declare function scanRaw(entries: ScanEntry[], options?: ScanOptions): Promise<string>;
export declare function scanHtmlRaw(input: Source | string, options?: ScanOptions): Promise<string>;
export declare function scanJsRaw(input: Source | string, options?: ScanOptions): Promise<string>;
export declare function version(): { scanner: string; parser: string; schema: number };
export declare function defaultBudgets(): Budgets;

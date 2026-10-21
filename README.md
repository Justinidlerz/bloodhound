# @bloodhound-rs/core

A Rust SDK that statically scans JavaScript, HTML, and their dependencies for global modifications and DOM/CSS risks. napi-rs generates the npm loader and method declarations. All scanning, fetching, resolution, and reporting run in Rust. The scanner never executes input code.

## Install

```sh
pnpm add @bloodhound-rs/core
```

Requires Node.js 22+. Keep optional dependencies enabled: the package manager installs the native binary for the running Node.js platform and architecture.

| Platform            | Binary package                        |
| ------------------- | ------------------------------------- |
| macOS Apple Silicon | `@bloodhound-rs/core-darwin-arm64`    |
| macOS Intel         | `@bloodhound-rs/core-darwin-x64`      |
| Linux glibc ARM64   | `@bloodhound-rs/core-linux-arm64-gnu` |
| Linux glibc x64     | `@bloodhound-rs/core-linux-x64-gnu`   |
| Windows x64         | `@bloodhound-rs/core-win32-x64-msvc`  |

Linux musl is not supported. Install on the target system instead of copying `node_modules` between platforms.

## Scan source text

```ts
import { scanHtml, scanJs } from '@bloodhound-rs/core';

const manifest = await scanHtml('<script>window.sharedConfig = {};</script>');
console.log(manifest.globals);

const javascript = await scanJs('globalThis.sharedConfig = {};', {
  scriptMode: 'module',
  executionEnvironment: 'window',
});
```

String inputs are source text. For HTML, the scanner reads base elements, import maps, script modes, inline code, and resource links automatically. Recursive scanning is the default. JavaScript callers supply `scriptMode` and `executionEnvironment`; missing context produces coverage gaps.

When the HTML uses relative URLs, supply its original page URL:

```ts
const manifest = await scanHtml(htmlText, {
  resourceUrl: 'https://example.com/page.html',
});
```

Text needs no URL when it contains only inline code or absolute dependencies. Without a document URL or an absolute HTML base, relative references remain unresolved with `missing-base-url`. An attachment filename is not a page URL.

Explicit source objects load files or URLs:

```ts
await scanHtml({ type: 'file', path: './page.html' });
await scanHtml({ type: 'url', url: 'https://example.com/page.html' });
```

`scan(entries, options)` accepts multiple `{ kind, source }` entries. `scanResource(entry, options)` scans one resource and records its dependency edges without following them. These APIs also support CSS through `kind: 'css'`. A content source is `{ type: 'content', content, resourceUrl? }`.

## Compact JSON output

The default result is a `Manifest`:

| Field                     | Contents                                                                              |
| ------------------------- | ------------------------------------------------------------------------------------- |
| `scanStatus`, `runStatus` | Completion, failure, partial coverage, or cancellation                                |
| `summary`                 | Counts, bytes, elapsed time, dependency mode, and omitted read-access count           |
| `sources`, `entries`      | Analyzed sources and entry points, using report-local IDs such as `s1`                |
| `dependencies`            | Dependency edges, resolution results, source locations, and failure reasons           |
| `globals`                 | Global declarations, writes, definitions, and deletes with target paths and locations |
| `findings`                | DOM/CSS findings; global modifications already appear in `globals`                    |
| `unresolved`              | Gaps grouped by source and reason, with exact counts and one `exampleLocation`        |

Internal hashes, repeated context snapshots, read-only accesses, and duplicate global findings are absent from the compact output. Every global modification and DOM/CSS finding is retained. Source IDs identify an analysis in one context; the same resource may have multiple contexts. When multiple global environments occur, `sources[].scope` distinguishes them with report-local numbers.

Lines start at 1. Columns start at 0 and count UTF-16 code units. Inline locations include a position in their containing document. Results are static candidates: they do not establish execution order or prove that a modification occurred at runtime. A partial report remains partial after compaction.

## Text in, JSON stdout

The SDK returns a JavaScript object. The caller chooses stdout or file persistence:

```ts
import { scanHtml } from '@bloodhound-rs/core';

const manifest = await scanHtml(htmlText);
process.stdout.write(JSON.stringify(manifest) + '\n');
```

Runnable examples after building:

```sh
node examples/scan.mjs
node examples/scan-html.mjs
node examples/scan-stdin.mjs < page.html
node examples/scan-stdin.mjs https://example.com/page.html < page.html > manifest.json
```

Each example writes one JSON document to stdout. It does not create output files. `scan-html.mjs` reads a local fixture and uses its file URL to resolve that fixture's relative dependencies. There is no published CLI entry point.

## Scan timings

Enable timings to locate slow stages without expanding the default manifest:

```ts
const started = performance.now();
const manifest = await scanHtml(htmlText, { timings: true });
console.log({ sdkElapsedMs: performance.now() - started });
console.table(manifest.timings?.stages);
console.log(manifest.timings?.resources);
```

`timings.stages` contains exclusive wall times in milliseconds. Nested stages are subtracted from their parents; the values, including `other`, sum to `timings.totalMs`.

| Stage                               | Work measured                                                                                             |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `setup`                             | Read options and prepare the run                                                                          |
| `load`                              | Load resource batches, including HTTP, file/content reads, retries, and worker waits                      |
| `analysis`                          | Analyze batches, including cache lookup, parsing, scope analysis, AST traversal, and internal conversions |
| `records`                           | Build records and hashes, and cache analysis results                                                      |
| `locations`                         | Convert source offsets to line/column and inline/source-map locations                                     |
| `deduplication`                     | Look up existing global targets and access records                                                        |
| `dependencies`                      | Resolve references and enqueue dependencies                                                               |
| `sourceMaps`                        | Load and decode source maps when enabled                                                                  |
| `moduleLinking`                     | Propagate imported constants and compare analysis passes                                                  |
| `checkpoint`                        | Write optional checkpoints                                                                                |
| `finalize`, `validation`, `compact` | Finalize coverage, validate the report, and build the compact manifest                                    |
| `other`                             | Coordinator work outside named stages and instrumentation overhead                                        |

`timings.resources` aggregates work by resource URL and kind across contexts and passes. Each row has coordinator `stages` and worker `tasks` with counts and elapsed milliseconds. JavaScript tasks separate `jsParse`, `jsSemantic`, and `jsVisit`; all three are included in `analysis`. HTML and CSS parsing and traversal are measured together in `analysis`. A `cacheHit` task records reuse of a previous analysis. Worker task times can overlap and must not be added to coordinator stage times or treated as CPU time.

Timing stops after native result construction, before the final JSONL summary, report-file output, JSON serialization, and Node.js conversion. The caller's `performance.now()` measurement includes the SDK call boundary. Timings are available on compact and full results, remain available through `compactReport`, and include work interrupted by scan budgets. A resumed scan reports timings for its current run. Timing uses the existing budgets and analysis rules. Instrumentation overhead counts toward the scan budget.

## Full reports and indexes

Use `reportFormat: 'full'` when querying all reads, cross-resource access paths, or report diffs:

```ts
import { scanJs, GlobalIndex, compactReport } from '@bloodhound-rs/core';

const report = await scanJs(code, {
  scriptMode: 'classic',
  executionEnvironment: 'window',
  reportFormat: 'full',
  scopeKey: 'checkout-page',
});
const index = new GlobalIndex(report);
for (const target of index.findTargets(['sharedConfig'], 'global-object')) {
  console.log(index.sources(target.targetId));
}
const manifest = compactReport(report);
```

`GlobalIndex`, `validateReport`, `indexArtifact`, `adaptReport`, `diffReports`, and `runtimeCandidates` accept full `Report` objects. `compactReport` can also reduce an existing full report without scanning again.

Full reports retain `scopeKey` to separate global environments, `contentHash` to match resource contents, and internal IDs/fingerprints for joins and comparisons. Set a readable `scopeKey` when comparing multiple scans of the same page. These fields are not required to consume the default manifest.

## Optional context and recovery

| Option                                              | Purpose                                                                  |
| --------------------------------------------------- | ------------------------------------------------------------------------ |
| `resourceUrl`, `baseUrl`                            | Original resource URL and optional document-base fallback                |
| `importMap`, `importMapBaseUrl`                     | Page mappings when scanning JavaScript outside its HTML document         |
| `resourceMappings`                                  | Substitute local files or text for resolved URLs during offline scanning |
| `budgets`, `signal`                                 | Bound work or cancel it with an `AbortSignal`                            |
| `access`                                            | Restrict protocols, hosts, and file roots                                |
| `sourceMaps`                                        | Add original-source locations when maps are available                    |
| `requestHeaders`, `proxy`                           | Configure HTTP fetching                                                  |
| `checkpointPath`, `resumeScan(checkpoint, options)` | Persist recovery state and scan the saved entries again                  |

HTML import maps and resource lists require no duplicate parameters. External modules resolve imports against their own URL. Browser module resolution does not search `node_modules` or infer extensions.

No report files are written by default. Optional `reportPath` persists the selected output format; `jsonlPath` writes detailed incremental records. Checkpoints retain full analysis state regardless of output format. Authentication headers and proxy settings must be supplied again when resuming. See [Support](docs/support.md) for analysis and recovery limits.

## Development

Requires pnpm 10.20.0, Rust 1.97+, and platform C/C++ build tools.

```sh
pnpm install --frozen-lockfile
pnpm build
pnpm test
```

`native/src/engine/` contains loading and scheduling; `js/` contains evaluation, bindings, and effect analysis; `markup/` contains HTML/CSS parsing; `report/` builds compact manifests. `src/` contains type declarations only. napi-rs generates `dist/index.js` and `dist/index.d.ts`.

Changesets keeps core and the five binary packages at the same version. CI verifies binary package publication before publishing core. See [Testing](docs/testing.md), [Technical design](docs/technical-design.md), [Verification](docs/verification.md), and [Releasing](docs/releasing.md).

# @bloodhound-rs/rollup-plugin

Scans the emitted bundle with [`@bloodhound-rs/core`](../../README.md) during `generateBundle`, before files are written. Tested with Rollup 4 and Rolldown 1. The plugin uses hooks compatible with Vite builds. It reports global writes, DOM insertions and style injections as warnings or errors, and emits a Manifest v2 beside the bundle. CSS resources are listed as dependencies; their contents are not scanned.

## Install

```sh
pnpm add -D @bloodhound-rs/rollup-plugin
```

Requires Node.js 22.12+ and a supported native platform for `@bloodhound-rs/core`.

Both ESM static imports and CommonJS loading are supported:

```js
const { bloodhound } = require('@bloodhound-rs/rollup-plugin');
```

Both entries use the same synchronous ESM module graph, including core and its native loader. CommonJS loading uses Node.js 22.12+'s native `require(esm)` support; consumers do not need dynamic imports. For TypeScript CommonJS source with static imports, use TypeScript 5.8+ with `module: 'NodeNext'`.

## Usage

```js
// rollup.config.mjs or rolldown.config.mjs
import { bloodhound } from '@bloodhound-rs/rollup-plugin';

export default {
  input: 'src/main.js',
  output: { dir: 'dist', format: 'es', sourcemap: 'hidden' },
  plugins: [
    bloodhound({
      publicPath: 'https://cdn.example.com/app/',
      allowGlobals: ['__APP_VERSION__', 'dataLayer', 'dataLayer.*'],
      globals: 'error',
    }),
  ],
};
```

```ts
// vite.config.ts
import { defineConfig } from 'vite';
import { bloodhound } from '@bloodhound-rs/rollup-plugin';

export default defineConfig({
  base: '/app/',
  build: { sourcemap: 'hidden' },
  plugins: [bloodhound({ publicPath: '/app/' })],
});
```

The same plugin instance is valid in all three bundlers. Scanning uses `generateBundle`, `this.emitFile`, `this.warn`, and `this.error`. Vite supplies its root through `configResolved` and reads `apply: 'build'` and `enforce: 'post'`; Rollup and Rolldown use the working directory or the explicit `root` option.

## What happens during `generateBundle`

1. Every emitted chunk, HTML, and CSS asset is registered as an in-memory resource addressed by `publicPath + fileName`. Source maps use the bundler's `sourcemapFileName`, with a virtual address when no map file is emitted. The scanned copy gets a final directive pointing to `chunk.map`; custom filenames and inline/hidden maps work without changing emitted code.
2. Entries are the emitted HTML files, or the entry chunks when no HTML is emitted. JavaScript imports and HTML script references resolve against the in-memory bundle. Stylesheet links are listed without fetching or scanning CSS. References outside the bundle appear as coverage gaps by default.
3. When `onManifest` is configured, it receives Manifest v2 and handles collection or writing; the plugin does not emit a manifest asset. Otherwise the manifest is emitted as `bloodhound.manifest.json` in the output directory, unless `manifestFileName` is `false`. Resource records include project-relative output paths and source-map associations for original-source previews. Existing map assets are reused; inline maps receive a companion asset under `bloodhound/sourcemaps/` in either mode.
4. Each global mutation and finding becomes a warning with `loc` pointing at the original source when a source map is available, otherwise at the emitted file. Inline scripts and styles point at their containing document.
5. When any `error`-severity diagnostic remains, the build fails with a summary. The callback or default manifest emission runs before this check. Async callbacks are awaited; callback failures fail the build without falling back to default manifest emission.

## Options

| Option             | Default                                           | Purpose                                                                                                            |
| ------------------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `root`             | Vite root or `process.cwd()`                      | Project root for output and original-source preview paths.                                                         |
| `publicPath`       | `https://bundle.local/`                           | Origin and prefix used to address emitted files. Match Vite `base` or the CDN prefix so `<script src>` resolves.   |
| `entries`          | HTML assets, else entries                         | `(fileName, item) => boolean` selecting scan entries.                                                              |
| `scriptMode`       | from `output.format`                              | `module` for `es`, otherwise `classic`.                                                                            |
| `scanOptions`      | `{ sourceMaps: true, access: { protocols: [] } }` | Extra `ScanOptions`; set `access` to allow fetching already deployed shared resources.                             |
| `manifestFileName` | `bloodhound.manifest.json`                        | Default output filename when no callback is configured; `false` disables it. Also the base for relative map URLs.  |
| `globals`          | `warn`                                            | `off`, `warn`, or `error` for global declarations, writes, definitions, and deletes.                               |
| `findings`         | `warn`                                            | Severity for DOM insertions, style injections, global listeners and ESM module-state writes.                       |
| `rules`            | `{}`                                              | Per-rule severity overrides, e.g. `{ 'host-api-override': 'error' }`.                                              |
| `coverage`         | `warn`                                            | Severity when the scan is `partial` or `failed`.                                                                   |
| `allowGlobals`     | `[]`                                              | Expected global paths: exact strings, `prefix.*`, `*` wildcards, or RegExp.                                        |
| `ignoreSources`    | `[/node_modules/]`                                | Drop diagnostics whose source-mapped origin matches.                                                               |
| `filter`           |                                                   | Final `(diagnostic) => boolean`.                                                                                   |
| `onManifest`       |                                                   | `(manifest, { diagnostics, bundle, outputOptions })` takes ownership of the result instead of emitting a manifest. |

`diagnosticsFromManifest(manifest, options, fileNames)` is exported for post-processing a saved manifest with the same filters.

Diagnostics expose the canonical rule and target (`global-write`, `window.sharedConfig`). `allowGlobals` accepts canonical paths and the shorthand `sharedConfig`. The manifest retains all findings even when warning filters suppress some diagnostics. Plugin severity defaults to warnings; use `rules` or category options to enforce your release policy independently of the SDK's severity labels.

ESM output also reports `module-state-write` when functions or callbacks assign module-level `let` variables or mutate their properties. These values survive page remounts because cached module initialization does not rerun. The diagnostic points to the write, and source maps can resolve its original source. Configure it with `rules: { 'module-state-write': 'warn' }` (the default). A reset function alone does not suppress this warning; the scanner does not prove page lifecycle execution.

Emit sourcemaps with `sourcesContent` to distinguish original module writes from bundler-generated initialization assignments. The scanner checks the original module's scopes and write locations, excluding declarations while retaining later mutations of `let` bindings. Original `var` and `const` bindings are excluded, including mutations of `const` objects; `let` bindings compiled to `var` remain eligible. The plugin enables `sourceMaps` by default; without usable original source content, only generated `let` bindings are eligible.

Function declarations and bindings initialized with function/arrow literals are excluded from `module-state-write`. Variables assigned a callback after initialization remain eligible. Recognized global-object aliases, including `document.defaultView` and simple synchronous IIFE return values, use the global mutation rules.

`module-state-write` ignores resources under `node_modules`, including dependency code identified by original sourcemap paths inside an application bundle. This path-based exclusion does not require `sourcesContent` and does not affect other rules.

Writes sharing a declaration within the same resource are grouped into one finding. `finding.writes` contains all write sites when there are multiple, while top-level position fields retain the first write. Manifest summaries and plugin diagnostic counts use the grouped records.

Other rules group findings by resource, rule, original source file and line, falling back to the generated line without a mapped position. Each operation retains its position, target, snippet, detail and polyfill metadata in `writes`. Polyfill and ordinary findings remain separate so the visibility toggle only hides polyfills.

`allowGlobals` filters each operation before producing a grouped diagnostic. Its `targets` lists the remaining targets, and its location points to the first remaining operation. The manifest retains all operations.

## Polyfill records

The Rust scanner identifies functions installed behind a missing-native API check. It handles `if`, conditional expressions and short-circuit checks for both JavaScript builtins and browser APIs, including Canvas, fetch and Request. A single guard can cover related API installations, such as `if (!window.fetch) { window.fetch = fetchPolyfill; window.Request = RequestPolyfill; }`.

Findings carry `polyfill.guard` and `polyfill.guardTarget` to identify the installation condition. Playground hides all identified polyfills by default with a show/hide toggle. No browser targets or compatibility database are required. The same metadata is available from the core SDK and in the plugin's callback or emitted manifest.

Unconditional overrides, monitoring wrappers and checks that cannot be resolved remain visible. Package names alone do not classify code as a polyfill. Existing manifests without this metadata need a new scan. Raw findings, summary counts, diagnostics, source maps and emitted JavaScript are preserved.

## Receive the manifest in a callback

```js
const manifests = [];

const plugin = bloodhound({
  onManifest(manifest, { diagnostics, outputOptions }) {
    manifests.push(manifest);
    // The build coordinator owns aggregation and the final file write.
  },
});
```

Supplying `onManifest` is enough to disable automatic manifest output; there is no need to also set `manifestFileName: false`. The callback runs once per scanned output and receives the full manifest, including source-map associations. Warnings, coverage reporting and blocking checks still run. In multi-process builds, collect the result in the child and return serializable data to the parent; create the callback inside each child configuration.

Without `onManifest`, `bloodhound()` emits the default file and `bloodhound({ manifestFileName: 'reports/scan.json' })` emits that custom path. Setting `manifestFileName: false` without a callback performs the scan and diagnostics without writing a manifest.

When the receiver writes the report elsewhere, it must update relative `sourceMap.url` references to match that location. `manifestFileName` still sets their base within each output directory; when false, the base is `bloodhound.manifest.json`. The callback does not disable source-map assets needed for previews.

## Notes

- Enable `sourcemap: true | 'hidden'` to locate diagnostics in source files and to make `ignoreSources` effective. Hidden maps are not referenced by the emitted code; the plugin attaches them only to the scanned copy.
- Results are static candidates. Use `allowGlobals` for intentional globals and `globals: 'error'` once the remaining list is empty.
- Multiple `output` configurations run the scan once per output.

## Preview original source

Open the manifest in the [Playground](../../playground/README.md), with `--project` pointing at the same project root used by the plugin. Select a finding and choose **Preview original source**. The viewer maps the generated position to the original file and reads its current contents from that project. If the file is unavailable, embedded `sourcesContent` provides the build snapshot.

| Manifest source field   | Meaning                                                                                                     |
| ----------------------- | ----------------------------------------------------------------------------------------------------------- |
| `file`                  | Emitted file relative to `root`                                                                             |
| `sourceMap.url`         | Companion map relative to the manifest file                                                                 |
| `sourceMap.file`        | Companion map relative to `root`, when inside the project                                                   |
| `sourceMap.sourcePaths` | Original project-relative paths in the flattened map's source order; `null` for external or virtual sources |

Keep the manifest and its referenced maps together when moving build artifacts. Paths outside `root` are not added as local preview paths. `sourcemapExcludeSources` remains respected: the plugin does not read source files into the map. Without embedded content, previews require the project files. Without a map, no source-map association is added.

## Example

See [`examples/rollup`](../../examples/rollup) in the repository: `pnpm example:rollup` and `pnpm example:rolldown` build the same sources with each bundler and report identical diagnostics. `BLOODHOUND_STRICT=1` turns global mutations into build errors.

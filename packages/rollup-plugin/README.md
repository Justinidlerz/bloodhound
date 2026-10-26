# @bloodhound-rs/rollup-plugin

Scans the emitted bundle with [`@bloodhound-rs/core`](../../README.md) during `generateBundle`, before files are written. Tested with Rollup 4 and Rolldown 1. The plugin uses hooks compatible with Vite builds. It reports global writes, DOM insertions and style injections as warnings or errors, and emits a Manifest v2 beside the bundle. CSS resources are listed as dependencies; their contents are not scanned.

## Install

```sh
pnpm add -D @bloodhound-rs/rollup-plugin
```

Requires Node.js 22+ and a supported native platform for `@bloodhound-rs/core`.

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

The same plugin instance is valid in all three bundlers; it only uses `generateBundle`, `this.emitFile`, `this.warn`, and `this.error`. Vite reads `apply: 'build'` and `enforce: 'post'`; Rollup and Rolldown ignore them.

## What happens during `generateBundle`

1. Every emitted chunk, HTML, and CSS asset is registered as an in-memory resource addressed by `publicPath + fileName`. Source maps use the bundler's `sourcemapFileName`, with a virtual address when no map file is emitted. The scanned copy gets a final directive pointing to `chunk.map`; custom filenames and inline/hidden maps work without changing emitted code.
2. Entries are the emitted HTML files, or the entry chunks when no HTML is emitted. JavaScript imports and HTML script references resolve against the in-memory bundle. Stylesheet links are listed without fetching or scanning CSS. References outside the bundle appear as coverage gaps by default.
3. Manifest v2 is emitted as `bloodhound.manifest.json` in the output directory.
4. Each global mutation and finding becomes a warning with `loc` pointing at the original source when a source map is available, otherwise at the emitted file. Inline scripts and styles point at their containing document.
5. When any `error`-severity diagnostic remains, the build fails with a summary. The manifest is emitted before the failure.

## Options

| Option             | Default                                           | Purpose                                                                                                          |
| ------------------ | ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `publicPath`       | `https://bundle.local/`                           | Origin and prefix used to address emitted files. Match Vite `base` or the CDN prefix so `<script src>` resolves. |
| `entries`          | HTML assets, else entries                         | `(fileName, item) => boolean` selecting scan entries.                                                            |
| `scriptMode`       | from `output.format`                              | `module` for `es`, otherwise `classic`.                                                                          |
| `scanOptions`      | `{ sourceMaps: true, access: { protocols: [] } }` | Extra `ScanOptions`; set `access` to allow fetching already deployed shared resources.                           |
| `manifestFileName` | `bloodhound.manifest.json`                        | `false` disables the emitted manifest.                                                                           |
| `globals`          | `warn`                                            | `off`, `warn`, or `error` for global declarations, writes, definitions, and deletes.                             |
| `findings`         | `warn`                                            | Severity for DOM insertions, style injections and global listeners.                                              |
| `rules`            | `{}`                                              | Per-rule severity overrides, e.g. `{ 'host-api-override': 'error' }`.                                            |
| `coverage`         | `warn`                                            | Severity when the scan is `partial` or `failed`.                                                                 |
| `allowGlobals`     | `[]`                                              | Expected global paths: exact strings, `prefix.*`, `*` wildcards, or RegExp.                                      |
| `ignoreSources`    | `[/node_modules/]`                                | Drop diagnostics whose source-mapped origin matches.                                                             |
| `filter`           |                                                   | Final `(diagnostic) => boolean`.                                                                                 |
| `onManifest`       |                                                   | `(manifest, { diagnostics, bundle, outputOptions })` after each scan; useful for custom reporting or baselines.  |

`diagnosticsFromManifest(manifest, options, fileNames)` is exported for post-processing a saved manifest with the same filters.

Diagnostics expose the canonical rule and target (`global-write`, `window.sharedConfig`). `allowGlobals` accepts canonical paths and the shorthand `sharedConfig`. The manifest retains all findings even when warning filters suppress some diagnostics. Plugin severity defaults to warnings; use `rules` or category options to enforce your release policy independently of the SDK's severity labels.

## Notes

- Enable `sourcemap: true | 'hidden'` to locate diagnostics in source files and to make `ignoreSources` effective. Hidden maps are not referenced by the emitted code; the plugin attaches them only to the scanned copy.
- Results are static candidates. Use `allowGlobals` for intentional globals and `globals: 'error'` once the remaining list is empty.
- Multiple `output` configurations run the scan once per output.

## Example

See [`examples/rollup`](../../examples/rollup) in the repository: `pnpm example:rollup` and `pnpm example:rolldown` build the same sources with each bundler and report identical diagnostics. `BLOODHOUND_STRICT=1` turns global mutations into build errors.

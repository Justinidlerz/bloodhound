# @bloodhound-rs/manifest

Browser-safe Manifest v2 types, parsing, indexing and source-map previews. This package has no React, DOM or native scanner dependency.

```ts
import { parseManifest, ManifestIndex } from '@bloodhound-rs/manifest';

const { manifest, notices } = parseManifest(jsonText);
const index = new ManifestIndex(manifest);
const findings = index.rows.findings;
const dependencies = index.outgoing.get(manifest.entries[0]);
```

| Export                                                   | Use                                                                                     |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `parseManifest(text)`                                    | Parse and validate display inputs, returning `{ manifest, notices }`                    |
| `ManifestIndex`                                          | Build record/source indexes, group/filter records and traverse dependency relationships |
| `filterManifestBySourcePaths(manifest, excludePatterns)` | Remove trusted source paths and their records using regular expressions                 |
| `OriginalSources(manifestUrl?, workspace?, loadText?)`   | Lazily resolve source maps and original code; holds links and caches for one report     |
| `viewerTarget`, `codeWindow`                             | Resolve generated-file previews and bounded highlighted code excerpts                   |
| `fetchText`, `defaultLoadText`                           | Read public text resources without credentials or code execution                        |
| `Manifest`, `Location`, `TextLoader`                     | Shared report and loading types                                                         |

```ts
import { OriginalSources } from '@bloodhound-rs/manifest';

const originals = new OriginalSources(manifestUrl, undefined, loadText);
const finding = manifest.findings[0];
const source = manifest.sources.find((source) => source.id === finding.source);
if (source?.sourceMap) {
  await originals.preview('selected-finding', source, finding.location);
  const preview = originals.previews.get('selected-finding');
}
```

`loadText(url, { kind, file, limit })` returns `{ text, url }`. Return the absolute resource URL for map-relative path resolution. The host implements authentication, artifact lookup and byte limits. Without a loader, HTTP(S) reads use the built-in fetcher. `workspace` enables the local Playground's project file endpoint.

`OriginalSources` does not fetch until `preview` is called. An absent map association produces no preview or request. `link(source, urlOrPath)` and `unlink(source)` change session-only associations. Create a new instance for each report/build.

Core continues to export the report types. Browser consumers import this package directly to avoid loading the native scanner. Types are also available from `@bloodhound-rs/manifest/types`.

## Filter trusted source paths

```ts
import { filterManifestBySourcePaths } from '@bloodhound-rs/manifest/utils';

const filteredManifest = filterManifestBySourcePaths(manifest, [
  /node_modules\/@module-federation(?:\/|$)/,
  /node_modules\/@omni-core\/vmok-loader(?:\/|$)/,
]);
```

Pass `filteredManifest` to `ManifestViewer` or `ManifestIndex`. The function is also exported from the package root. It accepts `readonly (string | RegExp)[]`; strings are JavaScript regular-expression source text, such as `'node_modules/@module-federation(/|$)'`. RegExp flags are preserved, repeated matching does not change caller-owned `lastIndex`, and invalid string patterns throw `SyntaxError`. Paths use forward slashes, including paths recorded on Windows. No package names are excluded by default.

| Report data           | Filtering behavior                                                                                                                                                                                                                                               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Findings              | Match `location.original.source`. Filter grouped `writes` individually and update the representative operation and confidence from the remaining writes. Remove empty groups and matching declaration metadata.                                                  |
| Resources             | Match `file` or `url`, or remove a chunk when every `sourceMap.sourcePaths` entry is known and matches. Remove inline children of removed resources. Mixed chunks and unknown source paths remain.                                                               |
| Dependencies and gaps | Remove links to/from removed resources, dependencies with matching target URLs/specifiers or original locations, and single-occurrence gaps with matching original locations. Keep scan-wide gaps and grouped gaps whose remaining origins cannot be determined. |
| Graph and counts      | Remove deleted entries, rebuild referrers and shortest entry chains, and recalculate record counts in `summary`. Unreachable retained resources have a chain containing only their own ID.                                                                       |

The function does not mutate the input report or regexes. Trust applies to matching paths, not transitively to other packages imported by them. A package merged into a mixed chunk can only be identified per record when the report contains original source locations; unmapped records remain.

Filtering does not rescan or rewrite build artifacts. `status`, `summary.bytes`, and `summary.elapsedMs` retain the original scan result and cost. Matching paths in a retained `sourceMap.sourcePaths` array become `null` without shifting source indexes; source-map files and preview content remain unchanged. The returned report remains suitable for parsing, indexing and source previews of retained records.

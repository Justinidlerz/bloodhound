# @bloodhound-rs/manifest

Browser-safe Manifest v2 types, parsing, indexing and source-map previews. This package has no React, DOM or native scanner dependency.

```ts
import { parseManifest, ManifestIndex } from '@bloodhound-rs/manifest';

const { manifest, notices } = parseManifest(jsonText);
const index = new ManifestIndex(manifest);
const findings = index.rows.findings;
const dependencies = index.outgoing.get(manifest.entries[0]);
```

| Export                                                 | Use                                                                                     |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------- |
| `parseManifest(text)`                                  | Parse and validate display inputs, returning `{ manifest, notices }`                    |
| `ManifestIndex`                                        | Build record/source indexes, group/filter records and traverse dependency relationships |
| `OriginalSources(manifestUrl?, workspace?, loadText?)` | Lazily resolve source maps and original code; holds links and caches for one report     |
| `viewerTarget`, `codeWindow`                           | Resolve generated-file previews and bounded highlighted code excerpts                   |
| `fetchText`, `defaultLoadText`                         | Read public text resources without credentials or code execution                        |
| `Manifest`, `Location`, `TextLoader`                   | Shared report and loading types                                                         |

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

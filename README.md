# @bloodhound-rs/core

A Rust-backed SDK that statically scans HTML and JavaScript for writes to shared globals, DOM insertion, and style injection. The result is a JSON-serializable Manifest v2 with source locations, dependency edges, severity, and a shortest entry-to-source chain. Input code is never executed.

CSS files and inline `<style>` blocks are discovered and listed. CSS selectors, custom properties, animations, keyframes, and CSS imports are not analyzed. Direct writes to document-level inline styles remain detectable.

## Install

```sh
pnpm add @bloodhound-rs/core
```

Requires Node.js 22+. Keep optional dependencies enabled so the native package for the current platform is installed. Supported targets are macOS arm64/x64, Linux glibc arm64/x64, and Windows x64. Linux musl is not supported.

## SDK

```ts
import { scan, scanHtml, scanJs, defaults, version } from '@bloodhound-rs/core';

const manifest = await scanHtml(htmlText, {
  resourceUrl: 'https://example.com/page.html',
  budgets: { maxResources: 200, timeoutMs: 60_000 },
});

for (const finding of manifest.findings) {
  const source = manifest.sources.find((s) => s.id === finding.source);
  console.log(finding.severity, finding.rule, finding.target, source?.chain, finding.location);
}

await scanJs('window.shared = 1', { scriptMode: 'module' });
await scanHtml({ type: 'file', path: './page.html' });
await scanHtml({ type: 'url', url: 'https://example.com/page.html' });
await scan([{ kind: 'js', source: { type: 'file', path: './app.js' }, scriptMode: 'module' }]);

process.stdout.write(JSON.stringify(manifest) + '\n');
```

Strings always mean source text. File paths and URLs use explicit source objects. Content sources accept `{ type: 'content', content, resourceUrl? }`. Supply a logical URL or `baseUrl` to resolve relative references in supplied text. Absolute dependencies and inline scripts can be scanned without a URL.

`scan`, `scanHtml`, and `scanJs` return the same `Promise<Manifest>`. JavaScript defaults to `classic`; choose `module` for ESM. HTML script tags determine their own mode. Recursive scanning is the default. `dependencyMode: 'single'` analyzes entries only and lists their dependencies without following them, including inline HTML scripts.

## Findings

| Rule                   | Examples                                                                        | Default severity |
| ---------------------- | ------------------------------------------------------------------------------- | ---------------- |
| `global-write`         | `window.config = …`, classic top-level `var`, sloppy implicit globals           | warning          |
| `document-write`       | `document.body.className = …`, document attributes                              | warning          |
| `builtin-mutation`     | `Array.prototype.x = …`, `Object.assign = …`                                    | error            |
| `host-api-override`    | `window.fetch = …`, `history.pushState = …`                                     | error            |
| `dom-insertion`        | `document.body.appendChild(…)`, selected element insertion, `document.write(…)` | warning          |
| `style-injection`      | injected `<style>` or stylesheet `<link>`, adopted stylesheets                  | warning          |
| `document-style-write` | `document.body.style.overflow = …`                                              | warning          |
| `global-listener`      | `window.addEventListener(…)`, document listeners                                | warning          |
| `html-global-style`    | `<html style="…">`, `<body style="…">`                                          | info             |

Ordinary element `style=` attributes in SSR markup are not findings. Expected browser state changes such as `document.title`, `document.cookie`, and navigation are excluded. A severity label is a default consumer policy, not proof that code is harmful.

An inserted node is suppressed when it is removed later in the same straight-line block, or inside a returned cleanup function. A listener is suppressed when the same file contains a removal with the same event and handler reference. Conditions, source order, receiver and capture do not affect listener pairing. Handler aliases and arguments passed to local helpers are resolved; matching an event alone is insufficient. Inline anonymous callbacks remain outside the listener rule, and `once`/`signal` options count as their own removal mechanisms. Pairing does not prove that teardown executes.

The scan follows scripts belonging to the current document. Iframe and `srcdoc` references are listed as `not-followed / iframe-isolated-global`; their child documents and scripts are not loaded or analyzed. SVG/MathML contents are skipped with a coverage gap, and ordinary HTML after a self-closing foreign element remains eligible for scanning.

## Manifest v2

| Field                             | Meaning                                                                              |
| --------------------------------- | ------------------------------------------------------------------------------------ |
| `schemaVersion`, `scannerVersion` | Format and scanner versions                                                          |
| `status`                          | `completed`, `partial`, `failed`, or `cancelled`                                     |
| `dependencyMode`                  | `single` or `recursive`                                                              |
| `summary`                         | Source, finding, severity, dependency and gap counts; bytes and scan duration        |
| `entries`                         | Entry source IDs, in request order                                                   |
| `sources`                         | Logical URL, kind, mode, status, hash, referrers and shortest source chain           |
| `findings`                        | Rule, severity, target, operation, confidence, location, snippet and optional detail |
| `dependencies`                    | Source/target IDs, specifier, URL, status, reason and discovery location             |
| `gaps`                            | Coverage limits grouped by source and reason, with occurrence counts                 |

Lines are 1-based, columns are 0-based UTF-16 units. Inline findings use fragment-relative coordinates; `source.inline` gives their origin in the parent. For a first-line finding, add the origin column; for later lines, use the finding column unchanged. The bundler plugin performs this conversion automatically. Source maps optionally add `location.original`.

CSS dependency edges use `not-followed` with `reason: 'css-not-analyzed'`. CSS dependencies do not create analyzed source records. An explicit `kind: 'css'` entry is loaded and listed with no findings. Unloaded or unparseable JavaScript produces unresolved edges and coverage gaps.

## Offline scans, limits and cancellation

```ts
const controller = new AbortController();
const manifest = await scanJs('import "./shared.js";', {
  scriptMode: 'module',
  resourceUrl: 'https://example.com/app.js',
  access: { protocols: [] },
  resourceMappings: {
    'https://example.com/shared.js': { type: 'content', content: 'window.shared = 1;' },
  },
  sourceMaps: true,
  signal: controller.signal,
});
```

`resourceMappings` supplies content or local files for resolved URLs, including source maps. `access` can restrict schemes, hosts and file roots. `requestHeaders` is keyed by host and optional port. Redirects are checked at every hop. URLs with embedded credentials are rejected.

`excludeUrls` takes regex source strings (Rust regex syntax, no lookaround) such as `['/vendor/', '\\.min\\.js($|\\?)']`. A recursively discovered dependency whose resolved logical URL matches is not loaded or analyzed. It stays visible in `dependencies` as `not-followed` with `reason: 'excluded'` and creates no source record. If an entry itself matches, the scan rejects instead of returning an empty manifest. Inline scripts are not matched.

`defaults().budgets` reports resource count, byte, depth, AST-step, candidate, concurrency and timeout limits. Workers each load then analyze one resource; their count is `min(concurrency, analysisConcurrency)`. Cancellation is cooperative. A blocking network request can take up to its request timeout to return. Budget stops and unknown dynamic dependencies remain visible in the manifest.

The scanner tracks local aliases, IIFE parameters, finite string candidates and browser import maps. It does not infer arbitrary function results or cross-module exported values. Worker environments, foreign HTML content and late import maps currently produce explicit coverage gaps. See [supported behavior and limits](docs/support.md).

## Rollup / Rolldown

```js
import { bloodhound } from '@bloodhound-rs/rollup-plugin';

export default {
  input: 'src/main.js',
  output: { dir: 'dist', format: 'es', sourcemap: 'hidden' },
  plugins: [
    bloodhound({
      allowGlobals: ['window.__APP_VERSION__'],
      rules: { 'host-api-override': 'error', 'builtin-mutation': 'error' },
    }),
  ],
};
```

The plugin scans emitted HTML or entry chunks in memory, follows bundle dependencies, emits `bloodhound.manifest.json`, and maps diagnostics back to original sources. It runs offline by default. See the [plugin documentation](packages/rollup-plugin/README.md).

## Manifest Playground

Run `pnpm playground` in this repository and open `http://127.0.0.1:4173`. Drop a Manifest v2 JSON file to explore findings, source files, dependency relationships and coverage gaps. Group by rule/type, file, URL folder or entry dependency tree; select any record to see its snippet, location and related files.

The viewer processes files locally in the browser. It displays the manifest's evidence and does not infer missing listener cleanup or business application ownership. See the [Playground documentation](playground/README.md) for navigation, build commands and interpretation.

## Development and migration

```sh
pnpm install --frozen-lockfile
pnpm build
pnpm test
pnpm example
pnpm example:html
pnpm example:rollup
pnpm example:rolldown
```

The build needs Rust 1.97+ for Oxc 0.154.0. [Architecture](docs/technical-design.md), [tests](docs/testing.md), [verification](docs/verification.md), and [release process](docs/releasing.md) describe the implementation and checks.

Manifest v2 replaces the previous full/compact reports. Global mutations now appear in `findings`; use `status` instead of `scanStatus`, and `gaps` for analysis coverage. The old query/index, checkpoint/resume, report adapters, runtime-candidate helpers, timings, and JSONL APIs are removed. Use `dependencyMode: 'single'` instead of `scanResource`. `version()` replaces the old version helpers. This is a breaking API change.

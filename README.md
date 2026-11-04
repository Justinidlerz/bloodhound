# @bloodhound-rs/core

A Rust-backed SDK that statically scans HTML and JavaScript for writes to shared globals, DOM insertion, and style injection. The result is a JSON-serializable Manifest v2 with source locations, dependency edges, severity, and a shortest entry-to-source chain. Input code is never executed.

CSS files and inline `<style>` blocks are discovered and listed. CSS selectors, custom properties, animations, keyframes, and CSS imports are not analyzed. Direct writes to document-level inline styles remain detectable.

## Install

```sh
pnpm add @bloodhound-rs/core
```

Requires Node.js 22.12+. Keep optional dependencies enabled so the native package for the current platform is installed. Supported targets are macOS arm64/x64, Linux glibc arm64/x64, and Windows x64. Linux musl is not supported.

ESM `import` and CommonJS `require('@bloodhound-rs/core')` load the same synchronous module graph. CommonJS loading uses Node.js 22.12+'s native `require(esm)` support; no dynamic import or initialization wrapper is needed.

## Embed the Playground

```tsx
import { ManifestViewer, ManifestPlayground } from '@bloodhound-rs/playground';
import '@bloodhound-rs/playground/styles.css';

// Platform-owned report and resource selection.
<ManifestViewer manifest={manifest} components={components} loadText={loadText} />;

// Includes the file/URL selector.
<ManifestPlayground components={components} loadText={loadText} />;
```

Install `@bloodhound-rs/playground` in a React 18/19 host. The package supports injected UI components and source loading, and exports headless parsing through `@bloodhound-rs/playground/manifest`. It is built and versioned alongside the scanner packages through Changesets. See [Playground integration](packages/playground/README.md) for the API.

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

| Rule                   | Examples                                                                           | Default severity |
| ---------------------- | ---------------------------------------------------------------------------------- | ---------------- |
| `global-write`         | `window.config = …`, classic top-level `var`, sloppy implicit globals              | warning          |
| `document-write`       | `document.body.className = …`, document attributes                                 | warning          |
| `builtin-mutation`     | `Array.prototype.x = …`, `Object.assign = …`                                       | error            |
| `host-api-override`    | `window.fetch = …`, `history.pushState = …`                                        | error            |
| `dom-insertion`        | `document.body.appendChild(…)`, selected element insertion, `document.write(…)`    | warning          |
| `style-injection`      | injected `<style>` or stylesheet `<link>`, adopted stylesheets                     | warning          |
| `document-style-write` | `document.body.style.overflow = …`                                                 | warning          |
| `global-listener`      | `window.addEventListener(…)`, document listeners                                   | warning          |
| `module-state-write`   | module state inherited by React state or suppressing component/model state updates | warning          |
| `html-global-style`    | `<html style="…">`, `<body style="…">`                                             | info             |

Ordinary element `style=` attributes in SSR markup are not findings. Expected browser state changes such as `document.title`, `document.cookie`, and navigation are excluded. A severity label is a default consumer policy, not proof that code is harmful.

Assignments through recognized native state setters are excluded from pollution findings:

| State                       | Examples                                                                                                                                                                                                                                                    |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Scroll position             | `document.body.scrollTop`, `document.documentElement.scrollLeft`, `document.scrollingElement.scrollTop`, and aliases of these elements                                                                                                                      |
| Control value and selection | `value`, `valueAsDate`, `valueAsNumber`, `checked`, `indeterminate`, `selected`, `selectedIndex`, `selectionStart`, `selectionEnd`, `selectionDirection`, `files` on a selected control, `document.activeElement`, or `document.forms[name].elements[name]` |
| Media playback              | `currentTime`, `volume`, `muted`, `playbackRate` on a selected media element or `document.activeElement`                                                                                                                                                    |
| Current event               | `window.event.cancelBubble`, `window.event.returnValue`, and aliases of the current event                                                                                                                                                                   |
| Document and navigation     | Existing document state setters and writable `location` components such as `href`, `pathname`, `search`, and `hash`                                                                                                                                         |

This applies to direct/compound assignments, increment/decrement, `Object.assign`, and `Reflect.set` when the receiver is recognized. Property deletion, `Object.defineProperty`/`defineProperties`, method replacement, prototype mutation, and writes to custom global objects remain eligible for findings. Global `style`, `className`, `classList`, `dataset`, and HTML insertion retain their existing rules. Unknown computed property names are not treated as known state setters. See the [scrolling setters](https://drafts.csswg.org/cssom-view/#dom-element-scrolltop), [control state APIs](https://html.spec.whatwg.org/multipage/input.html#common-input-element-apis), [media APIs](https://html.spec.whatwg.org/multipage/media.html#media-elements), and [event state setters](https://dom.spec.whatwg.org/#interface-event).

Recognized acquire/release pairs are excluded automatically:

| Effect                     | Pairing evidence                                                                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Document attributes        | `setAttribute` / `removeAttribute` on the same element and attribute, including opposite conditional branches                                    |
| Classes                    | `classList.add` / `classList.remove` with matching tokens; every token in a multi-token call must pair                                           |
| Inline styles              | The same property is removed or reset to an empty value, `auto`, `unset`, `initial`, `revert` or `revert-layer`                                  |
| DOM and stylesheet nodes   | The same node is inserted and removed, including `parentNode` / `parentElement`, callback cleanup, stable aliases and recognized instance fields |
| Global and host properties | A captured original value is written back to the same property, including keyed method storage and local instance forwarding helpers             |

Pairing follows lexical identities and class/prototype owners. Different attributes, nodes, classes, handlers and shadowed bindings do not pair. A removal before a later insertion in the same synchronous execution scope leaves that insertion visible. Capturing a value after overwriting it, or restoring it before the replacement in the same scope, is not restoration. Restoring an original object also discards writes to its replacement's own properties. A React unmount or an arbitrary `destroy()` call alone does not establish DOM removal.

With `sourceMaps: true`, embedded original sources also participate in pairing; generated operations from different mapped source files are kept separate. Missing source content, unresolved helpers and unsupported identities retain the finding. A pair recognizes an available release path, including conditional or callback paths; it does not prove that teardown runs or restores every runtime instance.

A listener is suppressed when the same file contains a removal with the same event and handler reference. Conditions, source order, receiver and capture do not affect listener pairing. Handler aliases and arguments passed to local helpers are resolved; matching an event alone is insufficient. Inline anonymous callbacks remain outside the listener rule, and `once`/`signal` options count as their own removal mechanisms.

Inline `forEach` callbacks over the same collection or fully known literal event arrays pair by iteration source, independently of callback parameter names. This includes `Object.entries`, `Object.keys`, `Object.values`, destructured items and computed handler references such as `handlers[eventName]`. Different collection bindings, different item projections or handlers, and unknown collection-producing calls remain unpaired. A partial removal list does not suppress an entire registration loop. Class listener facades and callbacks stored under the same event key in a local map/object also participate in pairing. Default-valued callback parameters and computed destructuring keys do not establish an item identity. Matching collection references does not prove that their contents stay unchanged at runtime.

The scan follows scripts belonging to the current document. Iframe and `srcdoc` references are listed as `not-followed / iframe-isolated-global`; their child documents and scripts are not loaded or analyzed. SVG/MathML contents are skipped with a coverage gap, and ordinary HTML after a self-closing foreign element remains eligible for scanning.

Findings of the same rule within one resource and one original source line are grouped into one record. The key includes the original file path; without a mapped position, it uses the generated line. `module-state-write` groups by declaration instead. Polyfill and ordinary findings remain in separate groups so hiding polyfills does not hide other operations.

For groups, `writes` preserves every operation's target, operation, confidence, location, snippet, detail and polyfill metadata. Top-level position fields describe the first operation; group confidence is `medium` if any operation is `medium`. Summaries and plugin diagnostics count grouped records. Playground provides a position selector for all operations in each group.

### Static finding refinements

These refinements run by default over the resources already included in the scan. They remove a finding only when its generated code provides the required evidence; no package-name exclusions or extra configuration are needed.

| Pattern                   | Behavior and boundary                                                                                                                                                                                                                                                                                                 |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unreachable branch        | Stable local constants can prove an `if`, conditional expression or short-circuit arm unreachable. Mutable or unknown values retain the finding.                                                                                                                                                                      |
| CommonJS factory callback | A local or directly imported factory must resolve to a closure calling its callback with fresh object arguments. Those actual arguments can prove a UMD fallback unreachable. Unknown factories, re-exports and replaced/escaped callback parameters remain unresolved.                                               |
| Null listener             | `addEventListener` with a resolved `null` or `undefined` callback has no listener finding. Argument expressions, getters and their other findings remain analyzed.                                                                                                                                                    |
| Storage data key          | A computed assignment to `localStorage`/`sessionStorage` whose string key necessarily contains a dot is data storage, not a native API override. Unknown keys, native method/property replacement, definitions and deletion retain their existing classification. This does not classify persistent data as harmless. |
| Symbol tag fallback       | A string written to a standard prototype's resolved `Symbol.toStringTag` key under a matching missing-property guard is marked `polyfill`. Availability-guarded key aliases also need a truthy guard. Unconditional writes, shadowed symbols and mismatched targets remain visible.                                   |

These are bounded source proofs, not general JavaScript execution. Incomplete parsing, exhausted analysis budgets or direct `eval` prevent refinement; original findings remain. Dependencies and existing coverage gaps are preserved. Grouped records disappear only when all their operations are removed; mixed records retain their other operations.

### ESM module state

`module-state-write` focuses on state that a later React component instance can inherit from a cached ESM module. It reports writes to module-level `let` bindings only when one of these relationships is resolved:

| Pattern                   | Required evidence                                                                                                                                                                                                                                                                 |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Inherited component state | A module value flows into a `useState` or `useReducer` initializer and is written by the same component, its callbacks or local helpers.                                                                                                                                          |
| Skipped component update  | A module value controls an empty early return before a resolved React state setter or reducer dispatch in a component callback or local helper.                                                                                                                                   |
| Skipped model update      | A module value controls an empty early return before `this.method(payload)` in a factory descriptor, and is written later in that action or its local callbacks. The descriptor contains an initial object and a unique matching reducer that returns `{ ...state, ...payload }`. |

```jsx
import { useState } from 'react';

let remembered = false;
export function App() {
  const [ready, setReady] = useState(remembered);
  const confirm = () => {
    remembered = true; // module-state-write: reused by a later mount
    setReady(true);
  };
  return <button onClick={confirm}>{String(ready)}</button>;
}
```

Use `scriptMode: 'module'` for JavaScript/JSX source. The plugin derives the mode from ESM output; HTML uses `<script type="module">`. No additional option is required. Other pollution rules keep their existing scope.

Hook identities are resolved from `react` imports, including named aliases and default/namespace imports. The rule follows stable local aliases, lazy initializers, local custom-hook wrappers, helper arguments and returned values. Reducer initializer functions must resolve; an unknown initializer may discard its input. A scalar snapshot captured during module evaluation does not become a live reference to its source binding.

Model matching uses local object structure and parameter bindings, without requiring particular factory, package or method names. It follows direct calls and local async arrow callbacks. These findings have `medium` confidence: verify that the factory binds the receiver to the reducer and creates a fresh store on remount. A module flag can outlive that store and prevent refilling its data. A singleton initialization flag alone does not qualify; resources retained alongside the flag are not evidence of lost page state.

A value refreshed before the initializer or guard is excluded when the assignment dominates that use. Recognized synchronous effect cleanup that restores the same binding, and complete `try/finally` resets for temporary guards, also exclude the matching pattern. An unrelated reset function or a conditional reset before use does not establish that restoration occurs. Existing initialization-only and paired-cleanup exclusions still apply.

Ordinary shared writes do not produce this finding: caches, counters, telemetry context, retry timestamps and singleton markers need the same positive component-state evidence. An `if (initialized) return` alone is insufficient. This is a precision-oriented scope: absence of a finding does not prove that shared state is safe, that requests finish, or that cleanup executes.

| Boundary          | Behavior                                                                                                                                                                                                                                                                                                          |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Binding scope     | Only module-level `let` assignments/updates participate. `var`, `const`, property mutations, local/shadowed bindings, imported bindings and writes during module initialization are outside this rule.                                                                                                            |
| Dependencies      | Resources or original source paths under `node_modules` are excluded from this rule; other rules still inspect their effects.                                                                                                                                                                                     |
| Source maps       | With `sourceMaps: true` and embedded `sourcesContent`, original JavaScript/TypeScript supplies hook identities and declaration kinds, including `let` emitted as `var`. Without it, only relationships recognizable in generated code are reported.                                                               |
| Unsupported flows | Opaque imported helpers, unresolved custom hooks, state containers without the local merge-reducer pattern, arbitrary instance initialization, object-property flows and unresolved cross-resource value flows are not reported. Dynamic/spread descriptor keys and nonlocal reducers are outside model matching. |
| Analysis limits   | Parse/budget failures in the positive pass produce `module-state-analysis-incomplete`; unconfirmed module-state writes are omitted. Existing scan gaps remain visible.                                                                                                                                            |

Each finding includes `declaration: { name, location }`, generated/original write locations and a `detail` explaining the matched relationship. Multiple writes to one declaration within a resource produce one record with `writes`. Playground can switch between the definition and write sites. Severity remains `warning`; resolved data-flow confidence does not confirm a runtime lifecycle defect.

### Conditional polyfills

API fallbacks installed behind a missing-native check carry `polyfill` metadata. This includes JavaScript builtins, browser APIs and grouped installations such as fetch with Request, Response and Headers. Playground hides identified polyfills by default, independently of browser targets, with an option to show them. Code, raw findings and severity counts are preserved. Unconditional overrides, monitoring wrappers and unrecognized checks remain visible.

| Supported check or installation                                                                     | Behavior                                                                                                                                                                                                                                                                         |
| --------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `!window.fetch`, `typeof fetch === 'undefined'`, `typeof fetch !== 'function'`                      | Identifies the branch that installs the missing or non-callable API.                                                                                                                                                                                                             |
| `!('fetch' in window)`                                                                              | Resolves a known property on a global or native API object, including aliases.                                                                                                                                                                                                   |
| `typeof window !== 'undefined' && !window.setImmediate`                                             | Uses the missing-API condition within a conjunction.                                                                                                                                                                                                                             |
| `if (nativeCapabilitiesAvailable) return;` followed by installation                                 | Carries the opposite condition through the remaining statements in the same block. Compound AND/OR capability checks can guard a fallback bundle.                                                                                                                                |
| `for (...; ... && !window.requestAnimationFrame; ...)` and `while (!nativeApi)`                     | Applies the missing-API condition to the loop body and the for-loop update, excluding the initializer and following statements.                                                                                                                                                  |
| Function and API-reference assignments, synchronous factories, function-valued property descriptors | Recognizes constructor/method installations, vendor implementation aliases and missing getters such as `isIntersecting`. Factories must resolve to a function or return a statically resolved API reference. Synchronous setup can also install companion APIs before returning. |

A disjunction such as `!window.fetch || force` does not prove that a native capability is missing and stays visible. Guards do not cross deferred function boundaries. A loop condition does not establish a guard after the loop; the first `do...while` iteration has no preceding condition. Early returns inside loops, `throw` and `try/finally` do not establish a continuation guard. Unknown property names and custom global APIs remain visible. A capability bundle can replace an existing member when another checked capability is missing; `guardTarget` names a checked API, not a guarantee that every member was absent. Other side effects inside a fallback retain their existing findings.

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

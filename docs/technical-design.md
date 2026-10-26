# Scanner architecture

The SDK accepts explicit file, URL or content sources and returns a single Manifest v2. Rust owns loading, parsing, analysis, dependency scheduling and report construction. The TypeScript wrapper calls the napi binding and parses its JSON result. It exports public types for platform consumers. CSS contents are outside the analysis scope. The Playground consumes the same manifest for local inspection; it does not change scanner output or add a server-side scan path.

## Modules and data flow

```text
TypeScript SDK → napi task → coordinator → worker pool
                                ↑          load → analyze
                                └── results ──────┘
                         graph → report → JSON → SDK
```

| Module                       | Responsibility                                                                 |
| ---------------------------- | ------------------------------------------------------------------------------ |
| `model.rs`                   | Typed input options, intermediate findings and public manifest                 |
| `loader.rs`                  | Content/file/HTTP reads, redirects, access policy, byte bounds and source maps |
| `resolve.rs`                 | URL joining, browser import-map resolution, credential handling                |
| `html/mod.rs`                | Script/style/frame discovery, base URL, import maps and document inline styles |
| `js/values.rs`, `eval.rs`    | Abstract values, binding aliases, finite strings and selected builtin results  |
| `js/visitor.rs`, `rules.rs`  | Scope-aware traversal, effects, cleanup matching and dependencies              |
| `scheduler.rs`               | Graph ownership, resource identity, work dispatch and budgets                  |
| `lines.rs`, `report.rs`      | UTF-16 positions, snippets, source maps, chains, gap grouping and summary      |
| `lib.rs`                     | napi task lifecycle, request normalization and AbortSignal cleanup             |
| `playground/src/manifest.ts` | Browser-side Manifest v2 validation, indexing, grouping and filtering          |
| `playground/src/app.ts`      | Explorer state, navigation, source fetching, startup loading and layout state  |
| `playground/src/views.ts`    | Record, source, dependency and embedded code rendering                         |
| `playground/scripts/dev.mjs` | Loopback static server, rebuild watch and optional startup manifest endpoint   |

The Playground follows a separate browser-local flow and never feeds data back into the scanner:

```text
Manifest file or URL → Web Worker parse/validate → ManifestIndex → explorer
Record source URL → explicit browser fetch → escaped text → embedded viewer
```

## Design principles

| Principle                              | Design rule                                                                                                                                                            |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Apply scope before work                | The scheduler resolves exclusions before it creates a resource record, loads bytes, or expands a subtree. An excluded dependency remains visible as an explicit edge.  |
| Fail visibly on scope errors           | A matching entry and an empty or invalid exclusion pattern fail the scan. The scheduler never returns an empty manifest for a scope mistake.                           |
| Match event and handler identity       | Listener pairing uses only the event and handler reference. DOM cleanup requires the same tracked allocation and execution path.                                       |
| Treat pairing as teardown evidence     | A matching listener removal anywhere in the same file suppresses the finding. Control-flow proof that teardown runs stays outside the claim.                           |
| Keep browser state disposable          | The Playground keeps one manifest in browser memory. It does not assign durable record identities, persist report data, or expose record URLs that cannot be restored. |
| Fetch only on an explicit action       | Source text and startup manifests load through named user or startup inputs. Fetched text is rendered as escaped text and never executed.                              |
| Keep the selected destination readable | Navigation leaves the inspector on a concrete record. Manifest fields are rendered with labels that state their meaning.                                               |

## Scheduling

One coordinator owns the graph; workers exchange owned jobs and results through channels. Each worker loads and analyzes a resource. `concurrency` bounds in-flight jobs; `analysisConcurrency` caps the worker count. These are not separate network and CPU stages. The bounded number of submitted jobs limits channel occupancy.

Resource identity includes kind, URL, script mode and document context. Different documents can resolve the same module URL under different import maps. Each finite dependency candidate receives its own edge. Cycles and duplicate entries reuse nodes. Redirected resources resolve imports against their final URL. Inline nodes retain their parent and source byte offset.

Analyses are not cached solely by content hash: `import.meta.url` and document-relative operations depend on resource context. Hashes remain useful report metadata. HTTP clients are reused for a scan. The resource count limits both graph growth and loading. Total bytes and deadlines are cooperative limits; already running jobs may finish before a stop is observed.

Workers convert recoverable panics to failed results, preventing the coordinator from waiting forever for a missing completion. Stop conditions preserve unresolved edges and failed resource records. Worker-script dependencies currently remain unresolved with an explicit environment gap. Iframe and srcdoc dependencies are not followed because their scripts run against a separate global object; their edges use `iframe-isolated-global` without adding a coverage gap.

### Scope exclusion

`ScanOptions.excludeUrls` carries regular-expression source strings from TypeScript into the Rust `Options`. The coordinator compiles them once into a `RegexSet` after request validation. The pattern form stays serializable across the napi boundary; it does not accept JavaScript `RegExp` objects, flags, lookaround, or backreferences. An empty or invalid pattern returns `invalid-exclude-url-pattern`.

| Matched input                                    | Result                                                                                                                         |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Entry URL from `resourceUrl` or the entry source | The scan fails with `entry-excluded: <url>` before the coordinator creates the graph                                           |
| Resolved recursive dependency URL                | The dependency edge is `not-followed` with reason `excluded`; no source record, finding, gap, load, or child expansion follows |
| Inline dependency                                | No exclusion test runs; the parent resource carries the scan decision                                                          |
| Unresolved dependency specifier                  | The edge remains `unresolved`; exclusion requires a resolved logical URL                                                       |

The coordinator tests the fully resolved logical URL after import-map, document-base, and module resolution. It runs that test before CSS handling, single-resource mode, Worker handling, depth and resource-count budgets, resource identity creation, and `resourceMappings` lookup. An excluded subtree therefore contributes no descendant edges. A pattern can match host, path, query, or filename text because the whole logical URL is the match input.

Exclusion is a scan-scope control, not a loading authorization rule. It does not replace `access`, credential rejection, redirect authorization, or per-hop loader checks. The match uses the URL selected before loading; a redirect target is not matched again. Redirected resources still resolve later imports against the final URL, and those resolved child URLs receive their own exclusion test.

## JavaScript analysis

Oxc parses each resource and builds scope/symbol references. A single traversal tracks abstract values: global paths, finite strings, primitive literals, local object/array shapes, DOM allocation identities and opaque functions. Assignment rules inspect the receiver of a property write, so replacing a local slot does not mutate the global value formerly stored in it.

Ordinary function bodies are visited for potential effects, while their binding changes are restored after analysis. IIFE parameters receive argument values. Dynamic keys and mutable aliases lower confidence. This is a bounded approximation; it does not model arbitrary calls, runtime order or cross-module value propagation.

Style/DOM insertions and listeners start as deferred findings. Cleanup matching then decides whether they enter the final findings array. URLs on created script/link nodes become dependencies when insertion is observed; stylesheet links require `rel=stylesheet`.

### Cleanup pairing

| Deferred effect | Pairing identity            | Suppression rule                                                                                                            |
| --------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Listener        | Event and handler reference | A matching removal anywhere in the file suppresses the finding, regardless of receiver, capture, source order or conditions |
| DOM/style node  | Tracked allocation          | A later removal in the same straight-line block or a returned cleanup function suppresses the finding                       |

Listener keys use Oxc bindings, unresolved global names and canonical member paths. Aliases retain the original reference. Parameters in different functions retain different binding identities; their names alone never establish a match. Calls to local registration and removal helpers substitute argument identities into both keys, including calls before the helper declaration. Static member paths such as `this.handler` remain an approximation: they do not distinguish runtime instances. Reassignments and arbitrary function-return values are not fully modeled.

A removal for a different handler or event does not suppress a registration. Unresolvable callbacks such as a direct `handler.bind(this)` call have no pairing key. Inline anonymous registrations stay outside the listener rule. `once: true` and `signal` options count as removal mechanisms. None of these rules proves that teardown executes.

DOM cleanup retains its execution-path checks. Ordinary conditional branches and loops prevent same-block suppression; null guards match the optional-chaining behavior. Returned function literals can pair with an insertion in their enclosing block. Every listener removal is recorded separately for end-of-file pairing.

## Reporting and performance

Intermediate records use byte spans. Each resource gets a line index with sparse UTF-16 checkpoints; lookups decode at most about 256 bytes instead of rescanning a long minified line. Snippet extraction stops after its display limit. Findings deduplicate by rule, target and span. Gaps group by source, reason and detail while preserving counts.

A BFS computes shortest referrer chains from entries. The manifest uses local source IDs, separate dependency edges and one findings array. Location source maps are optional; map directives must be actual parser-recognized comments. Inline fragment coordinates and parent origins remain separate so consumers can locate either view.

## Playground

The Playground is a local, read-only explorer for one Manifest v2. A Web Worker parses and validates the file; the main thread indexes records by view, source, group, and dependency relationship. It does not run the scanner, modify a manifest, or send an uploaded manifest to a service. Closing, reloading, clearing, or replacing the manifest discards the in-memory state. Only panel widths persist, in `localStorage` under a layout-only key.

### Layout and navigation

On wide screens, the explorer is a five-track grid: browse panel, 6 px splitter, records panel, 6 px splitter, and inspector. The records panel receives the remaining width. Defaults come from responsive CSS variables (`--col-sidebar` and `--col-inspector`), including wider defaults at 1750 px and narrower defaults at 1250 px.

| Control        | Behavior                                                                                                                                                           |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Pointer drag   | Each splitter changes its adjacent outer panel; pointer capture keeps the drag active outside the 6 px track                                                       |
| Arrow keys     | A focused splitter moves by 8 px, or 32 px with Shift; `aria-valuenow` reports the clamped width                                                                   |
| Double-click   | Clears both inline widths and the saved layout, restoring stylesheet defaults                                                                                      |
| Bounds         | The browse panel is at least 160 px, the inspector at least 300 px, and the records panel at least 280 px; saved widths are re-clamped on resize and manifest load |
| Narrow layouts | At 1000 px and below, splitters disappear and the inspector becomes an overlay; at 650 px and below, browse and records stack                                      |

The workspace uses the viewport height below the top bar. Summary metrics stay compact, and the explorer takes the remaining height. Metrics without a destination view render as plain values, not disabled buttons. The brand control scrolls to the top without writing a URL fragment or changing selection.

Navigation follows one state rule: a jump leaves the inspector on a concrete record when the destination list is not empty. `navigate()` selects the first visible row after filtering, without moving focus or opening the narrow-screen overlay. Explicit record selection still manages focus and the overlay. Same-file **Prev** and **Next** controls rescope the list to the target file, compute its page, and then select the target, so navigation works even when the prior filters excluded it. Arrow Up and Arrow Down move through records and cross page boundaries. The pagination label reports both the visible range and `Page x / y`. An active group chip carries its own clear control.

The inspector puts evidence before coordinates: recorded operation, source file and optional code viewer, raw location, file metadata, entry reachability, file relationships, then the raw record. Its header stays sticky while the detail body scrolls. Selecting a record resets detail scroll to the top, or positions an open code panel below the header. Scanner fields are labeled by meaning; for example, an insertion detail such as `<div>` renders under **Inserted element**, not as an unlabeled paragraph that looks like page content. **Copy JSON** remains available; record links and hash-based selection restore do not exist because a one-time in-memory upload has no persistent manifest or record identity to reopen.

### Embedded source viewer

**View source** is an explicit fetch action, separate from **Open in new tab**. The viewer resolves its target as follows:

| Record source                           | Viewer target                                               |
| --------------------------------------- | ----------------------------------------------------------- |
| External HTTP(S) source                 | That source URL and the record location                     |
| Inline fragment with a location         | Parent document URL and the translated parent location      |
| Inline source record without a location | Parent document URL at the inline origin point              |
| Anonymous or non-HTTP(S) source         | No viewer target; the manifest snippet remains the evidence |

The fetch omits credentials and a referrer. The dev server CSP permits `connect-src 'self' http: https:` for this read; the response is converted to text, cached by URL for the loaded manifest, escaped during rendering, and never executed. A declared byte length or decoded text length over the 8 MiB cap, a non-2xx response, a network failure, or a CORS rejection leaves an error state with **Retry**, while the recorded snippet and external link remain available. Opening another file closes the viewer; moving between records in the same display file keeps it open and recenters the highlight. Replacing or clearing the manifest clears cached source text.

The viewer renders a bounded window rather than a full editor document. The initial radius is 25 lines on each side of the recorded range; **Expand context** triples it to at most 5000. A file record without a location starts at line 1. Each rendered line is capped at 2000 characters, centered on the hit, with an ellipsis marking omitted text. The sticky line-number gutter, hit-line background, character `<mark>`, manual scroll centering, wrap toggle, line-range footer, and same-file finding navigation make minified bundles inspectable without building a DOM node per source character.

### Startup manifest loading

Startup loading uses one manifest query parameter, an optional display-name parameter, and two server inputs:

| Input                                  | Transfer path                                                                                                    | Result                                                             |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| `pnpm playground --manifest <path>`    | Dev server serves that one file at `/__manifest`; the printed URL sets `?manifest=%2F__manifest&name=<basename>` | The page imports the local file on open and re-reads it on refresh |
| `pnpm playground --manifest-url <url>` | Server validates and prints the URL; the browser fetches it directly                                             | The page imports the remote file when CORS permits the read        |
| Handwritten `?manifest=<value>`        | Browser resolves the value against the page URL and fetches the resulting HTTP(S) URL                            | Same-origin paths and remote URLs use the same import path         |

A browser cannot read a local disk path, so `file:` values produce an instruction to use `--manifest` rather than a fetch attempt. The query resolver rejects empty and unparsable values, unsupported protocols, and URLs with credentials. The display name comes from the basename of `name` or the URL path, is limited to 200 characters, and falls back to `manifest.json`.

The dev server resolves `--manifest` once at startup and fails before listening when the path is not a file. It exposes no arbitrary-path endpoint: `/__manifest` returns only that selected file, with a fresh read per request and a 64 MiB cap before reading. Static files still resolve only under the build output directory. The server binds to `127.0.0.1`, accepts only GET and HEAD, and sends `no-store`, `nosniff`, and no-referrer headers. `--manifest-url` is validated for HTTP(S) and no credentials but is never proxied or fetched by the server.

An autoloaded response becomes a browser `File` and enters the same size check, worker validation, and indexing path as an upload. The startup parameters stay in the address bar after a successful autoload, so refresh reimports the current bytes. Choosing a file, dropping a file, or clearing the workspace removes `manifest` and `name` with `history.replaceState`, making the manual state authoritative on later refreshes.

## Compatibility and migration

Manifest v2 is a breaking replacement for the previous full/compact formats, query/index helpers, checkpoint/resume, JSONL and timing APIs. The npm SDK exports `scan`, `scanHtml`, `scanJs`, `version`, and `defaults`. `excludeUrls` is an additive optional scan option; it adds the dependency reason `excluded` without changing the Manifest v2 record shape or schema version. The Rollup/Rolldown plugin consumes this same public SDK and owns only bundle mapping, source-location conversion, filtering, diagnostic severity and asset emission. Plugin callers receive the option through `scanOptions`, with the same entry-match failure rule.

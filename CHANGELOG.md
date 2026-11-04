# @bloodhound-rs/core

## 0.3.0

### Minor Changes

- 3c3521b: Support CommonJS require alongside ESM imports through the same synchronous module graph. Declare Node.js 22.12+ to match native require(esm) support, including the scanner's native loader.
- ca08a5a: Report `module-state-write` warnings for functions and callbacks that mutate cached ESM module state. Include direct assignments, updates, destructuring and property writes/deletes, with source locations and lifecycle context. Accept JSX in module sources.
- 3c3521b: Group findings by rule, resource and original source file/line, falling back to generated lines. Preserve declaration grouping for module-state writes and keep polyfills separate from ordinary findings. Retain every operation's position, target, detail and polyfill metadata in writes, and support position selection for all grouped rules in both Playground views.

  Apply allowGlobals to each grouped operation before emitting a diagnostic, retaining the remaining targets.

- 3c3521b: Attach lexical variable declaration locations to module-state findings, including original TypeScript/JavaScript positions and identifier ranges. Add write/declaration navigation to Playground source previews, with shared messages and host component support.
- 3c3521b: Group module-state findings by their declaration within each resource. Preserve all write locations and snippets in an optional writes array and count the grouped records in summaries. Add write-location switching to React and standalone Playground previews, with shared messages and independent preview caching. Keep older manifests compatible.
- d2a0e91: Associate plugin manifest resources with project-relative files and source maps, including companion assets for inline maps. Add optional source-preview metadata types. The Playground can preview original project files or source-map snapshots at mapped locations and link an external source map to scans produced without the plugin.
- 3c3521b: Identify conditional API polyfills in Rust, including browser APIs and grouped fallback installations. Hide all identified polyfills by default in Playground with a show/hide toggle, independent of browser targets. Preserve raw findings, diagnostics and emitted code; retain unconditional overrides and monitoring wrappers.

### Patch Changes

- 471f684: Recognize polyfills guarded by in checks, compound native capability checks, early returns and for/while conditions, including assignments of existing API references, synchronous factory results and missing accessor descriptors. Classify IntersectionObserverEntry and legacy immediate timers as host APIs while keeping forced overrides and unproven guards visible.
- 471f684: Pair DOM insertions with removals of the same tracked node across conditional branches and callbacks, including cleanup defined before mounting. Preserve synchronous removal order, distinguish separate node allocations and retain existing stylesheet cleanup rules.
- bed0c9a: Extract browser-safe manifest parsing, indexing and source previews into a shared package. Publish @bloodhound-rs/playground with independent React manifest selector and viewer components, a ready-to-use ManifestPlayground composition, injectable UI components and authenticated source loading. Export styles and a headless manifest entry for direct consumption. Keep core's existing public report types available through type re-exports.
- 471f684: Pair listener registration and removal across inline forEach callbacks using collection and item identity. Resolve Object entries/keys/values projections, destructured items and computed handler references without matching unrelated collections or callback bindings.
- 3c3521b: Resolve document.defaultView and simple synchronous IIFE return values so global-object aliases are not misclassified as module state. Preserve original global mutation classifications when sourcemaps resolve aliases hidden by bundler wrappers. Exclude function declarations and bindings initialized with function or arrow literals from module-state-write, including when their writes appear before their declarations. Preserve variables assigned callbacks after initialization.
- 3c3521b: Verify mapped module-state writes against the original JavaScript/TypeScript scopes and write positions. Exclude declarations rewritten into lazy module initialization assignments while preserving later mutations of eligible bindings. Keep generated-code analysis when original source content is unavailable.
- 3c3521b: Limit module-state-write findings to module-level let bindings. Exclude var and const bindings, including const-object property mutations. Use the original declaration kind when sourcemap source content is available, preserving let state mutations compiled to var. Leave other pollution rules unchanged.
- 471f684: Exclude string ID sequences, synchronously restored flags, paired cleanup state and successful module namespace caches, opaque generation counters and initialization-only helper call chains from module-state-write findings. Match complete lexical bindings and keep unknown value origins, async resets, cached promises and unrelated writes eligible for the subsequent component-state analysis. Follow resolved named imports and re-exports while retaining public entry exports and namespace exposure.
- 3c3521b: Ignore module-state-write findings from node_modules resources and original sourcemap paths, including maps without embedded source content. Match complete path segments for both slash styles. Preserve other rules for dependencies, including global mutations recovered through sourcemap analysis.
- 471f684: Narrow module-state-write to resolved React state inheritance and persistent guards that skip component/model state updates. Follow hook import identities, local aliases, lazy/reducer initializers and local helper arguments; exclude dominating refreshes, paired effect cleanup and temporary guards with complete finalizers. Recognize receiver calls matching a unique local state/payload merge reducer in a factory descriptor as medium-confidence candidates requiring store-lifetime verification. Ordinary shared writes and unsupported state flows no longer produce this warning. Preserve other rule categories and report positive-analysis budget failures as coverage gaps.
- 471f684: Exclude recognized native scroll, control, media and current event state setter assignments from pollution findings. Apply the same classification to compound updates, Object.assign and Reflect.set while retaining property redefinition, deletion, method replacement, prototype changes and global style mutations.
- 471f684: Exclude statically paired document attributes, classes, inline styles, DOM/style nodes, listeners and captured global/host value restorations. Resolve stable aliases, instance fields, local forwarding helpers, keyed callback storage and literal event lists, with embedded sourcemap evidence and conservative fallback for unknown identities.
- 471f684: Refine findings using stable constant reachability and actual callback arguments from locally resolved or directly imported factories. Exclude null listeners and dotted Storage data-key assignments from their respective pollution rules, while preserving other effects. Mark guarded Symbol.toStringTag string fallbacks as polyfills. Keep unresolved paths and existing coverage gaps visible.
- b824b4c: Publish the core, native platform packages, and Rollup plugin with synchronized release metadata and updated platform package descriptions. No runtime API changes.
- Updated dependencies [bed0c9a]
- Updated dependencies [ca08a5a]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
  - @bloodhound-rs/manifest@0.1.0

## 0.2.0

### Minor Changes

- 55094b9: Replace full/compact reports with Manifest v2, a typed Rust resource graph and unified pollution findings. CSS is discovered without scanning CSS rules. Fix global-alias writes, cleanup matching, finite dependency candidates, redirect/import-map context and UTF-16 location performance.

  This is a breaking pre-1.0 API change: remove report/index/query, checkpoint/resume, JSONL, timing and adapter helpers; use scan/scanHtml/scanJs, version/defaults and dependencyMode instead. Migrate the Rollup/Rolldown plugin to v2 and add per-rule severity configuration.

- 5bca92b: Accept HTML and JavaScript strings as source text and return a compact manifest by default. Use explicit file or URL source objects for loading, and `reportFormat: 'full'` for detailed reports and GlobalIndex. Resolve HTML bases and import maps automatically, and report missing bases for relative references in unnamed text.

  Remove internal hashes and duplicate mutation findings from the default output, group coverage gaps with counts, and retain source locations, dependency edges, global modifications, DOM/CSS findings, and incomplete-scan status. Add JSON stdout examples with caller-owned persistence. Split Rust loading, scheduling, JavaScript analysis, markup parsing, and reporting into focused modules.

### Patch Changes

- 5bca92b: Handle large scan reports in GlobalIndex, report helpers, and checkpoint inputs by converting report collections record by record. Preserve per-record JSON depth and expansion limits, support large query outputs, and buffer report writes before atomically replacing the output file.
- 55094b9: External `sourceMappingURL` references now consult `resourceMappings` before loading, so bundlers can supply in-memory source maps alongside in-memory chunks.
- e814af8: Distribute native binaries as separate optional packages selected by operating system, CPU architecture, and Linux libc. Keep the core package limited to the generated loader, type declarations, and schemas.
- 5bca92b: Add optional scan timing diagnostics for loading, analysis, record construction, dependency resolution, and report generation, with per-resource durations and JavaScript parser, semantic, and traversal breakdowns.
- 55094b9: Keep scripts after self-closing SVG/MathML elements visible to HTML scanning, and exclude iframe/srcdoc child globals from recursive analysis. Detect bare global listener calls and pair removals by event and handler reference, independent of receiver, capture and conditions.

  Use bundler source-map metadata for custom, inline and hidden maps. Include the Rollup plugin in Changesets preparation, package assembly and publishing, including plugin-only releases.

- 5bca92b: Keep entry-path pagination stable across repeated queries and reject malformed release repository owners. Add automated Rust, SDK, and release-script regression coverage to CI.

## 0.1.1

### Patch Changes

- 4c754ee: Release the Rust resource scanning SDK under the MIT license with native binaries for Linux, macOS, and Windows. Fix release artifact extraction on Windows.

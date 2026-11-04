# @bloodhound-rs/manifest

## 0.1.0

### Minor Changes

- bed0c9a: Extract browser-safe manifest parsing, indexing and source previews into a shared package. Publish @bloodhound-rs/playground with independent React manifest selector and viewer components, a ready-to-use ManifestPlayground composition, injectable UI components and authenticated source loading. Export styles and a headless manifest entry for direct consumption. Keep core's existing public report types available through type re-exports.
- ca08a5a: Report `module-state-write` warnings for functions and callbacks that mutate cached ESM module state. Include direct assignments, updates, destructuring and property writes/deletes, with source locations and lifecycle context. Accept JSX in module sources.
- 3c3521b: Group findings by rule, resource and original source file/line, falling back to generated lines. Preserve declaration grouping for module-state writes and keep polyfills separate from ordinary findings. Retain every operation's position, target, detail and polyfill metadata in writes, and support position selection for all grouped rules in both Playground views.

  Apply allowGlobals to each grouped operation before emitting a diagnostic, retaining the remaining targets.

- 471f684: Export `filterManifestBySourcePaths` from the package root and `/utils` to filter trusted source paths using regular expressions. Remove matching findings, grouped writes, resources and dependency references, rebuild remaining graph links and record counts, and preserve the original scan status and cost metrics without mutating the input report.
- 3c3521b: Attach lexical variable declaration locations to module-state findings, including original TypeScript/JavaScript positions and identifier ranges. Add write/declaration navigation to Playground source previews, with shared messages and host component support.
- 3c3521b: Group module-state findings by their declaration within each resource. Preserve all write locations and snippets in an optional writes array and count the grouped records in summaries. Add write-location switching to React and standalone Playground previews, with shared messages and independent preview caching. Keep older manifests compatible.
- 3c3521b: Identify conditional API polyfills in Rust, including browser APIs and grouped fallback installations. Hide all identified polyfills by default in Playground with a show/hide toggle, independent of browser targets. Preserve raw findings, diagnostics and emitted code; retain unconditional overrides and monitoring wrappers.

### Patch Changes

- 471f684: Narrow module-state-write to resolved React state inheritance and persistent guards that skip component/model state updates. Follow hook import identities, local aliases, lazy/reducer initializers and local helper arguments; exclude dominating refreshes, paired effect cleanup and temporary guards with complete finalizers. Recognize receiver calls matching a unique local state/payload merge reducer in a factory descriptor as medium-confidence candidates requiring store-lifetime verification. Ordinary shared writes and unsupported state flows no longer produce this warning. Preserve other rule categories and report positive-analysis budget failures as coverage gaps.

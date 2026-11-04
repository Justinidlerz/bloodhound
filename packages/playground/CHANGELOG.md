# @bloodhound-rs/playground

## 0.1.0

### Minor Changes

- bed0c9a: Extract browser-safe manifest parsing, indexing and source previews into a shared package. Publish @bloodhound-rs/playground with independent React manifest selector and viewer components, a ready-to-use ManifestPlayground composition, injectable UI components and authenticated source loading. Export styles and a headless manifest entry for direct consumption. Keep core's existing public report types available through type re-exports.
- 3c3521b: Group findings by rule, resource and original source file/line, falling back to generated lines. Preserve declaration grouping for module-state writes and keep polyfills separate from ordinary findings. Retain every operation's position, target, detail and polyfill metadata in writes, and support position selection for all grouped rules in both Playground views.

  Apply allowGlobals to each grouped operation before emitting a diagnostic, retaining the remaining targets.

- 3c3521b: Attach lexical variable declaration locations to module-state findings, including original TypeScript/JavaScript positions and identifier ranges. Add write/declaration navigation to Playground source previews, with shared messages and host component support.
- 3c3521b: Group module-state findings by their declaration within each resource. Preserve all write locations and snippets in an optional writes array and count the grouped records in summaries. Add write-location switching to React and standalone Playground previews, with shared messages and independent preview caching. Keep older manifests compatible.
- ca08a5a: Add per-record before/after render slots for custom markers and actions. Accept a messages object across the viewer, selector and composed Playground, with all default English UI copy in an exported catalog.
- 3c3521b: Identify conditional API polyfills in Rust, including browser APIs and grouped fallback installations. Hide all identified polyfills by default in Playground with a show/hide toggle, independent of browser targets. Preserve raw findings, diagnostics and emitted code; retain unconditional overrides and monitoring wrappers.

### Patch Changes

- 471f684: Show code directly after the record summary in the detail panel and open original source by default when a source map is linked. Default module-state records to the declaration position when available. Fall back to generated code or the recorded snippet when no preview resource is available. Move source map association, record JSON, entry links and dependencies below the code while retaining grouped location and declaration navigation.
- Updated dependencies [bed0c9a]
- Updated dependencies [ca08a5a]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
  - @bloodhound-rs/manifest@0.1.0

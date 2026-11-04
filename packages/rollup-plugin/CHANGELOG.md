# @bloodhound-rs/rollup-plugin

## 0.3.0

### Minor Changes

- d2a0e91: Let onManifest take ownership of the scan result. When a callback is configured, await it instead of emitting a manifest asset. Without a callback, continue emitting the configured manifest file by default. Update the examples to retain default file output.
- 3c3521b: Support CommonJS require alongside ESM imports through the same synchronous module graph. Declare Node.js 22.12+ to match native require(esm) support, including the scanner's native loader.
- ca08a5a: Report `module-state-write` warnings for functions and callbacks that mutate cached ESM module state. Include direct assignments, updates, destructuring and property writes/deletes, with source locations and lifecycle context. Accept JSX in module sources.
- d2a0e91: Associate plugin manifest resources with project-relative files and source maps, including companion assets for inline maps. Add optional source-preview metadata types. The Playground can preview original project files or source-map snapshots at mapped locations and link an external source map to scans produced without the plugin.
- 3c3521b: Identify conditional API polyfills in Rust, including browser APIs and grouped fallback installations. Hide all identified polyfills by default in Playground with a show/hide toggle, independent of browser targets. Preserve raw findings, diagnostics and emitted code; retain unconditional overrides and monitoring wrappers.

### Patch Changes

- 3c3521b: Group findings by rule, resource and original source file/line, falling back to generated lines. Preserve declaration grouping for module-state writes and keep polyfills separate from ordinary findings. Retain every operation's position, target, detail and polyfill metadata in writes, and support position selection for all grouped rules in both Playground views.

  Apply allowGlobals to each grouped operation before emitting a diagnostic, retaining the remaining targets.

- b824b4c: Publish the core, native platform packages, and Rollup plugin with synchronized release metadata and updated platform package descriptions. No runtime API changes.
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [471f684]
- Updated dependencies [bed0c9a]
- Updated dependencies [ca08a5a]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [471f684]
- Updated dependencies [d2a0e91]
- Updated dependencies [471f684]
- Updated dependencies [3c3521b]
- Updated dependencies [471f684]
- Updated dependencies [b824b4c]
  - @bloodhound-rs/core@0.3.0

## 0.2.0

### Minor Changes

- 55094b9: Replace full/compact reports with Manifest v2, a typed Rust resource graph and unified pollution findings. CSS is discovered without scanning CSS rules. Fix global-alias writes, cleanup matching, finite dependency candidates, redirect/import-map context and UTF-16 location performance.

  This is a breaking pre-1.0 API change: remove report/index/query, checkpoint/resume, JSONL, timing and adapter helpers; use scan/scanHtml/scanJs, version/defaults and dependencyMode instead. Migrate the Rollup/Rolldown plugin to v2 and add per-rule severity configuration.

- 55094b9: Add `@bloodhound-rs/rollup-plugin`, a Rollup, Rolldown, and Vite plugin that scans emitted bundles during `generateBundle`, reports diagnostics with source-mapped locations, and emits the compact manifest next to the bundle.

### Patch Changes

- 55094b9: Keep scripts after self-closing SVG/MathML elements visible to HTML scanning, and exclude iframe/srcdoc child globals from recursive analysis. Detect bare global listener calls and pair removals by event and handler reference, independent of receiver, capture and conditions.

  Use bundler source-map metadata for custom, inline and hidden maps. Include the Rollup plugin in Changesets preparation, package assembly and publishing, including plugin-only releases.

- Updated dependencies [5bca92b]
- Updated dependencies [55094b9]
- Updated dependencies [55094b9]
- Updated dependencies [e814af8]
- Updated dependencies [5bca92b]
- Updated dependencies [55094b9]
- Updated dependencies [5bca92b]
- Updated dependencies [5bca92b]
  - @bloodhound-rs/core@0.2.0

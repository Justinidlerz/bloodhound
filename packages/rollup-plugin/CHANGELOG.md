# @bloodhound-rs/rollup-plugin

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

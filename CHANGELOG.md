# @bloodhound-rs/core

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

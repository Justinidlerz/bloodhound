---
'@bloodhound-rs/core': minor
'@bloodhound-rs/rollup-plugin': minor
---

Replace full/compact reports with Manifest v2, a typed Rust resource graph and unified pollution findings. CSS is discovered without scanning CSS rules. Fix global-alias writes, cleanup matching, finite dependency candidates, redirect/import-map context and UTF-16 location performance.

This is a breaking pre-1.0 API change: remove report/index/query, checkpoint/resume, JSONL, timing and adapter helpers; use scan/scanHtml/scanJs, version/defaults and dependencyMode instead. Migrate the Rollup/Rolldown plugin to v2 and add per-rule severity configuration.

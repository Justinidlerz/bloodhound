---
'@bloodhound-rs/core': patch
---

Ignore module-state-write findings from node_modules resources and original sourcemap paths, including maps without embedded source content. Match complete path segments for both slash styles. Preserve other rules for dependencies, including global mutations recovered through sourcemap analysis.

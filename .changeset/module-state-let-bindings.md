---
'@bloodhound-rs/core': patch
---

Limit module-state-write findings to module-level let bindings. Exclude var and const bindings, including const-object property mutations. Use the original declaration kind when sourcemap source content is available, preserving let state mutations compiled to var. Leave other pollution rules unchanged.

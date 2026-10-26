---
'@bloodhound-rs/core': patch
---

External `sourceMappingURL` references now consult `resourceMappings` before loading, so bundlers can supply in-memory source maps alongside in-memory chunks.

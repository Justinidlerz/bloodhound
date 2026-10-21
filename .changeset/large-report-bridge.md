---
'@bloodhound-rs/core': patch
---

Handle large scan reports in GlobalIndex, report helpers, and checkpoint inputs by converting report collections record by record. Preserve per-record JSON depth and expansion limits, support large query outputs, and buffer report writes before atomically replacing the output file.

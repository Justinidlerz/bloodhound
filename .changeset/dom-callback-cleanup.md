---
'@bloodhound-rs/core': patch
---

Pair DOM insertions with removals of the same tracked node across conditional branches and callbacks, including cleanup defined before mounting. Preserve synchronous removal order, distinguish separate node allocations and retain existing stylesheet cleanup rules.

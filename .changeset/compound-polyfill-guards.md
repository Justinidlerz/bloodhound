---
'@bloodhound-rs/core': patch
---

Recognize polyfills guarded by in checks, compound native capability checks, early returns and for/while conditions, including assignments of existing API references, synchronous factory results and missing accessor descriptors. Classify IntersectionObserverEntry and legacy immediate timers as host APIs while keeping forced overrides and unproven guards visible.

---
'@bloodhound-rs/core': patch
---

Verify mapped module-state writes against the original JavaScript/TypeScript scopes and write positions. Exclude declarations rewritten into lazy module initialization assignments while preserving later mutations of eligible bindings. Keep generated-code analysis when original source content is unavailable.

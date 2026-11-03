---
'@bloodhound-rs/core': minor
'@bloodhound-rs/manifest': minor
'@bloodhound-rs/playground': minor
'@bloodhound-rs/rollup-plugin': patch
---

Group findings by rule, resource and original source file/line, falling back to generated lines. Preserve declaration grouping for module-state writes and keep polyfills separate from ordinary findings. Retain every operation's position, target, detail and polyfill metadata in writes, and support position selection for all grouped rules in both Playground views.

Apply allowGlobals to each grouped operation before emitting a diagnostic, retaining the remaining targets.

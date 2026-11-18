---
'@bloodhound-rs/core': minor
'@bloodhound-rs/manifest': minor
'@bloodhound-rs/rollup-plugin': minor
'@bloodhound-rs/playground': patch
---

Restore CSS artifact scanning with warning-level css-global-style findings for document roots and unscoped selectors. Follow stylesheet links, inline styles and CSS imports, preserve component selector and scope boundaries, and expose CSS source locations and paired source maps in the playground.

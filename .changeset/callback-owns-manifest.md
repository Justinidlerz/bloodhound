---
'@bloodhound-rs/rollup-plugin': minor
---

Let onManifest take ownership of the scan result. When a callback is configured, await it instead of emitting a manifest asset. Without a callback, continue emitting the configured manifest file by default. Update the examples to retain default file output.

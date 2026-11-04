---
'@bloodhound-rs/core': patch
---

Resolve document.defaultView and simple synchronous IIFE return values so global-object aliases are not misclassified as module state. Preserve original global mutation classifications when sourcemaps resolve aliases hidden by bundler wrappers. Exclude function declarations and bindings initialized with function or arrow literals from module-state-write, including when their writes appear before their declarations. Preserve variables assigned callbacks after initialization.

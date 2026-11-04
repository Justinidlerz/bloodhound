---
'@bloodhound-rs/core': patch
'@bloodhound-rs/manifest': patch
---

Narrow module-state-write to resolved React state inheritance and persistent guards that skip component/model state updates. Follow hook import identities, local aliases, lazy/reducer initializers and local helper arguments; exclude dominating refreshes, paired effect cleanup and temporary guards with complete finalizers. Recognize receiver calls matching a unique local state/payload merge reducer in a factory descriptor as medium-confidence candidates requiring store-lifetime verification. Ordinary shared writes and unsupported state flows no longer produce this warning. Preserve other rule categories and report positive-analysis budget failures as coverage gaps.

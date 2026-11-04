---
'@bloodhound-rs/core': patch
---

Exclude recognized native scroll, control, media and current event state setter assignments from pollution findings. Apply the same classification to compound updates, Object.assign and Reflect.set while retaining property redefinition, deletion, method replacement, prototype changes and global style mutations.

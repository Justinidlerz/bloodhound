---
'@bloodhound-rs/manifest': minor
---

Export `filterManifestBySourcePaths` from the package root and `/utils` to filter trusted source paths using regular expressions. Remove matching findings, grouped writes, resources and dependency references, rebuild remaining graph links and record counts, and preserve the original scan status and cost metrics without mutating the input report.

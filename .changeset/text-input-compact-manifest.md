---
'@bloodhound-rs/core': minor
---

Accept HTML and JavaScript strings as source text and return a compact manifest by default. Use explicit file or URL source objects for loading, and `reportFormat: 'full'` for detailed reports and GlobalIndex. Resolve HTML bases and import maps automatically, and report missing bases for relative references in unnamed text.

Remove internal hashes and duplicate mutation findings from the default output, group coverage gaps with counts, and retain source locations, dependency edges, global modifications, DOM/CSS findings, and incomplete-scan status. Add JSON stdout examples with caller-owned persistence. Split Rust loading, scheduling, JavaScript analysis, markup parsing, and reporting into focused modules.

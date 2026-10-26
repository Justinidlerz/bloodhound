# Manifest v2 verification

The refactored scanner was checked locally on macOS arm64, Node.js 22.23.1 and Rust 1.99.0-nightly with Oxc 0.154.0. Native/TypeScript/plugin builds and public type checks passed. The current suites passed 33 Rust tests, 47 build/release script tests, 11 Node SDK tests and 11 Rollup/Rolldown integration tests. These counts supersede the historical test and coverage measurements below for this implementation. Rust line coverage has not been remeasured.

The current regressions cover local-slot/global-alias distinctions, cleanup identity and execution order, CSS discovery only, UTF-16 positions, distinct import-map contexts, all finite import candidates, URL-dependent analysis, resource budgets, redirects, parse failures, offline source maps, AbortSignal listener cleanup and native concurrent calls. Unsupported Worker environments and selected HTML contexts remain explicit gaps.

Earlier live-page comparisons used a 35,351-byte sanitized and shortened HTML sample. Network resources and reachable graphs differed between runs. The old engine reached its 180-second budget; early v2 runs returned in roughly 6–14 seconds, still with partial coverage. These observations do not establish a controlled speedup, precision or recall. No finding's severity alone proves harmful runtime behavior.

The final sample run with `maxResources: 200`, `timeoutMs: 45000` and `resourceTimeoutMs: 10000` returned in 25.7 seconds: 200 sources, 29,190,193 bytes and 947 candidate findings (110 error-level and 837 warning-level). It remained partial with 275 unmapped module references, 145 resource-budget exclusions, and 164 counted analysis gaps. Seventeen CSS edges were listed without analysis. The single-mode scan took 8 ms, listed 34 dependencies and zero findings, and remained partial because of five foreign-HTML coverage gaps. These live-network counts are observations, not a stable benchmark.

The sections below are historical records for the previous implementation. APIs, test totals, coverage percentages and platform artifacts described there do not validate Manifest v2.

# Historical local verification

This record describes SDK checks performed on macOS arm64 with Node.js 22.23.1 and Rust 1.99.0-nightly. Oxc components are pinned to 0.154.0, with a minimum Rust version of 1.97.

| Check                                                                                        | Result                                                                                                                             |
| -------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `cargo check --locked`                                                                       | Passed                                                                                                                             |
| Rust release and napi native builds                                                          | Passed                                                                                                                             |
| `pnpm typecheck` and TypeScript declaration generation                                       | Passed                                                                                                                             |
| Initial `pnpm pack`                                                                          | Produced `bloodhound-core-0.1.0.tgz`                                                                                               |
| External consumer extraction, `@bloodhound-rs/core` import, native scanning, and index calls | Passed                                                                                                                             |
| External TypeScript consumer type checking                                                   | Passed                                                                                                                             |
| napi-rs generated entry point and Rust scan / index exports                                  | External ESM imports, JS/CSS scans, index queries, and recovery passed                                                             |
| Native Rust cancellation listener                                                            | Cancellation before and during scans passed. Existing onabort handlers were preserved, and listeners were removed after completion |
| sharedConfig design example                                                                  | completed, preserving write / read / write records, conflict candidates, and entry paths                                           |
| Checkpoint save and resume                                                                   | completed, with Rust revalidating inputs                                                                                           |
| HTML + base + import map + inline styles + module summaries                                  | completed, with four resources, three references, and a sharedConfig write                                                         |
| Separate iframe scopes and JSONL readback                                                    | Six resources, five references, and two independent sharedConfig targets. JSONL entity counts matched the standard report          |
| Public css-tree 3.2.1 minified artifact                                                      | partial, with 202655 bytes, 360 accesses, and one candidate finding                                                                |

The public artifact's partial result includes gaps involving mutable captured aliases, switch / exception control flow, and unknown object operations. Results are saved in `outputs/csstree-rust-report.json` and `outputs/csstree-rust.jsonl`. This run is not a precision, recall, or performance benchmark.

TypeSafe reviewed SDK code, public technical facts, and aggregate verification results. Captured HTML and resource bodies were not sent. Local commands produced the compilation, package loading, and scan results.

Automated Rust, SDK, and release-script suites supplement these checks. See [Testing](testing.md) for commands and feature coverage. Evaluation against a fully labeled dataset has not been performed.

# Platform package verification

The following checks cover separate binary packages:

| Check                                               | Result                                                                                                                   |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Six release archives                                | Core is about 17 KB with no binary. Each platform package is about 4.2–4.7 MB and contains one binary                    |
| macOS arm64 consumers                               | npm and pnpm each downloaded only core and darwin-arm64. External imports and native scans passed                        |
| Installation selection for the other four platforms | With target OS, CPU, and libc settings, npm downloaded only core and the matching platform package                       |
| Changesets                                          | All six packages advanced together. Check, beta, and latest preparation rehearsals and frozen lockfile validation passed |
| Release manifest                                    | Five platform packages precede core. Archive SHA-512 checks and dry-run passed                                           |
| Local SDK                                           | Rust build, TypeScript checks, and existing JS / HTML examples passed                                                    |
| Workflow                                            | actionlint passed. The workflow with separate platform packages has not yet run across all CI targets                    |

Installation checks used a temporary local registry and actual package archives. The macOS arm64 binary came from a local build, while the other platforms reused existing CI artifacts. Those platforms were checked for installation selection only, without executing their binaries locally. TypeSafe assisted with checking the packaging approach and the scope of the conclusions. Installation, build, and scan commands produced the actual results.

# Automated test verification

The automated suites passed locally on macOS arm64 with Node.js 22.23.1 and Rust 1.99.0-nightly:

| Suite                                      | Passed | Failed |
| ------------------------------------------ | -----: | -----: |
| Rust modules and scan engine               |     86 |      0 |
| Build and release scripts                  |     48 |      0 |
| Public Node SDK and local HTTP integration |     21 |      0 |
| Total                                      |    155 |      0 |

`pnpm test` also passed native compilation and TypeScript checks. Prettier, rustfmt, `git diff --check`, and actionlint passed. Tests now run in the release workflow, but these test additions have not yet run on remote CI.

Production Rust line coverage measured with cargo-llvm-cov 0.9.1 excludes test files and `test_support.rs`. The Rust-only run covered 88.00%. Combining its profiles with SDK tests against an instrumented debug native library covered 3,649 of 3,926 lines (92.94%):

| Module              | Line coverage |
| ------------------- | ------------: |
| Scan engine         |        91.27% |
| JavaScript analysis |        92.98% |
| Native bindings     |        91.63% |
| HTML and CSS        |        92.64% |
| Data model          |        97.65% |
| Index queries       |        98.03% |
| Resolution          |        97.86% |

The combined measurement uses the generated loader's `NAPI_RS_NATIVE_LIBRARY_PATH` override to load the instrumented binary and merges the Rust and Node process profiles. It does not include JavaScript script coverage, third-party dependencies, or branch coverage. Remaining lines include error handling and less common input forms. These results do not establish complete coverage of every possible input or application-level precision and recall.

Regression tests exercise stable entry-path pagination across repeated queries and rejection of malformed release repository owners. TypeSafe assisted with reviewing coverage scope and reporting limits. Executed tests and LLVM profiles provide the measured results. See [Testing](testing.md) for suite commands and feature groups.

# Direct release workflow

The release script uses `npm publish`, verifies public registry integrity after each package, and publishes core after all platform packages are available. Retries skip identical published versions. Main-branch releases record version metadata and a release tag after publishing succeeds. OIDC authentication does not require the `NPM_TOKEN` secret.

All 48 build and release-script tests passed, including 27 packaging and publishing cases. Prettier, actionlint, and `git diff --check` passed. Publisher tests mock npm commands and registry visibility. They do not publish packages or approve pending staged versions. Local workflow edits require a new workflow run to validate remote publishing.

# Large report bridge

The native bridge converts report collections separately, including reports embedded in checkpoints. Query outputs use the same serializer as scan results. This removes the aggregate JSON node limit from report APIs while preserving depth and expansion checks on individual input records. Report files use buffered writes followed by flush, sync, and atomic replacement.

All 81 Rust tests and 17 SDK tests passed. The SDK regressions cover reports exceeding one million JSON value nodes, report helpers, large query outputs, both checkpoint entry points, and rejection of cyclic, deeply nested, amplified, or malformed records. Type checking, formatting, and diff checks passed.

A previously failing 42 MiB report passed validation, GlobalIndex construction, exact report round trip, index export, adapters, diffs, and runtime-candidate queries using the local build. Its 47,647 global access records produced 22 static conflict candidates. The original report remains partial because of scan budgets. This is local verification, not evidence of a published fix.

A fresh scan of the original HTML completed the scan-to-index path with default budgets: 58 resources, 50,589 global access records, and 24 static conflict candidates. The approximately 44 MiB report was saved and indexed successfully. The scan returned after 120.852 seconds and retained `partial` status with explicit time and analysis-budget gaps.

# Text input and compact manifests

| Check                                           | Result                                                                                                 |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Raw HTML input without an attachment URL        | Inline code and absolute dependencies scanned; no attachment path used as a document base              |
| Relative dependencies without a base            | Reported as `missing-base-url`                                                                         |
| Automatic HTML base and import map discovery    | Passed with absolute bases and supplied document URLs                                                  |
| JSON stdout examples                            | One JSON document, no implicit file writes                                                             |
| Report comparison using identical serialization | 38,311,812 bytes full; 635,396 bytes compact, a 98.34% reduction                                       |
| Preserved results                               | 225 global modifications, 2,487 DOM/CSS findings, 389 dependency edges                                 |
| Coverage gaps                                   | 15,184 records grouped into 346 entries with counts and example locations                              |
| Completion status                               | `partial`; the 120-second scan budget was reached                                                      |
| TypeScript API                                  | Default manifests, explicit full reports, dynamic format options, and full-report-only indexes checked |

The comparison uses full and compact output from the same scan. It measures removed fields and records independently of whitespace. All modifications and DOM/CSS findings retain locations. Read-only access details remain available through the explicit full-report option. Rust module splitting preserves existing analysis regression coverage. These changes are local and have not been published.

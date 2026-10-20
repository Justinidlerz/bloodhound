# Local verification

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

TypeSafe checked public technical facts about JavaScript, native extensions, and HTML tokenization. Internal designs, repository source, and scan reports were not uploaded. Local commands produced the compilation, package loading, and scan results.

The initial SDK checks did not add or modify unit tests or use a browser. Evaluation against a fully labeled dataset has not been performed.

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

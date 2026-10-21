# Test the SDK

This guide is for contributors running the Rust, Node.js SDK, and release-script tests. Use Node.js 22+, pnpm 10.20.0, Rust 1.97+, and the platform build tools listed in the README.

## Run tests

From the repository root, install dependencies and run the full suite:

```sh
pnpm install --frozen-lockfile
pnpm test
```

The command runs Rust and script tests, builds the native extension, checks TypeScript declarations, and runs SDK tests against the compiled extension. A failed assertion exits with a nonzero status.

Use these commands for individual suites:

| Command                   | Scope                                                                 | Prerequisite                |
| ------------------------- | --------------------------------------------------------------------- | --------------------------- |
| `pnpm test:rust`          | Rust module and scanner tests                                         | Rust dependencies installed |
| `pnpm test:scripts`       | Versioning, platform metadata, schemas, packing, and publishing logic | Node dependencies installed |
| `pnpm test:sdk`           | Public napi exports and real HTTP behavior                            | `pnpm build`                |
| `pnpm format:check`       | JavaScript, TypeScript, JSON, YAML, and documentation formatting      | Node dependencies installed |
| `cargo fmt --all --check` | Rust formatting                                                       | rustfmt installed           |

SDK network tests bind ephemeral ports on `127.0.0.1`. Permit local listening when running in a sandbox. They do not require an external HTTP service or browser.

## Test coverage

The suites cover these feature groups:

| Area                | Test files                                                                 | Cases                                                                                                                                                                                                                                                    |
| ------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Data model          | `native/src/tests/model.rs`                                                | Hash stability, source identity, UTF-16 locations, serialization, default budgets                                                                                                                                                                        |
| Resolution          | `native/src/tests/resolve.rs`                                              | URL bases, bare specifiers, import-map scopes and prefixes, blocked mappings, map merges                                                                                                                                                                 |
| HTML and CSS        | `native/src/tests/markup.rs`                                               | Base URLs, import-map timing, script modes, templates, iframes, inline locations, CSS imports, assets, global styles, malformed input                                                                                                                    |
| JavaScript analysis | `native/src/tests/js.rs`                                                   | Scope and alias tracking, shadowing, global operations, finite evaluation, symbols, destructuring, branches and loops, module summaries, workers, DOM, webpack, Vite, analysis limits                                                                    |
| Scan engine         | `native/src/tests/engine.rs`                                               | Recursive graphs and cycles, context isolation, loading, access restrictions, budgets, cancellation, source maps, JSONL replay, checkpoint recovery, report validation                                                                                   |
| Compact output      | `native/src/tests/compact.rs`, `tests/sdk.test.mjs`, `tests/types.test.ts` | Mutation and finding preservation, grouped gap counts, scopes, text input, URL bases, automatic import maps, JSON stdout, full-report selection, and TypeScript inference                                                                                |
| Index queries       | `native/src/tests/query.rs`                                                | Target and source filters, stable pagination, bounded traversal, conflicts, unbound candidates, report adapters, runtime candidates, version diffs                                                                                                       |
| Native bindings     | `native/src/tests/bindings.rs`, `tests/sdk.test.mjs`                       | Every public export, input adaptation, errors, default values, real native loading, signal lifecycle, report files                                                                                                                                       |
| HTTP                | `tests/sdk.test.mjs`                                                       | Redirect authorization, credential isolation, retries, size limits, timeouts, proxy routing, in-flight cancellation                                                                                                                                      |
| Build and release   | `tests/scripts.test.mjs`, `tests/release.test.mjs`                         | Schema generation, cleanup, version synchronization, Changesets check/beta/latest modes, platform metadata, six-package archives, integrity checks, publishing order and visibility polling, partial failures, public-version retries, registry failures |

Release tests use temporary workspaces. Packing tests create real archives containing fixture bytes, not executable native libraries. Publisher tests replace every npm command with a mock and simulate registry visibility delays, timeouts, and integrity mismatches. They do not publish packages, use npm tokens, or alter repository versions. Native compilation and generated bindings are exercised by the full suite's build and SDK stages.

## Measure Rust coverage

Install `cargo-llvm-cov` and the toolchain's `llvm-tools-preview` component, then run:

```sh
cargo llvm-cov --lib --locked \
  --ignore-filename-regex '(tests/|test_support.rs)' \
  --html
```

The HTML report is written to `target/llvm-cov/html`. This measurement excludes test source and includes production Rust code. A Rust-only run does not count Node SDK execution, including napi callbacks and HTTP integration cases. Line coverage does not establish branch coverage or precision and recall on real applications.

## CI

The preparation job runs script tests before version preparation. Each of the five native build jobs runs Rust tests, builds the extension, checks declarations, runs SDK tests, and runs the JavaScript and HTML examples. Package assembly and publishing depend on these jobs succeeding. Local results cover the current machine; the other platforms require their CI jobs to run.

# Large report regression coverage

SDK tests construct a valid report exceeding one million JSON value nodes and exercise validation, indexing, report getters, source queries, conflict queries, adapters, diffs, runtime candidates, and both checkpoint entry points. Separate cases reject cyclic records, excessive nesting, shared-object expansion, and malformed collections. Fixtures are generated locally and contain no captured page data.

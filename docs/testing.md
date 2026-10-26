# Testing

Run `pnpm test` after installing with `pnpm install --frozen-lockfile`. It runs Rust unit/integration tests, build and release script tests, builds the native SDK and bundler plugin, checks TypeScript declarations, then runs SDK and plugin integration tests.

| Command                      | Coverage                                                                                                                                                                                  |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm test:rust`             | Global/local alias distinction, cleanup identity/order, scope, DOM/script/style detection, HTML extraction, import maps, loading, scheduling, UTF-16 locations, reports and JSON boundary |
| `pnpm test:scripts`          | Native version/platform metadata, clean builds, release preparation, package contents and mocked registry publishing                                                                      |
| `pnpm build`                 | Native release build, generated loader/declarations, SDK and plugin TypeScript                                                                                                            |
| `pnpm typecheck`             | SDK usage contracts and structural compatibility with Rollup/Rolldown plugin types                                                                                                        |
| `pnpm test:sdk`              | Public source forms, offline mappings, recursive/single behavior, errors, source maps, HTTP redirects, AbortSignal lifecycle and concurrent scans                                         |
| `pnpm test:plugin`           | Real in-memory Rollup/Rolldown builds, hidden maps, HTML inline positions, filtering, manifests and build failures                                                                        |
| `pnpm format:check`          | JavaScript, TypeScript, documentation and metadata formatting                                                                                                                             |
| `cargo fmt --all -- --check` | Rust formatting                                                                                                                                                                           |

Rust and Node network tests use loopback HTTP servers. Recursive test graphs use in-memory mappings and disable network access. They cover duplicate entries, cycles, finite dynamic-import candidates, separate document import maps, URL-specific import.meta resolution, redirects, resource/depth limits and parse failures. CSS tests assert discovery without detailed rules and distinguish document inline styles from ordinary SSR markup.

Plugin tests execute both bundlers and compare source-map locations, emitted manifests and diagnostic behavior. Release tests mock npm publishing; no test publishes packages. Current tests are regressions for specified behavior, not a measurement of application-level precision or recall.

Manual examples: `pnpm example`, `pnpm example:html`, `pnpm example:rollup`, and `pnpm example:rolldown`. The first two emit JSON to stdout; bundler examples write into `examples/rollup/dist/`. A live-page scan is useful for performance investigation but is not a reproducible benchmark unless dependency bytes, network state and budgets are fixed.

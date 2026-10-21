# Supported capabilities

Use this reference to identify supported syntax, analysis limits, and report semantics.

| Area                 | Implementation                                                                                                            | Coverage gaps                                                                                     |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| JavaScript input     | Files, URLs, source text, and classic / module / unknown modes                                                            | Syntax errors and missing script mode or environment                                              |
| Bindings and targets | Oxc scopes, local shadowing, separate global lexical bindings and object properties, and unbound references               | Dynamic scopes introduced by `with`                                                               |
| Global operations    | Declarations, reads, assignments, compound assignments, updates, delete, Object / Reflect APIs, and prototypes            | Unknown property keys remain separate accesses without a shared placeholder target                |
| Aliases              | Sequential assignments, invalidation, bounded branch merging, direct IIFEs, and wrapper parameters with known bindings    | Complex destructuring, exception and switch value propagation, and general function side effects  |
| Bounded evaluation   | Literals, concatenation, templates, bounded conditions, constant array and object keys, and restricted return expressions | Getters, proxies, general calls, and expansion beyond budgets                                     |
| ESM                  | Imports, exports, dynamic import candidates, cross-module primitive summaries, and bounded iteration                      | Unbounded dynamic dependencies and unknown bare-specifier mappings                                |
| webpack              | Runtime object properties, pure chunk mapping functions, script `src` expressions, and bounded ID tables                  | Automatic publicPath, complex runtime functions, and unknown addresses                            |
| Vite                 | Dynamic imports, preload wrappers with link creation and callback invocation, and bounded dependency arrays               | General caching mapDeps wrappers and unbounded arrays                                             |
| DOM                  | Candidate operations on document / body / head and aliases, and recoverable node resource addresses                       | Unknown node identity, runtime insertion, and teardown residue                                    |
| Workers / iframes    | Separate scopes for resolvable entry points, with reference edges preserved                                               | Unknown document bases and dynamic child-environment addresses                                    |
| HTML                 | HTML5 tokenization, raw text, base URLs, import map snapshots, scripts, styles, iframes, and inline offsets               | Foreign-content tree contexts, noscript environments, token recovery, and event-handler scopes    |
| CSS                  | CSS Syntax parsing, @import, url, global selectors, resets, custom properties, and animations                             | Parser recovery and unknown resource values. Candidates do not prove actual matching or pollution |
| Source maps          | Rust sourcemap support for external maps and data URLs                                                                    | Missing or unavailable maps affect only optional original-source locations                        |

HTML processing uses tokenization with source locations, without constructing a complete browser DOM tree. Template contents remain inert, and special tree contexts explicitly reduce coverage. For entity-decoded content such as `srcdoc`, the host location identifies the tag containing the attribute. Child-resource locations remain independent.

The webpack adapter matches syntax structures and Oxc bindings rather than fixed minified variable names. Chunk mappings produce bounded candidates without proving that every chunk loads. ESM analysis handles ordinary Vite dynamic imports, while preload analysis adds resource edges.

# Inputs and output formats

String inputs are JavaScript or HTML source text. File and URL loading use explicit source objects. A content URL is optional; relative dependencies without a document URL or usable base produce `missing-base-url`.

Compact manifests are the default. They preserve all global modifications and DOM/CSS findings, dependency edges, source locations, and scan status. Read-only accesses are counted in `summary.omittedReadAccesses`; coverage gaps are grouped with counts and an explicit example location. Full reports (`reportFormat: 'full'`) preserve every access and detailed gap location for indexing, diffs, and runtime attribution candidates.

# Status

| Status    | Meaning                                                                  |
| --------- | ------------------------------------------------------------------------ |
| completed | No required coverage gaps were recorded within the declared scope        |
| partial   | Results exist, with gaps in dependencies, context, semantics, or budgets |
| failed    | No valid analysis result was established                                 |
| cancelled | A `runStatus` value, independent of the scan statuses                    |

`single` excludes the dependency closure and marks reference edges as `not-followed`. `completed` does not mean that the program has no dynamic behavior or that candidate issues occurred at runtime.

# Indexes and locations

- Target IDs use scope, namespace, path segments, and key type. Local Symbols also include the analysis instance and do not merge across resources.
- Identical contents with different URLs or contexts retain separate instances. Cycles prevent repeated analysis while preserving reference edges.
- Unbound bare references retain candidate relationships. They are not automatically classified as implicit global properties.
- Artifact locations use UTF-16 offsets, one-based lines, and zero-based columns. Rust converts its internal UTF-8 offsets when emitting results.
- Multiple writes, prototype changes, and DOM/CSS operations remain `static-candidate` findings.
- Version diffs match resources, scopes, modes, environments, targets, and operation features. Ambiguous matches become additions and removals. Changes between arbitrary values of the same type are not guaranteed to be detected.

# Runtime budgets

Limits apply to resource counts, total bytes, bytes per resource, download buffers, concurrency, depth, time, AST visits, bounded evaluation, candidate counts, summary rounds, and record counts. ASTs are released after each resource analysis, and caches have byte limits.

These limits bound input, work, and result sizes. They are not operating-system RSS limits. A host process can impose additional memory limits for large or untrusted inputs. Request timeouts bound network waits, while parsing does not guarantee immediate preemption.

The Node.js bridge converts report collections record by record, including reports embedded in checkpoints. Per-record JSON depth and expansion limits remain active; large reports and query results do not share one aggregate conversion limit.

JSONL supports consumer backpressure through the filesystem. Standard reports and in-memory indexes use memory proportional to their record counts. Recovery rebuilds the graph from saved entries and reads resources again, including remote content validation.

Checkpoints preserve access restrictions, resource mappings, and source map settings. Supply authentication headers and proxy settings again when resuming.

# Delivery scope

The SDK excludes platform databases, release gates, browser probes, and production deployment. Automated Rust, SDK, and release-script tests are described in [Testing](testing.md). Scans of real artifacts and build evidence do not replace precision and recall evaluation against a fully labeled application dataset.

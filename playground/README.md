# Manifest Playground

A local, read-only viewer for Bloodhound Manifest v2. Open a JSON file to browse every finding, source, dependency and coverage gap.

```sh
pnpm install --frozen-lockfile
pnpm playground
# http://127.0.0.1:4173
```

The viewer uses the repository's existing TypeScript and Rolldown dependencies. It does not require a native SDK build. Use `pnpm playground --port 4174` to choose a different port. Source changes rebuild automatically; reload the page after a rebuild.

## Preload a manifest

```sh
# Local file: served read-only by the dev server and imported on open
pnpm playground --manifest /tmp/test-html.filtered-v3.manifest.json

# Preview files from the scanned project (defaults to the server's working directory)
pnpm playground --project /path/to/project --manifest /path/to/project/dist/bloodhound.manifest.json

# Remote file: fetched by the page on open (the host must allow CORS)
pnpm playground --manifest-url https://example.com/reports/manifest.json
```

Both forms print a startup URL carrying a `?manifest=` parameter. The same parameter can be added by hand to any playground URL: an `http(s)` value is fetched by the page, and a same-origin path is fetched from the serving host. A browser cannot read a local disk path directly, so a `file:` value is rejected with a pointer to `--manifest`. The local file is read fresh per request, so regenerating it and refreshing the page reloads it. Opening or clearing a manifest manually removes the parameter from the address bar.

## Explore a manifest

1. Drop a `manifest.json` file onto the page or select `Open manifest`.
2. Choose `Findings`, `Files`, `Dependencies` or `Coverage`.
3. Group by `Type`, `File`, `Folder` or `Entry`. File groups can include reachable dependencies. Entry groups follow all recorded dependency paths, including shared files and cycles.
4. Search by target, source ID, URL, rule, snippet or dependency specifier. Filter findings by severity and confidence.
5. Select a record to inspect its location, code excerpt, owning file, shortest entry chain and directly related files. Click a graph node to continue browsing that file.

Lists are paginated; every record remains accessible. The file graph initially shows five neighbors in each direction, with an option to expand all neighbors. Unresolved and not-followed references remain available in the Dependencies view.

Drag the dividers between the browse, records, and detail panels to resize them. A focused divider also responds to the arrow keys (`Shift` for larger steps); double-click resets both widths. Widths are remembered in this browser as a layout preference only.

In a record's Source file section, `View source` loads the scanned file into an embedded read-only viewer and highlights the recorded location. Plugin manifests use their project-relative output paths when the local server is available; other records use their HTTP(S) URL or a `file:` URL inside the selected project. Use Prev/Next to move between findings in the same file, expand the surrounding context, or wrap long (minified) lines. Inline records open their parent document at the translated position.

### Original source

Plugin manifests contain `sourceMap` associations. Select a finding, dependency or coverage record, then choose **Preview original source**. The viewer resolves the recorded generated position through the associated map, displays the source path and highlights the mapped line. Original mappings identify a point, not an expression range.

| Available data                                         | Preview                                                                        |
| ------------------------------------------------------ | ------------------------------------------------------------------------------ |
| Source file within `--project`                         | Current project contents, labeled separately from the scanned build            |
| Embedded `sourcesContent`                              | Source-map build snapshot when the local file is unavailable                   |
| Resolvable HTTP(S) source without embedded content     | Source fetched on demand, subject to CORS                                      |
| No source-map association                              | Original-source panel stays hidden                                             |
| Map has no mapping or readable source at that position | Preview explains the missing information; the recorded excerpt stays available |

For a manifest produced without the plugin, expand **Link source map** on its file card. Enter an HTTP(S) map URL or a map path relative to `--project`, then choose **Link**. All records in that scanned file share the association. **Remove link** hides the original-source panel. Manual associations remain in the current session and do not modify the imported JSON. Indexed maps with embedded sections and `sourceRoot` are supported.

Project-relative paths are resolved against the directory passed to `--project`, so the project can be moved without changing a plugin manifest. Positions still describe the scanned build; editing the current source file can move them. Serving a remote manifest preserves its URL as the base for relative map references. A manually uploaded manifest with relative references requires the local project directory or an explicit map URL.

`Copy JSON` copies one original record. There is no record link: a manifest is a one-time upload kept only in memory for the current browser session, so a copied URL could not restore it. Reloading or clearing the view removes the uploaded data. `Ctrl+K` / `⌘K` focuses record search, Arrow Up/Down moves through records, and `Escape` closes the narrow-screen detail panel.

## Interpretation

| Display          | Meaning                                                                               |
| ---------------- | ------------------------------------------------------------------------------------- |
| Severity         | The scanner's rule label, not proof of runtime execution or harm                      |
| Confidence       | Confidence in the tracked target, not confidence that cleanup is missing              |
| Listener cleanup | Not reported by the current manifest; the viewer does not infer a missing teardown    |
| Entry membership | Reachability from recorded entry sources, not inferred business application ownership |
| Folder           | URL directory, with inline scripts grouped under their parent document                |
| Coverage         | Grouped analysis limits; occurrence count and group count are distinct                |
| Reported time    | `summary.elapsedMs`, which measures scheduler wall time                               |

Source positions use 1-based lines and 0-based UTF-16 columns. Inline records also show their translated parent-document position. Original source-map positions appear when present. Source maps and source files are read only when a preview is opened. When a remote host blocks cross-origin reads (CORS), the viewer reports that and the recorded excerpt remains available.

## Local data handling

The JSON file is read in a browser worker and is never submitted to a service. With `--manifest`, the loopback dev server returns the named manifest. Its source-preview endpoint reads files only within `--project`, resolves symlinks before checking that boundary, and rejects cross-origin requests. An uploaded manifest is never sent anywhere. All imported values are displayed as text, and the viewer never executes snippets or fetched source code. Remote reads omit credentials and referrers. Source text is capped at 8 MiB and maps at 64 MiB. Files up to 64 MiB are accepted as manifests; unsupported schema versions, invalid records and broken source references produce an error without replacing the current manifest.

```sh
pnpm typecheck:playground
pnpm build:playground
```

The production assets are written to `playground/dist/`. Serve that directory from an HTTP server; opening `index.html` directly with `file:` does not support the module worker. The development server binds to loopback only and serves generated playground assets.

## React integration

`/react.html` renders the `ManifestPlayground` export from `@bloodhound-rs/playground`, which composes the independent `ManifestSelector` and `ManifestViewer` components. The development server prints its URL, including any `--manifest` preload. Host applications can mount `ManifestViewer` alone and pass their existing UI components and resource loader. See [component integration](../packages/playground/README.md) for the API and an Ant Design example.

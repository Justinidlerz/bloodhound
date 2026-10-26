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

In a record's Source file section, `View source` loads the file into an embedded read-only viewer and highlights the recorded location. Use Prev/Next to move between findings in the same file, expand the surrounding context, or wrap long (minified) lines. Inline records open their parent document at the translated position.

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

Source positions use 1-based lines and 0-based UTF-16 columns. Inline records also show their translated parent-document position. Original source-map positions appear when present. The manifest contains excerpts rather than full source files, so the embedded viewer fetches the file text from its HTTP(S) URL on request. When the serving host blocks cross-origin reads (CORS), the viewer reports that and the recorded excerpt remains the fallback; `Open in new tab` still follows the URL.

## Local data handling

The JSON file is read in a browser worker and is never submitted to a service. With `--manifest`, the loopback dev server only returns the one file named at startup; an uploaded manifest is never sent anywhere. All imported values are displayed as text, and the viewer never executes snippets or fetched source code. A remote source file is fetched only after `View source` is selected, is capped at 8 MiB, and is sent without credentials or a referrer. That request does reveal the source URL to its host. Files up to 64 MiB are accepted; unsupported schema versions, invalid records and broken source references produce an error without replacing the current manifest.

```sh
pnpm typecheck:playground
pnpm build:playground
```

The production assets are written to `playground/dist/`. Serve that directory from an HTTP server; opening `index.html` directly with `file:` does not support the module worker. The development server binds to loopback only and serves generated playground assets.

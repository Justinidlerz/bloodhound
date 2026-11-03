# @bloodhound-rs/playground

React 18/19 components for Manifest v2. The viewer and resource selector are independent exports. Neither depends on the scanner's native binary or an application router.

Module-state records with `declaration` metadata show both the write and variable definition positions. Select either position to preview generated code or the original source, using the same UI adapters, text loader and message overrides.

Grouped records with a `writes` array appear once, with a count and a position button for each operation. Module-state records use their declaration name; other rules group operations on the same original source line and display the first target. Each position shows its own target, snippet and detail, with generated-code and original-source previews. Module-state records also link to their shared declaration. Older manifests without `writes` retain their single-position display.

| Export               | Responsibility                                                                             |
| -------------------- | ------------------------------------------------------------------------------------------ |
| `ManifestViewer`     | Render an already loaded manifest, filter records, inspect dependencies and preview source |
| `ManifestSelector`   | Read a local JSON file or manifest URL and call `onChange`                                 |
| `ManifestPlayground` | Ready-to-use selector and viewer composition                                               |
| `ManifestComponents` | UI component slots, including an optional replacement for the source viewer                |

## Install and use

```sh
pnpm add @bloodhound-rs/playground
```

React 18 or 19 is supplied by the host application. JavaScript, TypeScript declarations and CSS are included in the package.

```tsx
import { ManifestPlayground } from '@bloodhound-rs/playground';
import '@bloodhound-rs/playground/styles.css';

<ManifestPlayground components={components} loadText={loadText} />;
```

`initialResource` preloads a parsed report when mounting. `onChange(resource)` receives resource selections. `selectorProps` forwards `loadManifest`, `parse` and `maxBytes` to the selector. The remaining props go to `ManifestViewer`. Hosts that already manage the selected build use `ManifestViewer` directly.

Headless parsing is available without importing React components:

```ts
import { parseManifest, ManifestIndex } from '@bloodhound-rs/playground/manifest';

const { manifest } = parseManifest(jsonText);
const index = new ManifestIndex(manifest);
```

## Ant Design example

Install `antd` in the host application and pass its components with adapters for input events and table data. The host loads the manifest; this example renders the report without a resource selector.

```tsx
import { Button, Input, Select, Table, Tag } from 'antd';
import { ManifestViewer, type Manifest, type ManifestComponents } from '@bloodhound-rs/playground';
import '@bloodhound-rs/playground/styles.css';

const components: Partial<ManifestComponents> = {
  Button: (props) => <Button {...props} />,
  Select,
  Tag,
  Input: ({ onChange, ...props }) => (
    <Input {...props} onChange={(event) => onChange(event.target.value)} />
  ),
  Table: ({ data, ...props }) => <Table {...props} dataSource={data} />,
};

export function ScanReport({ manifest }: { manifest: Manifest }) {
  return <ManifestViewer manifest={manifest} components={components} />;
}
```

Ant Design's `Input.onChange` receives an event, while the viewer expects a string. Its `Table` uses `dataSource`, while the viewer supplies `data`. The adapters above translate these props. Define the component map outside rendering to keep component identities stable.

Bloodhound does not depend on a UI library or inject a theme provider. Its CSS only styles the viewer's layout, default table and code excerpt, under `bh-` classes. Existing providers and component themes remain owned by the host.

Button adapters receive `variant` (`action`, `toggle`, `link`, or `text`), `aria-pressed` for selection, and `disabled` only when an action is unavailable. Tags receive the report's machine-readable `status`; tables receive `selectedRowId`. These optional props let hosts apply their own button hierarchy, status colors and selected-row styles without inspecting labels or DOM position.

Components with different props use a small adapter. `Input.onChange` and `Select.onChange` receive a string. `Table` receives `data`, `columns`, `rowKey="id"` and `pagination={false}`. Missing slots use native HTML controls.

## Record markers and actions

`renderRecordBefore` and `renderRecordAfter` add the first and last table columns. Both receive `(row, context)` and return any React node, including tags, icons, checkboxes and buttons. Return `null` for records that need no extra content. Without these props, no extra columns are added.

```tsx
import { ManifestViewer, type RecordRenderer } from '@bloodhound-rs/playground';

const actionMessages = {
  selected: 'Selected',
  needsReview: 'Needs review',
  markReviewed: 'Mark reviewed',
};
const renderMarker: RecordRenderer = (row, { selected, components: { Tag } }) =>
  row.view === 'findings' ? (
    <Tag>{selected ? actionMessages.selected : actionMessages.needsReview}</Tag>
  ) : null;

const renderActions: RecordRenderer = (row, { openDetails, components: { Button }, messages }) => (
  <>
    <Button onClick={() => reviewService.markReviewed(row)}>{actionMessages.markReviewed}</Button>
    <Button onClick={openDetails}>{messages['layout.details']}</Button>
  </>
);

<ManifestViewer
  manifest={manifest}
  components={components}
  renderRecordBefore={renderMarker}
  renderRecordAfter={renderActions}
/>;
```

`reviewService` belongs to the host. Render callbacks run only for visible rows and must remain pure; put mutations in event handlers. Hosts own action state and persistence. Clicking the extra content does not bubble into table-row selection; call `openDetails()` explicitly when needed. The context includes `selected`, the resolved `components`, and the resolved `messages` dictionary. Use a build/report identifier together with `row.id` when persisting annotations, since row IDs belong to one report.

## Messages

All default English UI copy is defined in `src/locales/en.ts` and exported as `englishMessages`. `ManifestViewer`, `ManifestSelector` and `ManifestPlayground` default their `messages` parameter to this object. Supply a plain key/value object to override messages; omitted keys use the English catalog. No translation function or global locale setup is required.

```tsx
import {
  ManifestViewer,
  englishMessages,
  type PlaygroundMessages,
} from '@bloodhound-rs/playground';

const messages: Partial<PlaygroundMessages> = {
  'viewer.title': '扫描报告',
  'record.before': '标记',
  'record.after': '操作',
  'pagination.previous': '上一页',
  'pagination.next': '下一页',
  'pagination.summary': '第 {page} / {pages} 页，共 {count} 条记录',
  'value.warning': '警告',
  'value.module-state-write': '模块状态写入',
};

<ManifestViewer manifest={manifest} messages={messages} />;
// Copy englishMessages to create a complete translation catalog.
```

Values are strings. Keep the `{name}` placeholders used by a message; interpolation is handled by the component. Keys cover headings, filters, categories, pagination, empty/loading states, file selection, source preview, splitter accessibility labels, and standard status/type labels. Changing `messages` on rerender updates labels without resetting the selected record, filters or loaded source. Viewer instances keep their dictionaries separate; the composed Playground forwards the same dictionary to its selector and viewer.

Labels do not change rule IDs, filter values, paths, snippets, record JSON or host callbacks. Manifest diagnostics and loader/server errors remain data; `viewer.notice`, `selector.error` and `source.error` provide message wrappers. Custom action text and custom UI components remain owned by the host.

## Polyfill visibility

Records identified by the scanner as polyfills are hidden from the findings list by default, regardless of API type or browser targets. The banner shows the hidden count and a toggle restores the original records. Category counts, search, grouping and pagination follow the toggle; the manifest's original findings and summary remain intact.

The `polyfill` metadata records the missing-API check that guards installation, including related API bundles such as fetch, Request, Response and Headers. Unconditional overrides, monitoring wrappers and older manifests without polyfill metadata remain visible. Customize this copy through the `polyfill.*` message keys. Headless consumers can set `Filters.includePolyfills: true` when calling `ManifestIndex.baseRows`.

## Source loading

`manifestUrl` resolves relative map URLs. `loadText` routes generated code, source maps and original files through the host's resource service. The callback returns text and its absolute resource URL after redirects. No fetched code is executed.

```tsx
import { ManifestViewer, type TextLoader } from '@bloodhound-rs/playground';

// Defined outside rendering; bind the selected build's ID in the host's service layer.
const loadText: TextLoader = async (url, request) => {
  return resourceService.readText({
    url,
    file: request.file,
    kind: request.kind,
    maxBytes: request.limit,
  });
};

<ManifestViewer
  manifest={manifest}
  manifestUrl={manifestUrl}
  components={components}
  loadText={loadText}
/>;
```

`resourceService` above is the host's implementation. `request.file` carries the project-relative path recorded by the plugin, when available. `request.kind` is `source` or `sourcemap`. A copied build artifact can therefore be resolved by build ID and path even when the public URL has changed. The custom loader owns authentication and enforces `request.limit`.

With no loader, public HTTP(S) resources are fetched without credentials. The local Playground passes `workspace={{ projectRoot, fileEndpoint }}` to read files through its project endpoint. Original source is loaded only after opening a preview: project/host file first, then `sourcesContent`, then a resolvable source URL. A missing map association hides the original-source preview; users can explicitly link a map URL or, with a workspace/custom loader, a project path. Links last for the current report session.

To use the platform's Monaco wrapper, supply `components.SourceCode`. Its props are `{ text, path?, location? }`; `location` uses 1-based lines and 0-based UTF-16 columns. Convert columns to Monaco's 1-based convention. The slot controls readonly rendering, highlighting and scrolling.

## Compose the selector and viewer

```tsx
import { useState } from 'react';
import { ManifestSelector, ManifestViewer, type ManifestResource } from '@bloodhound-rs/playground';
import '@bloodhound-rs/playground/styles.css';

export function Playground() {
  const [resource, setResource] = useState<ManifestResource>();
  return (
    <>
      <ManifestSelector onChange={setResource} />
      {resource ? (
        <ManifestViewer
          manifest={resource.manifest}
          manifestUrl={resource.url}
          notices={resource.notices}
        />
      ) : null}
    </>
  );
}
```

`ManifestSelector.loadManifest(url)` replaces URL fetching. `parse(text)` accepts a synchronous or asynchronous parser, including a host-managed worker. `maxBytes` defaults to 64 MiB. Local files remain in the browser; URL resources follow the browser's CORS rules unless the host supplies a loader.

`ManifestViewer` additionally accepts `pageSize` (default 50), `className`, and `onSelect(row)`. Its four record categories are findings, sources, dependencies and gaps. Keep `manifest`, `workspace` and `loadText` references stable while displaying the same build. Replacing any resource context resets selection, filters, map links and pending preview state. Independent viewer instances do not share state.

Groups, records and details occupy independently scrolling columns. Drag either divider to resize it, or focus a divider and use the arrow keys (Shift for larger steps), Home or End. Double-click a divider to reset both widths. Widths are scoped to the viewer and clamped when its container resizes. Set `--bh-layout-height` on the viewer to override the default `65vh` height (minimum `360px`). In containers at most `800px` wide, selecting a record opens details over the list; **Close details** returns to the list.

## Local development

```sh
pnpm build:playground-package
pnpm playground --manifest /path/to/bloodhound.manifest.json --project /path/to/project
```

Open the printed **React components** URL (`/react.html`). It composes these exact exports. The classic Playground uses the same `@bloodhound-rs/manifest` parser and source resolver.

## Changesets and release artifacts

This public workspace package participates in the repository's existing Changesets release flow. Changesets versions `@bloodhound-rs/playground` and its shared manifest dependency. The release workflow builds workspace packages, packs their `dist` output and publishes dependencies before consumers. PR releases use the workflow's beta suffix and tag; main-branch releases use the stable version and latest tag.

`pnpm build:playground-package` builds the component package. `pnpm build:playground` produces the repository's standalone preview site. The npm package exports components, styles and parsing utilities; it does not start a server or mount itself into the host page.

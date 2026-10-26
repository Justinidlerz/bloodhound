import {
  type Filters,
  type ManifestDependency,
  type ManifestFinding,
  type ManifestGap,
  type ManifestIndex,
  type ManifestSource,
  type RecordRow,
  sourceName,
} from './manifest.js';
import { badge, bytes, count, empty, externalLink, html, human, type Markup } from './format.js';
import {
  codeWindow,
  viewerTarget,
  type SourceText,
  type Viewer,
  type ViewerTarget,
} from './source-view.js';

export function sourceButton(index: ManifestIndex, id: string, compact = false): Markup {
  const source = index.sources.get(id);
  if (!source) return html`<span class="muted">${id}</span>`;
  return html`<button
    class="source-link"
    data-action="source"
    data-source="${id}"
    title="${source.url || sourceName(source)}"
  >
    <span class="source-id">${id}</span>${compact ? '' : sourceName(source)}
  </button>`;
}

export function recordList(
  index: ManifestIndex,
  rows: RecordRow[],
  selected: string | null,
  offset: number,
  pageSize: number,
): Markup {
  if (!rows.length)
    return empty('No matching records', 'Try another group or clear the search and filters.');
  return html`${rows.slice(offset, offset + pageSize).map((row) => {
    const source = row.source ? index.sources.get(row.source) : undefined;
    const location = 'location' in row.data ? row.data.location : undefined;
    let preview: string;
    if (row.view === 'findings') preview = (row.data as ManifestFinding).snippet;
    else if (row.view === 'sources')
      preview =
        (row.data as ManifestSource).url ||
        (source?.inline ? `Inline in ${source.inline.parent}` : 'Content supplied without a URL');
    else if (row.view === 'dependencies')
      preview =
        (row.data as ManifestDependency).reason ||
        (row.data as ManifestDependency).url ||
        'Resolved dependency';
    else
      preview =
        (row.data as ManifestGap).detail ||
        `${count((row.data as ManifestGap).count)} occurrence(s) · coverage limit`;
    return html`<button
      class="record ${selected === row.id ? 'selected' : ''}"
      data-action="record"
      data-record="${row.id}"
      aria-pressed="${selected === row.id}"
    >
      <span class="record-top"
        ><span class="eyebrow">${human(row.type)}</span>${badge(row.status)}</span
      >
      <span class="record-title">${row.title}</span>
      <span class="record-preview ${row.view === 'findings' ? 'code-text' : ''}"
        >${preview || 'No excerpt provided'}</span
      >
      <span class="record-meta"
        ><span
          >${row.source || 'scan-wide'}${source && row.view !== 'sources'
            ? ` · ${sourceName(source)}`
            : ''}</span
        ><span
          >${location
            ? `L${location.line}:${location.column}`
            : row.view === 'sources'
              ? `${count(index.findingCounts.get(row.source!) ?? 0)} findings`
              : ''}</span
        ></span
      >
    </button>`;
  })}`;
}

export function groupList(
  index: ManifestIndex,
  rows: RecordRow[],
  filters: Filters,
  query: string,
): Markup {
  const groups = index.groups(rows, filters.grouping);
  const term = query.toLowerCase().trim();
  const visible = groups.filter((group) =>
    `${group.id} ${group.name} ${group.description}`.toLowerCase().includes(term),
  );
  return html`<button
      class="group-row ${filters.group === null ? 'active' : ''}"
      data-action="group-all"
      aria-pressed="${filters.group === null}"
    >
      <span>All ${filters.view === 'gaps' ? 'coverage gaps' : filters.view}</span
      ><span class="group-count">${count(rows.length)}</span>
    </button>
    ${visible.map(
      (group) =>
        html`<button
          class="group-row ${group.id === filters.group ? 'active' : ''}"
          data-action="group"
          data-group="${group.id}"
          aria-pressed="${group.id === filters.group}"
          title="${group.description || group.name}"
        >
          <span class="group-label"
            >${filters.grouping === 'type' ? human(group.name) : group.name}${group.description
              ? html`<small>${group.id} · ${group.description}</small>`
              : ''}</span
          ><span class="group-count">${count(group.count)}</span>
        </button>`,
    )}
    ${visible.length ? '' : html`<p class="aside-hint">No groups match this search.</p>`}`;
}

function field(label: string, value: unknown): Markup {
  return html`<div class="field">
    <dt>${label}</dt>
    <dd>${value}</dd>
  </div>`;
}

function chain(index: ManifestIndex, source: ManifestSource): Markup {
  if (!source.chain.length) return html`<p class="muted">No entry chain was recorded.</p>`;
  return html`<div class="chain">
    ${source.chain.map(
      (id, position) =>
        html`${position
          ? html`<span class="chain-arrow" aria-hidden="true">→</span>`
          : ''}${sourceButton(index, id)}`,
    )}
  </div>`;
}

/** Previous/next finding in the same file, ordered by source position. */
function findingNav(index: ManifestIndex, row: RecordRow): Markup {
  if (row.view !== 'findings' || !row.source) return html``;
  const rows = index.rows.findings
    .filter((candidate) => candidate.source === row.source)
    .sort((a, b) => {
      const x = (a.data as ManifestFinding).location;
      const y = (b.data as ManifestFinding).location;
      return x.line - y.line || x.column - y.column || a.index - b.index;
    });
  if (rows.length < 2) return html``;
  const at = rows.findIndex((candidate) => candidate.id === row.id);
  const prev = rows[at - 1];
  const next = rows[at + 1];
  return html`<span class="code-nav"
    ><button
      class="text-link"
      data-action="record-jump"
      data-record="${prev?.id ?? ''}"
      ${prev ? html`` : html`disabled`}
    >
      ← Prev</button
    ><span>${count(at + 1)} / ${count(rows.length)} in file</span
    ><button
      class="text-link"
      data-action="record-jump"
      data-record="${next?.id ?? ''}"
      ${next ? html`` : html`disabled`}
    >
      Next →
    </button></span
  >`;
}

function codeViewer(
  index: ManifestIndex,
  row: RecordRow,
  target: ViewerTarget,
  viewer: Viewer,
  state: SourceText | undefined,
): Markup {
  const location = target.location;
  const heading = html`<div class="code-head">
    <span class="code-file" title="${target.url}">${sourceName(target.source)}</span
    ><span class="code-loc"
      >${location ? `L${location.line}:${location.column}` : 'File start'}</span
    >${findingNav(index, row)}
  </div>`;
  if (!state || state.status === 'loading')
    return html`<div class="code-panel">
      ${heading}
      <p class="code-state">Loading source…</p>
    </div>`;
  if (state.status === 'error')
    return html`<div class="code-panel">
      ${heading}
      <p class="code-state">
        Source could not be loaded in the browser${state.error ? `: ${state.error}` : ''}. The
        server may block cross-origin reads (CORS); the snippet above is the recorded excerpt.
        <button class="text-link" data-action="source-retry">Retry</button>
      </p>
    </div>`;
  const view = codeWindow(state.text ?? '', location, viewer.radius);
  const truncated = view.from > 1 || view.to < view.total;
  return html`<div class="code-panel">
    ${heading}
    <div class="code-view ${viewer.wrap ? 'wrap' : ''}" tabindex="0" aria-label="Source code">
      <div class="code-lines">
        ${view.lines.map(
          (line) =>
            html`<div class="code-line ${line.inRange ? 'hit' : ''}" data-line="${line.number}">
              <span class="line-no" aria-hidden="true">${line.number}</span
              ><code
                >${line.cutBefore ? '…' : ''}${line.before}${line.hit
                  ? html`<mark>${line.hit}</mark>`
                  : ''}${line.after}${line.cutAfter ? '…' : ''}</code
              >
            </div>`,
        )}
      </div>
    </div>
    <div class="code-foot">
      <span>Lines ${count(view.from)}–${count(view.to)} of ${count(view.total)}</span>${truncated
        ? html`<button class="text-link" data-action="source-expand">Expand context</button>`
        : ''}<button class="text-link" data-action="source-wrap" aria-pressed="${viewer.wrap}">
        ${viewer.wrap ? 'No wrap' : 'Wrap lines'}
      </button>
    </div>
  </div>`;
}

function fileCard(
  index: ManifestIndex,
  source: ManifestSource,
  row: RecordRow,
  viewer: Viewer | undefined,
  texts: Map<string, SourceText>,
): Markup {
  const target = viewerTarget(index, row);
  const open = Boolean(target && viewer && viewer.sourceId === target.source.id);
  return html`<section class="detail-section">
    <h3>Source file</h3>
    <div class="file-card">
      <div>${sourceButton(index, source.id)}</div>
      <p class="url">
        ${source.url ||
        (source.inline
          ? `Inline ${source.kind} in ${source.inline.parent}`
          : 'Anonymous content · no logical URL supplied')}
      </p>
      <div class="action-row">
        ${target
          ? html`<button
              class="text-link"
              data-action="${open ? 'source-close' : 'source-open'}"
              aria-expanded="${open}"
            >
              ${open ? 'Hide source' : 'View source'}
            </button>`
          : ''}${externalLink(target?.url ?? source.url)}<button
          class="text-link"
          data-action="file-findings"
          data-source="${source.id}"
        >
          View ${count(index.findingCounts.get(source.id) ?? 0)} findings →
        </button>
      </div>
      ${open && target && viewer
        ? codeViewer(index, row, target, viewer, texts.get(target.url))
        : ''}
    </div>
    <h4>Shortest entry chain</h4>
    ${chain(index, source)}
  </section>`;
}

function graphNode(index: ManifestIndex, id: string, current = false): Markup {
  const source = index.sources.get(id)!;
  return html`<button
    class="graph-node ${current ? 'current' : ''}"
    data-action="source"
    data-source="${id}"
    title="${source.url || sourceName(source)}"
  >
    <span class="eyebrow">${source.id} · ${source.kind}</span><strong>${sourceName(source)}</strong
    ><small>${count(index.findingCounts.get(id) ?? 0)} findings</small>
  </button>`;
}

export function dependencyGraph(
  index: ManifestIndex,
  source: ManifestSource,
  expanded: boolean,
): Markup {
  const parents = [...(index.parents.get(source.id) ?? [])];
  const children = [...(index.children.get(source.id) ?? [])];
  const limit = expanded ? Infinity : 5;
  const unresolved = (index.outgoing.get(source.id) ?? []).filter(
    (d) => d.status === 'unresolved',
  ).length;
  const omitted = (index.outgoing.get(source.id) ?? []).filter(
    (d) => d.status === 'not-followed',
  ).length;
  return html`<section class="detail-section">
    <div class="section-heading">
      <h3>File relationships</h3>
      <button class="text-link" data-action="file-dependencies" data-source="${source.id}">
        All references →
      </button>
    </div>
    <div class="relation-map" aria-label="Files that import this source and files it imports">
      <div class="graph-column">
        <h4>Imported by <span>${count(parents.length)}</span></h4>
        ${parents.slice(0, limit).map((id) => graphNode(index, id))}${parents.length
          ? ''
          : html`<p class="graph-empty">No referrers</p>`}
      </div>
      <div class="graph-center">
        <span class="graph-arrow" aria-hidden="true">→</span>${graphNode(
          index,
          source.id,
          true,
        )}<span class="graph-arrow" aria-hidden="true">→</span>
      </div>
      <div class="graph-column">
        <h4>Imports <span>${count(children.length)}</span></h4>
        ${children.slice(0, limit).map((id) => graphNode(index, id))}${children.length
          ? ''
          : html`<p class="graph-empty">No linked files</p>`}
      </div>
    </div>
    ${!expanded && (parents.length > limit || children.length > limit)
      ? html`<button class="text-link graph-expand" data-action="expand-graph">
          Show all connected files (${count(parents.length + children.length)})
        </button>`
      : ''}
    ${unresolved || omitted
      ? html`<p class="aside-hint">
          ${count(unresolved)} unresolved and ${count(omitted)} not-followed references are listed
          in Dependencies.
        </p>`
      : ''}
  </section>`;
}

export function inspector(
  index: ManifestIndex,
  row: RecordRow | undefined,
  expanded: boolean,
  viewer: Viewer | undefined,
  texts: Map<string, SourceText>,
): Markup {
  if (!row)
    return empty(
      'Inspect a record',
      'Select a record to see its source, location and dependency relationships.',
    );
  const source = row.source ? index.sources.get(row.source) : undefined;
  const location = 'location' in row.data ? row.data.location : undefined;
  const parent = source && location ? index.parentLocation(source.id, location) : undefined;
  const entryOwners = row.source
    ? [...index.entryReach].filter(([, reach]) => reach.has(row.source!)).map(([id]) => id)
    : [];
  return html`<div class="inspector-header">
      <div>
        <span class="eyebrow">${row.view === 'sources' ? 'File' : 'Record'} ${row.index + 1}</span>
        <h2>${row.title}</h2>
      </div>
      <button
        class="icon-button close-detail"
        data-action="close-detail"
        aria-label="Close details"
      >
        ×
      </button>
    </div>
    <div class="detail-body">
      <div class="badge-row">
        ${badge(row.status)}<span class="pill">${human(row.type)}</span>${row.confidence
          ? html`<span
              class="pill"
              title="Confidence describes the tracked target, not whether cleanup exists."
              >${human(row.confidence)} confidence</span
            >`
          : ''}
      </div>
      <div class="action-row">
        <button class="text-link" data-action="copy-record" data-record="${row.id}">
          Copy JSON
        </button>
      </div>
      ${row.view === 'findings' ? findingDetails(row.data as ManifestFinding) : ''}
      ${row.view === 'dependencies' ? dependencyDetails(index, row.data as ManifestDependency) : ''}
      ${row.view === 'gaps' ? gapDetails(row.data as ManifestGap) : ''}
      ${source
        ? fileCard(index, source, row, viewer, texts)
        : html`<section class="detail-section">
            <h3>Scope</h3>
            <p>This gap applies to the scan as a whole.</p>
          </section>`}
      ${location
        ? html`<section class="detail-section">
            <h3>Location</h3>
            <dl class="fields">
              ${field('Start', `Line ${location.line} · column ${location.column}`)}${field(
                'End',
                `Line ${location.endLine} · column ${location.endColumn}`,
              )}
            </dl>
            <p class="aside-hint">Lines start at 1. Columns start at 0 (UTF-16).</p>
            ${parent
              ? html`<div class="inset">
                  <span class="eyebrow">In parent document</span>
                  <p>
                    ${sourceButton(index, parent.source.id)} ·
                    L${parent.location.line}:${parent.location.column}
                  </p>
                </div>`
              : ''}
            ${location.original
              ? html`<div class="inset">
                  <span class="eyebrow">Original source map position</span>
                  <p class="url">${location.original.source}</p>
                  <p>
                    L${location.original.line}:${location.original.column}${location.original.name
                      ? ` · ${location.original.name}`
                      : ''}
                  </p>
                </div>`
              : ''}
          </section>`
        : ''}
      ${row.view === 'sources' && source
        ? html`<section class="detail-section">
            <h3>File metadata</h3>
            <dl class="fields">
              ${field('Kind', source.kind)}${field('Script mode', source.scriptMode || '—')}${field(
                'Size',
                bytes(source.bytes),
              )}${field('Status', badge(source.status))}
            </dl>
            ${source.inline
              ? html`<p class="aside-hint">
                  Inline origin: ${source.inline.parent} ·
                  L${source.inline.line}:${source.inline.column}
                </p>`
              : ''}
            <details class="raw-details">
              <summary>Content hash</summary>
              <p class="hash">${source.contentHash || 'Not available'}</p>
            </details>
          </section>`
        : ''}
      ${source
        ? html`<section class="detail-section">
              <h3>Reachable from</h3>
              <div class="chip-wrap">
                ${entryOwners.length
                  ? entryOwners.map((id) => sourceButton(index, id))
                  : html`<span class="muted">Not linked to an entry</span>`}
              </div>
              <p class="aside-hint">
                Entry membership follows recorded dependencies. Shared files can belong to multiple
                entries. The manifest does not name business applications.
              </p>
            </section>
            ${dependencyGraph(index, source, expanded)}`
        : ''}
      <details class="raw-details">
        <summary>Raw record</summary>
        <pre>${JSON.stringify(row.data, null, 2)}</pre>
      </details>
    </div>`;
}

function findingDetails(finding: ManifestFinding): Markup {
  // The scanner's detail for an insertion is the created element (`<div>`,
  // `<script> https://…`); other rules record a value type or method name.
  const detailLabel =
    (finding.rule === 'dom-insertion' || finding.rule === 'style-injection') &&
    finding.detail?.startsWith('<')
      ? 'Inserted element'
      : 'Detail';
  return html`<section class="detail-section">
    <h3>Recorded operation <span class="pill">${finding.operation}</span></h3>
    <pre class="snippet"><code>${finding.snippet || 'No code excerpt provided.'}</code></pre>
    ${finding.detail
      ? html`<p class="detail-line">
          <span class="detail-label">${detailLabel}</span><code>${finding.detail}</code>
        </p>`
      : ''}
    ${finding.rule === 'global-listener'
      ? html`<div class="notice-inline">
          <strong>Cleanup is not reported</strong>
          <p>
            This manifest records registration. It does not include a lifecycle or matching removal
            result, so this record alone does not establish a missing teardown.
          </p>
        </div>`
      : ''}
    <p class="aside-hint">
      This is a static code location. Severity does not establish runtime execution or harm.
    </p>
  </section>`;
}

function dependencyDetails(index: ManifestIndex, dependency: ManifestDependency): Markup {
  return html`<section class="detail-section">
    <h3>Dependency edge</h3>
    <div class="edge-path">
      ${sourceButton(index, dependency.from)}<span aria-hidden="true">→</span>${dependency.to
        ? sourceButton(index, dependency.to)
        : html`<span class="muted">No analyzed source</span>`}
    </div>
    <dl class="fields">
      ${field('Kind', dependency.kind)}${field('Status', badge(dependency.status))}
    </dl>
    ${dependency.specifier
      ? html`<h4>Specifier</h4>
          <p class="url">${dependency.specifier}</p>`
      : ''}${dependency.url
      ? html`<h4>Resolved URL</h4>
          <p class="url">${dependency.url}</p>
          ${externalLink(dependency.url)}`
      : ''}${dependency.reason
      ? html`<div class="notice-inline"><strong>${dependency.reason}</strong></div>`
      : ''}
  </section>`;
}

function gapDetails(gap: ManifestGap): Markup {
  return html`<section class="detail-section">
    <h3>Coverage limit</h3>
    <p><strong>${count(gap.count)}</strong> occurrence(s) grouped under this reason.</p>
    ${gap.detail ? html`<p class="url">${gap.detail}</p>` : ''}
    <p class="aside-hint">
      A gap marks content the scanner could not fully analyze. It is separate from a finding.
    </p>
  </section>`;
}

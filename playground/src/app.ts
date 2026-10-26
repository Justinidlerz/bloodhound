import {
  ManifestIndex,
  type Filters,
  type Grouping,
  type Manifest,
  type RecordRow,
  type View,
} from './manifest.js';
import { badge, bytes, count, html, human, time } from './format.js';
import { groupList, inspector, recordList } from './views.js';
import { DEFAULT_RADIUS, viewerTarget, type SourceText, type Viewer } from './source-view.js';
import { resolveManifestParam } from './autoload.js';

function element<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing UI element: ${id}`);
  return node as T;
}
const initialFilters = (): Filters => ({
  view: 'findings',
  query: '',
  severity: '',
  confidence: '',
  grouping: 'type',
  group: null,
  includeDependencies: false,
});
const maxBytes = 64 * 1024 * 1024;
let index: ManifestIndex | undefined;
let filters = initialFilters();
let selected: string | null = null;
let groupQuery = '';
let page = 0;
let pageSize = 50;
let graphExpanded = false;
let viewer: Viewer | undefined;
const sourceTexts = new Map<string, SourceText>();
let codeScrollKey = '';
let visibleRows: RecordRow[] = [];
let worker: Worker | undefined;
let toastTimer: ReturnType<typeof setTimeout>;
let searchTimer: ReturnType<typeof setTimeout>;

function message(text: string) {
  const box = element('message');
  box.textContent = text;
  box.hidden = !text;
}
function toast(text: string) {
  clearTimeout(toastTimer);
  element('toast').textContent = text;
  element('toast').hidden = false;
  toastTimer = setTimeout(() => {
    element('toast').hidden = true;
  }, 4000);
}
/** Drops the startup parameters once the user takes over, so a refresh does not revert. */
function dropManifestParams() {
  const url = new URL(window.location.href);
  if (!url.searchParams.has('manifest')) return;
  url.searchParams.delete('manifest');
  url.searchParams.delete('name');
  history.replaceState(null, '', url);
}

/**
 * Loads the manifest named by `?manifest=` at startup. Remote HTTP(S) URLs
 * are fetched directly (CORS permitting); local files arrive through the
 * development server's `/__manifest` endpoint.
 */
async function autoloadManifest() {
  const params = new URL(window.location.href).searchParams;
  const value = params.get('manifest');
  if (!value) return;
  const target = resolveManifestParam(value, window.location.href, params.get('name'));
  if (target.kind === 'file') {
    message(
      'A browser cannot read a local file path. Start the server with `pnpm playground --manifest <path>` to preload a file from disk.',
    );
    return;
  }
  if (target.kind === 'invalid') {
    message(`Cannot load the manifest parameter: ${target.reason}.`);
    return;
  }
  element('loading').hidden = false;
  try {
    const response = await fetch(target.url, {
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const blob = await response.blob();
    element('loading').hidden = true;
    await openFile(new File([blob], target.name, { type: 'application/json' }));
  } catch (error) {
    element('loading').hidden = true;
    const detail = error instanceof Error ? error.message : 'network error';
    message(
      `Could not load a manifest from ${target.url} (${detail}). Remote hosts must allow cross-origin reads (CORS). For a file on disk, start the server with \`pnpm playground --manifest <path>\`.`,
    );
  }
}

/** Keeps an open viewer only while the selection stays in the same file. */
function syncViewer(row: RecordRow | undefined) {
  if (!viewer || !index || !row) {
    viewer = undefined;
    return;
  }
  const target = viewerTarget(index, row);
  if (!target || target.source.id !== viewer.sourceId) viewer = undefined;
  else viewer.url = target.url;
}

/** Centers the highlighted range inside the code viewer after a render. */
function scrollCode() {
  if (!viewer) return;
  const key = `${selected}|${viewer.url}|${viewer.radius}|${viewer.wrap}`;
  if (key === codeScrollKey) return;
  const view = element('inspector').querySelector<HTMLElement>('.code-view');
  if (!view) return;
  codeScrollKey = key;
  const hit = view.querySelector<HTMLElement>('.code-line.hit');
  if (!hit) {
    view.scrollTop = 0;
    view.scrollLeft = 0;
    return;
  }
  view.scrollTop = Math.max(0, hit.offsetTop - view.clientHeight / 2 + hit.clientHeight / 2);
  const mark = hit.querySelector<HTMLElement>('mark');
  if (mark && !viewer.wrap) view.scrollLeft = Math.max(0, mark.offsetLeft - view.clientWidth / 3);
}

const MAX_SOURCE_BYTES = 8 * 1024 * 1024;

/** Fetches a source file as text on explicit user request. Never executes it. */
async function fetchSource(url: string) {
  if (sourceTexts.has(url)) return;
  sourceTexts.set(url, { status: 'loading' });
  render();
  try {
    const response = await fetch(url, { credentials: 'omit', referrerPolicy: 'no-referrer' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const declared = Number(response.headers.get('content-length') ?? 0);
    if (declared > MAX_SOURCE_BYTES) throw new Error('file larger than 8 MiB');
    const text = await response.text();
    if (text.length > MAX_SOURCE_BYTES) throw new Error('file larger than 8 MiB');
    sourceTexts.set(url, { status: 'ready', text });
  } catch (error) {
    sourceTexts.set(url, {
      status: 'error',
      error: error instanceof Error ? error.message : 'network error',
    });
  }
  render();
}
function syncFilters() {
  element<HTMLInputElement>('search').value = filters.query;
  element<HTMLInputElement>('group-search').value = groupQuery;
  element<HTMLInputElement>('include-dependencies').checked = filters.includeDependencies;
  const statuses = index
    ? [
        ...new Set([
          ...index.rows[filters.view].map((row) => row.status),
          ...(filters.severity ? [filters.severity] : []),
        ]),
      ]
    : [];
  element('status-label').textContent = filters.view === 'findings' ? 'Severity' : 'Status';
  const status = element<HTMLSelectElement>('status-filter');
  status.innerHTML = String(
    html`<option value="">All ${filters.view === 'findings' ? 'severities' : 'statuses'}</option>
      ${statuses.map((value) => html`<option value="${value}">${human(value)}</option>`)}`,
  );
  status.value = filters.severity;
  element('confidence-control').hidden = filters.view !== 'findings';
  element<HTMLSelectElement>('confidence-filter').value = filters.confidence;
  for (const button of document.querySelectorAll<HTMLButtonElement>('[data-action="view"]')) {
    button.setAttribute('aria-pressed', String(button.dataset.view === filters.view));
  }
  for (const button of document.querySelectorAll<HTMLButtonElement>('[data-action="grouping"]')) {
    button.setAttribute('aria-pressed', String(button.dataset.mode === filters.grouping));
  }
}

function render(updateDetails = true) {
  if (!index) return;
  const focused = document.activeElement as HTMLElement | null;
  const groupFocus = focused?.dataset.action?.startsWith('group')
    ? { action: focused.dataset.action, group: focused.dataset.group }
    : undefined;
  const base = index.baseRows(filters);
  visibleRows = index.filter(base, filters);
  const pages = Math.max(1, Math.ceil(visibleRows.length / pageSize));
  page = Math.max(0, Math.min(page, pages - 1));
  if (selected && !visibleRows.some((row) => row.id === selected)) {
    selected = null;
    graphExpanded = false;
    viewer = undefined;
    document.body.classList.remove('detail-open');
    updateDetails = true;
  }
  element('group-list').innerHTML = String(groupList(index, base, filters, groupQuery));
  element('group-total').textContent = count(index.groups(base, filters.grouping).length);
  element('record-list').innerHTML = String(
    recordList(index, visibleRows, selected, page * pageSize, pageSize),
  );
  element('result-count').textContent =
    `${count(visibleRows.length)} ${filters.view === 'gaps' ? 'groups' : 'records'}`;
  element('page-label').textContent = visibleRows.length
    ? `${count(page * pageSize + 1)}–${count(Math.min((page + 1) * pageSize, visibleRows.length))} of ${count(visibleRows.length)} · Page ${count(page + 1)} / ${count(pages)}`
    : '0 records';
  element<HTMLButtonElement>('previous-page').disabled = page === 0;
  element<HTMLButtonElement>('next-page').disabled = page === pages - 1;
  element('scope-control').hidden =
    filters.grouping !== 'source' || !filters.group || !index.sources.has(filters.group);
  const hints: Record<Grouping, string> = {
    type: 'Group records by their reported type.',
    source: 'Choose a file. Include dependencies to explore its reachable files too.',
    folder: 'Group by source URL directory. Inline scripts use their parent directory.',
    entry:
      'Follow every reachable dependency from an entry. Shared files can appear in multiple entries.',
  };
  element('group-hint').textContent = hints[filters.grouping];
  const scope = element('active-scope');
  scope.hidden = filters.group === null;
  scope.textContent =
    filters.group === null
      ? ''
      : `${human(filters.grouping)}: ${filters.group}${filters.grouping === 'source' && filters.includeDependencies ? ' + dependencies' : ''}`;
  if (filters.group !== null) {
    const clear = document.createElement('button');
    clear.className = 'scope-clear';
    clear.dataset.action = 'group-all';
    clear.setAttribute('aria-label', 'Clear group filter');
    clear.title = 'Clear group filter';
    clear.textContent = '×';
    scope.append(clear);
  }
  if (updateDetails) {
    element('inspector').innerHTML = String(
      inspector(
        index,
        selected ? index.byId.get(selected) : undefined,
        graphExpanded,
        viewer,
        sourceTexts,
      ),
    );
    scrollCode();
  }
  if (groupFocus && document.activeElement === document.body) {
    [...element('group-list').querySelectorAll<HTMLButtonElement>('[data-action]')]
      .find(
        (button) =>
          button.dataset.action === groupFocus.action && button.dataset.group === groupFocus.group,
      )
      ?.focus({ preventScroll: true });
  }
}

function header(file: File) {
  if (!index) return;
  const manifest = index.manifest;
  element('filename').textContent = file.name;
  element('filename').title = file.name;
  element('scan-status').innerHTML = String(badge(manifest.status));
  element('manifest-meta').textContent =
    `Scanner ${manifest.scannerVersion} · ${manifest.dependencyMode} · ${bytes(file.size)} manifest`;
  const unresolved = manifest.dependencies.filter(
    (dependency) => dependency.status === 'unresolved',
  ).length;
  const gaps = manifest.gaps.reduce((total, gap) => total + gap.count, 0);
  const metrics = [
    {
      value: count(manifest.findings.length),
      name: 'Recorded findings',
      note: `${count(manifest.findings.filter((finding) => finding.severity === 'error').length)} error · ${count(manifest.findings.filter((finding) => finding.severity === 'warning').length)} warning`,
      view: 'findings',
    },
    {
      value: count(manifest.sources.length),
      name: 'Source files',
      note: `${bytes(manifest.summary.bytes)} scanned`,
      view: 'sources',
    },
    {
      value: count(manifest.dependencies.length),
      name: 'Dependencies',
      note: 'Recorded references',
      view: 'dependencies',
    },
    {
      value: count(unresolved),
      name: 'Unresolved references',
      note: unresolved ? 'References without analysis' : 'No unresolved references',
      view: 'dependencies',
      status: 'unresolved',
    },
    {
      value: count(gaps),
      name: 'Coverage gap occurrences',
      note: `${count(manifest.gaps.length)} grouped records`,
      view: 'gaps',
    },
    {
      value: time(manifest.summary.elapsedMs),
      name: 'Reported scan time',
      note: 'Scheduler wall time',
      view: null,
    },
  ];
  element('stats').innerHTML = String(
    html`${metrics.map((metric) => {
      const content = html`<span class="stat-value">${metric.value}</span
        ><span class="stat-name">${metric.name}</span
        ><span class="stat-note">${metric.note}</span>`;
      return metric.view
        ? html`<button
            class="stat"
            data-action="metric"
            data-view="${metric.view}"
            data-status="${metric.status || ''}"
          >
            ${content}
          </button>`
        : html`<div class="stat">${content}</div>`;
    })}`,
  );
  for (const view of ['findings', 'sources', 'dependencies', 'gaps'] as View[])
    element(`count-${view}`).textContent = count(index.rows[view].length);
}

async function openFile(file: File) {
  if (file.size > maxBytes) {
    message('This file is larger than 64 MiB. Choose a smaller manifest.');
    return;
  }
  worker?.terminate();
  message('');
  element('loading').hidden = false;
  let active: Worker;
  try {
    active = new Worker(new URL('./manifest-worker.js', import.meta.url), { type: 'module' });
  } catch {
    element('loading').hidden = true;
    message('The JSON worker could not start. Run pnpm playground and open the local HTTP URL.');
    return;
  }
  worker = active;
  active.onmessage = (
    event: MessageEvent<{ ok: boolean; error?: string; manifest: Manifest; notices: string[] }>,
  ) => {
    if (worker !== active) return;
    active.terminate();
    worker = undefined;
    element('loading').hidden = true;
    if (!event.data.ok) {
      message(event.data.error || 'Unable to load this manifest.');
      return;
    }
    try {
      const loaded = new ManifestIndex(event.data.manifest);
      index = loaded;
      filters = initialFilters();
      groupQuery = '';
      page = 0;
      graphExpanded = false;
      viewer = undefined;
      sourceTexts.clear();
      codeScrollKey = '';
      selected = index.rows.findings[0]?.id ?? index.rows.sources[0]?.id ?? null;
      if (selected) {
        filters.view = index.byId.get(selected)!.view;
        page = Math.floor(index.byId.get(selected)!.index / pageSize);
      }
      document.body.classList.remove('detail-open');
      element('welcome').hidden = true;
      element('workspace').hidden = false;
      element('clear-button').hidden = false;
      clampPanels();
      header(file);
      syncFilters();
      render();
      if (event.data.notices.length) message(event.data.notices.join(' '));
      toast(`Opened ${file.name}`);
    } catch (error) {
      message(error instanceof Error ? error.message : 'Unable to display this manifest.');
    }
  };
  active.onerror = () => {
    if (worker !== active) return;
    active.terminate();
    worker = undefined;
    element('loading').hidden = true;
    message('The JSON worker stopped unexpectedly. Reload the playground and try again.');
  };
  active.postMessage({ file });
}

function selectRecord(id: string) {
  if (!index?.byId.has(id)) return;
  const changed = selected !== id;
  selected = id;
  graphExpanded = false;
  syncViewer(index.byId.get(id));
  if (viewer && changed) viewer.radius = DEFAULT_RADIUS;
  render();
  const inspectorEl = element('inspector');
  const panel = viewer ? inspectorEl.querySelector<HTMLElement>('.code-panel') : null;
  inspectorEl.scrollTop = panel
    ? Math.max(
        0,
        panel.getBoundingClientRect().top -
          inspectorEl.getBoundingClientRect().top +
          inspectorEl.scrollTop -
          96,
      )
    : 0;
  document.body.classList.add('detail-open');
  if (matchMedia('(max-width: 1000px)').matches) {
    element('inspector').querySelector<HTMLButtonElement>('.close-detail')?.focus();
  } else {
    element('record-list')
      .querySelector<HTMLButtonElement>(`[data-record="${id}"]`)
      ?.focus({ preventScroll: true });
  }
}

function closeDetails() {
  document.body.classList.remove('detail-open');
  if (selected) {
    element('record-list')
      .querySelector<HTMLButtonElement>(`[data-record="${selected}"]`)
      ?.focus({ preventScroll: true });
  }
}

function navigate(view: View, source?: string, status = '') {
  filters = {
    ...initialFilters(),
    view,
    severity: status,
    grouping: source ? 'source' : filters.grouping,
    group: source ?? null,
  };
  groupQuery = '';
  page = 0;
  selected = null;
  graphExpanded = false;
  viewer = undefined;
  document.body.classList.remove('detail-open');
  syncFilters();
  render();
  // Keep the inspector populated after a jump (for example "View N
  // findings"): select the first visible record without opening the
  // narrow-screen overlay or moving focus.
  const first = visibleRows[0];
  if (first) {
    selected = first.id;
    syncViewer(first);
    render();
  }
}

async function copy(text: string, confirmation: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast(confirmation);
  } catch {
    toast('Clipboard is unavailable. Select and copy the raw record instead.');
  }
}

document.addEventListener('click', (event) => {
  const button = (event.target as HTMLElement).closest<HTMLElement>('[data-action]');
  if (!button) return;
  const action = button.dataset.action;
  if (action === 'home') {
    event.preventDefault();
    window.scrollTo({ top: 0 });
    return;
  }
  if (action === 'upload') {
    element<HTMLInputElement>('file-input').click();
    return;
  }
  if (action === 'clear') {
    dropManifestParams();
    worker?.terminate();
    worker = undefined;
    index = undefined;
    selected = null;
    filters = initialFilters();
    visibleRows = [];
    groupQuery = '';
    page = 0;
    graphExpanded = false;
    viewer = undefined;
    sourceTexts.clear();
    codeScrollKey = '';
    element('workspace').hidden = true;
    element('welcome').hidden = false;
    element('clear-button').hidden = true;
    element('loading').hidden = true;
    element('filename').textContent = 'No manifest loaded';
    element('filename').title = 'No manifest loaded';
    for (const id of [
      'inspector',
      'record-list',
      'group-list',
      'stats',
      'manifest-meta',
      'active-scope',
      'scan-status',
    ])
      element(id).replaceChildren();
    syncFilters();
    document.body.classList.remove('detail-open');
    message('');
    return;
  }
  if (!index) return;
  if (action === 'record') {
    selectRecord(button.dataset.record!);
    return;
  }
  if (action === 'record-jump') {
    // Used by same-file finding navigation: scope the list to that file so
    // the target is visible even when the current filters excluded it.
    const target = index.byId.get(button.dataset.record!);
    if (!target) return;
    filters = {
      ...initialFilters(),
      view: target.view,
      grouping: target.source ? 'source' : 'type',
      group: target.source ?? null,
    };
    groupQuery = '';
    syncFilters();
    const rows = index.filter(index.baseRows(filters), filters);
    const at = rows.findIndex((row) => row.id === target.id);
    page = at >= 0 ? Math.floor(at / pageSize) : 0;
    selectRecord(target.id);
    return;
  }
  if (action === 'source-open' || action === 'source-retry') {
    const row = selected ? index.byId.get(selected) : undefined;
    const target = row && viewerTarget(index, row);
    if (!target) return;
    viewer = {
      sourceId: target.source.id,
      url: target.url,
      radius: DEFAULT_RADIUS,
      wrap: viewer?.wrap ?? false,
    };
    if (action === 'source-retry') sourceTexts.delete(target.url);
    codeScrollKey = '';
    void fetchSource(target.url);
    render();
    return;
  }
  if (action === 'source-close') {
    viewer = undefined;
    render();
    return;
  }
  if (action === 'source-expand') {
    if (viewer) viewer.radius = Math.min(viewer.radius * 3, 5000);
    render();
    return;
  }
  if (action === 'source-wrap') {
    if (viewer) viewer.wrap = !viewer.wrap;
    render();
    return;
  }
  if (action === 'source') {
    const source = button.dataset.source!;
    navigate('sources', source);
    const row = index.rows.sources.find((row) => row.source === source);
    if (row) selectRecord(row.id);
    return;
  }
  if (action === 'file-findings' || action === 'file-dependencies') {
    navigate(action === 'file-findings' ? 'findings' : 'dependencies', button.dataset.source!);
    return;
  }
  if (action === 'view' || action === 'metric') {
    if (button.dataset.view)
      navigate(button.dataset.view as View, undefined, button.dataset.status);
    return;
  }
  if (action === 'grouping') {
    filters.grouping = button.dataset.mode as Grouping;
    filters.group = null;
    filters.includeDependencies = false;
    groupQuery = '';
    page = 0;
    syncFilters();
    render();
    return;
  }
  if (action === 'group' || action === 'group-all') {
    filters.group = action === 'group' ? button.dataset.group! : null;
    page = 0;
    render();
    return;
  }
  if (action === 'previous-page' || action === 'next-page') {
    page += action === 'previous-page' ? -1 : 1;
    render(false);
    element('record-list').scrollTop = 0;
    return;
  }
  if (action === 'reset-filters') {
    navigate(filters.view);
    filters.grouping = 'type';
    syncFilters();
    render();
    return;
  }
  if (action === 'expand-graph') {
    graphExpanded = true;
    render();
    return;
  }
  if (action === 'close-detail') {
    closeDetails();
    return;
  }
  if (action === 'copy-record') {
    const row = index.byId.get(button.dataset.record!);
    if (row) void copy(JSON.stringify(row.data, null, 2), 'Record JSON copied');
  }
});

element<HTMLInputElement>('file-input').addEventListener('change', (event) => {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = '';
  if (file) {
    dropManifestParams();
    void openFile(file);
  }
});
element<HTMLInputElement>('search').addEventListener('input', (event) => {
  filters.query = (event.target as HTMLInputElement).value;
  page = 0;
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => render(), 120);
});
element<HTMLInputElement>('group-search').addEventListener('input', (event) => {
  groupQuery = (event.target as HTMLInputElement).value;
  render(false);
});
element<HTMLSelectElement>('status-filter').addEventListener('change', (event) => {
  filters.severity = (event.target as HTMLSelectElement).value;
  page = 0;
  render();
});
element<HTMLSelectElement>('confidence-filter').addEventListener('change', (event) => {
  filters.confidence = (event.target as HTMLSelectElement).value;
  page = 0;
  render();
});
element<HTMLInputElement>('include-dependencies').addEventListener('change', (event) => {
  filters.includeDependencies = (event.target as HTMLInputElement).checked;
  page = 0;
  render();
});
element<HTMLSelectElement>('page-size').addEventListener('change', (event) => {
  pageSize = Number((event.target as HTMLSelectElement).value);
  page = 0;
  render(false);
});

let dragDepth = 0;
document.addEventListener('dragenter', (event) => {
  if (!event.dataTransfer?.types.includes('Files')) return;
  event.preventDefault();
  dragDepth++;
  element('drop-overlay').hidden = false;
});
document.addEventListener('dragover', (event) => {
  if (event.dataTransfer?.types.includes('Files')) {
    event.preventDefault();
    event.dataTransfer.dropEffect = 'copy';
  }
});
document.addEventListener('dragleave', () => {
  dragDepth = Math.max(0, dragDepth - 1);
  if (!dragDepth) element('drop-overlay').hidden = true;
});
document.addEventListener('drop', (event) => {
  event.preventDefault();
  dragDepth = 0;
  element('drop-overlay').hidden = true;
  const files = event.dataTransfer?.files;
  if (files && files.length > 1) {
    message('Open one manifest at a time.');
    return;
  }
  if (files?.[0]) {
    dropManifestParams();
    void openFile(files[0]);
  }
});
// Arrow keys move through the record list, flipping pages at the edges.
element('record-list').addEventListener('keydown', (event) => {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
  const buttons = [...element('record-list').querySelectorAll<HTMLButtonElement>('[data-record]')];
  const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
  if (at === -1) return;
  event.preventDefault();
  const step = event.key === 'ArrowDown' ? 1 : -1;
  const next = buttons[at + step];
  if (next) {
    selectRecord(next.dataset.record!);
    return;
  }
  const globalAt = page * pageSize + at + step;
  const target = visibleRows[globalAt];
  if (!target) return;
  page = Math.floor(globalAt / pageSize);
  selectRecord(target.id);
  element('record-list').scrollTop = step > 0 ? 0 : element('record-list').scrollHeight;
});
document.addEventListener('keydown', (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === 'k') {
    event.preventDefault();
    if (index) element<HTMLInputElement>('search').focus();
    else element<HTMLInputElement>('file-input').click();
  }
  if (event.key === 'Escape') closeDetails();
});
window.addEventListener('beforeunload', () => worker?.terminate());

// ---------------------------------------------------------------------------
// Resizable explorer columns. Widths are layout preferences only; manifest
// data itself is never persisted.
// ---------------------------------------------------------------------------
const LAYOUT_KEY = 'bloodhound-playground-layout';
const MIN_RECORDS = 280;

function panelWidth(selector: string): number {
  return element('explorer').querySelector(selector)?.getBoundingClientRect().width ?? 0;
}

function persistLayout() {
  try {
    localStorage.setItem(
      LAYOUT_KEY,
      JSON.stringify({
        sidebar: Math.round(panelWidth('.sidebar')),
        inspector: Math.round(panelWidth('.inspector')),
      }),
    );
  } catch {
    // Layout memory is optional (private mode, disabled storage).
  }
}

function setPanel(side: 'sidebar' | 'inspector', width: number) {
  const explorer = element('explorer');
  const other = panelWidth(side === 'sidebar' ? '.inspector' : '.sidebar');
  const min = side === 'sidebar' ? 160 : 300;
  const max = Math.max(min, explorer.clientWidth - other - MIN_RECORDS - 12);
  const clamped = Math.min(Math.max(width, min), max);
  explorer.style.setProperty(
    side === 'sidebar' ? '--col-sidebar' : '--col-inspector',
    `${clamped}px`,
  );
  explorer
    .querySelector(`[data-splitter="${side}"]`)
    ?.setAttribute('aria-valuenow', String(Math.round(clamped)));
}

/** Re-clamps saved widths when the window is narrower than when saved. */
function clampPanels() {
  const explorer = element('explorer');
  if (!explorer.clientWidth) return;
  if (explorer.style.getPropertyValue('--col-sidebar')) setPanel('sidebar', panelWidth('.sidebar'));
  if (explorer.style.getPropertyValue('--col-inspector'))
    setPanel('inspector', panelWidth('.inspector'));
}

function initSplitters() {
  const explorer = element('explorer');
  window.addEventListener('resize', clampPanels);
  try {
    const saved = JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? 'null') as {
      sidebar?: unknown;
      inspector?: unknown;
    } | null;
    if (typeof saved?.sidebar === 'number')
      explorer.style.setProperty('--col-sidebar', `${saved.sidebar}px`);
    if (typeof saved?.inspector === 'number')
      explorer.style.setProperty('--col-inspector', `${saved.inspector}px`);
  } catch {
    // Keep stylesheet defaults.
  }
  for (const splitter of explorer.querySelectorAll<HTMLElement>('[data-splitter]')) {
    const side = splitter.dataset.splitter as 'sidebar' | 'inspector';
    splitter.setAttribute('aria-valuemin', side === 'sidebar' ? '160' : '300');
    let startX = 0;
    let startWidth = 0;
    let dragging = false;
    splitter.addEventListener('pointerdown', (event) => {
      dragging = true;
      startX = event.clientX;
      startWidth = panelWidth(side === 'sidebar' ? '.sidebar' : '.inspector');
      splitter.setPointerCapture(event.pointerId);
      document.body.classList.add('resizing');
      event.preventDefault();
    });
    splitter.addEventListener('pointermove', (event) => {
      if (!dragging) return;
      const dx = event.clientX - startX;
      setPanel(side, side === 'sidebar' ? startWidth + dx : startWidth - dx);
    });
    const end = () => {
      if (!dragging) return;
      dragging = false;
      document.body.classList.remove('resizing');
      persistLayout();
    };
    splitter.addEventListener('pointerup', end);
    splitter.addEventListener('pointercancel', end);
    splitter.addEventListener('dblclick', () => {
      explorer.style.removeProperty('--col-sidebar');
      explorer.style.removeProperty('--col-inspector');
      try {
        localStorage.removeItem(LAYOUT_KEY);
      } catch {
        // Already reset for this session.
      }
    });
    splitter.addEventListener('keydown', (event) => {
      if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
      event.preventDefault();
      const step = (event.shiftKey ? 32 : 8) * (event.key === 'ArrowRight' ? 1 : -1);
      const current = panelWidth(side === 'sidebar' ? '.sidebar' : '.inspector');
      setPanel(side, side === 'sidebar' ? current + step : current - step);
      persistLayout();
    });
  }
}
initSplitters();
void autoloadManifest();

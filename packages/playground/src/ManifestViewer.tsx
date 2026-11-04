import { useDeferredValue, useMemo, useState, type ReactNode } from 'react';
import {
  ManifestIndex,
  OriginalSources,
  sourceName,
  type Filters,
  type Grouping,
  type Manifest,
  type ManifestSource,
  type ProjectWorkspace,
  type RecordRow,
  type TextLoader,
  type View,
} from '@bloodhound-rs/manifest';
import { defaultComponents, type ManifestComponents, type TableColumn } from './components.js';
import { SourcePreview } from './SourcePreview.js';
import { ViewerLayout } from './ViewerLayout.js';
import { englishMessages } from './locales/en.js';
import { MessagesProvider, useMessages, type PlaygroundMessages } from './messages.js';

export interface RecordRenderContext {
  selected: boolean;
  openDetails: () => void;
  components: ManifestComponents;
  messages: PlaygroundMessages;
}
export type RecordRenderer = (row: RecordRow, context: RecordRenderContext) => ReactNode;

export interface ManifestViewerProps {
  /** Already parsed report; replace this object when the build changes. */
  manifest: Manifest;
  components?: Partial<ManifestComponents>;
  messages?: Partial<PlaygroundMessages>;
  /** Optional first/last columns for host-owned markers, tags and actions. */
  renderRecordBefore?: RecordRenderer;
  renderRecordAfter?: RecordRenderer;
  /** Base URL for relative source map references. */
  manifestUrl?: string;
  workspace?: ProjectWorkspace;
  loadText?: TextLoader;
  notices?: readonly string[];
  pageSize?: number;
  className?: string;
  onSelect?: (row: RecordRow) => void;
}

/** Display only: no file picker, manifest request, router or application provider. */
export function ManifestViewer({ messages = englishMessages, ...props }: ManifestViewerProps) {
  const { manifest, manifestUrl, workspace, loadText } = props;
  const [session, setSession] = useState({
    manifest,
    manifestUrl,
    workspace,
    loadText,
    version: 0,
  });
  // Reset all selection, filters and source requests together when the host changes reports.
  if (
    session.manifest !== manifest ||
    session.manifestUrl !== manifestUrl ||
    session.workspace !== workspace ||
    session.loadText !== loadText
  ) {
    setSession({ manifest, manifestUrl, workspace, loadText, version: session.version + 1 });
  }
  return (
    <MessagesProvider messages={messages}>
      <ViewerSession key={session.version} {...props} />
    </MessagesProvider>
  );
}

const views: View[] = ['findings', 'sources', 'dependencies', 'gaps'];
const groupings: Grouping[] = ['type', 'source', 'folder', 'entry'];
const initialFilters: Filters = {
  view: 'findings',
  query: '',
  severity: '',
  confidence: '',
  grouping: 'type',
  group: null,
  includeDependencies: false,
  includePolyfills: false,
};

function ViewerSession({
  manifest,
  components,
  manifestUrl,
  workspace,
  loadText,
  notices,
  pageSize = 50,
  className = '',
  onSelect,
  renderRecordBefore,
  renderRecordAfter,
}: ManifestViewerProps) {
  const { text: t, label, messages } = useMessages();
  const ui = { ...defaultComponents, ...components };
  const { Button, Input, Select, Table, Tag } = ui;
  const index = useMemo(() => new ManifestIndex(manifest), [manifest]);
  const originals = useMemo(
    () => new OriginalSources(manifestUrl, workspace, loadText),
    [manifestUrl, workspace, loadText],
  );
  const [filters, setFilters] = useState(initialFilters);
  const [selected, setSelected] = useState<string>();
  const [page, setPage] = useState(0);
  const query = useDeferredValue(filters.query);
  const baseRows = useMemo(
    () => index.baseRows({ ...filters, query }),
    [index, filters.view, query, filters.severity, filters.confidence, filters.includePolyfills],
  );
  const groups = useMemo(
    () => index.groups(baseRows, filters.grouping),
    [index, baseRows, filters.grouping],
  );
  const rows = useMemo(
    () => index.filter(baseRows, filters),
    [index, baseRows, filters.group, filters.grouping, filters.includeDependencies],
  );
  const size = Math.max(1, Math.floor(pageSize));
  const pages = Math.max(1, Math.ceil(rows.length / size));
  const activePage = Math.min(page, pages - 1);
  const row = selected ? index.byId.get(selected) : undefined;

  function filter(patch: Partial<Filters>) {
    setFilters((value) => ({ ...value, ...patch }));
    setPage(0);
    setSelected(undefined);
  }
  function select(row: RecordRow) {
    setSelected(row.id);
    onSelect?.(row);
  }
  function selectSource(id: string) {
    const row = index.rows.sources.find((row) => row.source === id);
    if (row) select(row);
  }

  function sourceTitle(source: ManifestSource) {
    if (source.inline) return t('source.inline', { kind: label(source.kind), id: source.id });
    if (!source.url)
      return t('source.anonymous', { kind: label(source.kind).toUpperCase(), id: source.id });
    return sourceName(source);
  }

  function recordTitle(record: RecordRow) {
    if (record.view === 'sources' && record.source)
      return sourceTitle(index.sources.get(record.source)!);
    if (
      record.view === 'dependencies' &&
      'from' in record.data &&
      !record.data.specifier &&
      !record.data.url &&
      !record.data.to
    )
      return t('record.unresolved');
    return record.title;
  }

  function groupTitle(group: (typeof groups)[number]) {
    const source =
      filters.grouping === 'source' || filters.grouping === 'entry'
        ? index.sources.get(group.id)
        : undefined;
    return source ? sourceTitle(source) : label(group.name);
  }

  function recordExtras(render: RecordRenderer, record: RecordRow) {
    return (
      <div className="bh-record-extras" onClick={(event) => event.stopPropagation()}>
        {render(record, {
          selected: selected === record.id,
          openDetails: () => select(record),
          components: ui,
          messages,
        })}
      </div>
    );
  }

  const columns: TableColumn[] = [
    {
      title: t('record.title'),
      dataIndex: 'title',
      render: (_, row) => (
        <Button variant="link" onClick={() => select(row)}>
          {recordTitle(row)}
          {'writes' in row.data && row.data.writes?.length
            ? ` · ${t(row.type === 'module-state-write' ? 'record.writes' : 'record.occurrences', { count: row.data.writes.length })}`
            : ''}
        </Button>
      ),
    },
    {
      title: t('record.type'),
      dataIndex: 'type',
      render: (_, row) => t('record.typeValue', { value: label(row.type) }),
    },
    {
      title: t('record.status'),
      dataIndex: 'status',
      render: (_, row) => (
        <Tag status={row.status}>{t('record.statusValue', { value: label(row.status) })}</Tag>
      ),
    },
    {
      title: t('record.source'),
      dataIndex: 'source',
      render: (_, row) =>
        row.source ? sourceTitle(index.sources.get(row.source)!) : t('record.scanWide'),
    },
  ];
  if (renderRecordBefore)
    columns.unshift({
      title: t('record.before'),
      dataIndex: 'before',
      render: (_, row) => recordExtras(renderRecordBefore, row),
    });
  if (renderRecordAfter)
    columns.push({
      title: t('record.after'),
      dataIndex: 'after',
      render: (_, row) => recordExtras(renderRecordAfter, row),
    });
  const source = row?.source ? index.sources.get(row.source) : undefined;
  const statusOptions = [...new Set(index.rows[filters.view].map((row) => row.status))];

  return (
    <section className={`bh-viewer ${className}`} aria-label={t('viewer.label')}>
      <header className="bh-heading">
        <strong>{t('viewer.title')}</strong>
        <Tag status={manifest.status}>
          {t('record.statusValue', { value: label(manifest.status) })}
        </Tag>
        <span>
          {t('viewer.scanner', {
            version: manifest.scannerVersion,
            mode: label(manifest.dependencyMode),
          })}
        </span>
      </header>
      <p className="bh-muted">{t('viewer.disclaimer')}</p>
      {notices?.map((notice) => (
        <p role="status" key={notice}>
          {t('viewer.notice', { message: notice })}
        </p>
      ))}
      <nav className="bh-controls bh-categories" aria-label={t('view.categories')}>
        {views.map((view) => (
          <Button
            key={view}
            variant="toggle"
            aria-pressed={filters.view === view}
            onClick={() => {
              if (filters.view !== view)
                filter({ ...initialFilters, view, includePolyfills: filters.includePolyfills });
            }}
          >
            {t('view.count', {
              name: t(`view.${view}`),
              count:
                index.rows[view].length -
                (view === 'findings' && !filters.includePolyfills ? index.polyfills.size : 0),
            })}
          </Button>
        ))}
      </nav>
      {filters.view === 'findings' && index.polyfills.size > 0 ? (
        <div className="bh-controls">
          <span>
            {t(filters.includePolyfills ? 'polyfill.included' : 'polyfill.hidden', {
              count: index.polyfills.size,
            })}
          </span>
          <Button
            variant="toggle"
            aria-pressed={Boolean(filters.includePolyfills)}
            onClick={() => filter({ includePolyfills: !filters.includePolyfills })}
          >
            {t(filters.includePolyfills ? 'polyfill.hide' : 'polyfill.show')}
          </Button>
        </div>
      ) : null}
      <div className="bh-controls bh-filters">
        <Input
          value={filters.query}
          onChange={(query) => filter({ query })}
          placeholder={t('filter.search')}
          aria-label={t('filter.searchLabel')}
        />
        <Select
          value={filters.severity}
          onChange={(severity) => filter({ severity, group: null })}
          aria-label={t('filter.status')}
          options={[
            { label: t('filter.allStatuses'), value: '' },
            ...statusOptions.map((value) => ({
              label: t('record.statusValue', { value: label(value) }),
              value,
            })),
          ]}
        />
        {filters.view === 'findings' ? (
          <Select
            value={filters.confidence}
            onChange={(confidence) => filter({ confidence, group: null })}
            aria-label={t('filter.confidence')}
            options={[
              { label: t('filter.allConfidence'), value: '' },
              { label: t('filter.high'), value: 'high' },
              { label: t('filter.medium'), value: 'medium' },
            ]}
          />
        ) : null}
        <Select
          value={filters.grouping}
          onChange={(value) => filter({ grouping: value as Grouping, group: null })}
          aria-label={t('filter.groupBy')}
          options={groupings.map((value) => ({ label: t(`group.${value}`), value }))}
        />
      </div>
      <ViewerLayout
        selected={selected}
        groups={
          <>
            <Button
              variant="toggle"
              aria-pressed={filters.group === null}
              onClick={() => {
                if (filters.group !== null) filter({ group: null });
              }}
            >
              {t('group.all', { count: baseRows.length })}
            </Button>
            {groups.map((group) => (
              <Button
                key={group.id}
                title={t('group.name', { name: groupTitle(group) })}
                variant="toggle"
                aria-pressed={filters.group === group.id}
                onClick={() => {
                  if (filters.group !== group.id) filter({ group: group.id });
                }}
              >
                {t('view.count', {
                  name: t('group.name', { name: groupTitle(group) }),
                  count: group.count,
                })}
              </Button>
            ))}
            {filters.grouping === 'source' ? (
              <Button
                variant="toggle"
                aria-pressed={filters.includeDependencies}
                onClick={() => filter({ includeDependencies: !filters.includeDependencies })}
              >
                {t('group.includeDependencies')}
              </Button>
            ) : null}
          </>
        }
        results={
          <>
            <div className="bh-table-scroll">
              <Table
                columns={columns}
                data={rows.slice(activePage * size, (activePage + 1) * size)}
                rowKey="id"
                pagination={false}
                selectedRowId={selected}
              />
            </div>
            {rows.length === 0 ? <p role="status">{t('record.empty')}</p> : null}
            <div className="bh-controls" aria-label={t('pagination.label')}>
              <Button disabled={activePage === 0} onClick={() => setPage(activePage - 1)}>
                {t('pagination.previous')}
              </Button>
              <span>
                {t('pagination.summary', { page: activePage + 1, pages, count: rows.length })}
              </span>
              <Button disabled={activePage + 1 >= pages} onClick={() => setPage(activePage + 1)}>
                {t('pagination.next')}
              </Button>
            </div>
          </>
        }
        detail={
          row ? (
            <section className="bh-detail">
              <div className="bh-heading">
                <strong>{recordTitle(row)}</strong>
                <Button variant="text" onClick={() => setSelected(undefined)}>
                  {t('record.close')}
                </Button>
              </div>
              <p>
                {t('record.typeValue', { value: label(row.type) })} ·{' '}
                {t('record.statusValue', { value: label(row.status) })}
                {row.confidence
                  ? ` · ${t('record.confidence', { value: label(row.confidence) })}`
                  : ''}
              </p>
              {source ? <p className="bh-path">{source.file ?? source.url ?? source.id}</p> : null}
              <SourcePreview
                key={row.id}
                index={index}
                row={row}
                originals={originals}
                components={ui}
                customLoader={Boolean(loadText)}
              />
              <details>
                <summary>{t('record.json')}</summary>
                <pre className="bh-snippet">{JSON.stringify(row.data, null, 2)}</pre>
              </details>
              {source && source.chain.length > 0 ? (
                <div className="bh-controls" aria-label={t('record.entryChain')}>
                  {t('record.entryChain')}{' '}
                  {source.chain.map((id) => (
                    <Button key={id} variant="link" onClick={() => selectSource(id)}>
                      {sourceTitle(index.sources.get(id)!)}
                    </Button>
                  ))}
                </div>
              ) : null}
              {source ? (
                <div className="bh-controls" aria-label={t('record.dependencies')}>
                  {t('record.dependencies')}{' '}
                  {[...(index.children.get(source.id) ?? [])].map((id) => (
                    <Button key={id} variant="link" onClick={() => selectSource(id)}>
                      {sourceTitle(index.sources.get(id)!)}
                    </Button>
                  ))}
                </div>
              ) : null}
            </section>
          ) : null
        }
      />
    </section>
  );
}

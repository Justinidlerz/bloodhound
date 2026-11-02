import { useDeferredValue, useMemo, useState } from 'react';
import {
  ManifestIndex,
  OriginalSources,
  sourceName,
  type Filters,
  type Grouping,
  type Manifest,
  type ProjectWorkspace,
  type RecordRow,
  type TextLoader,
  type View,
} from '@bloodhound-rs/manifest';
import { defaultComponents, type ManifestComponents, type TableColumn } from './components.js';
import { SourcePreview } from './SourcePreview.js';

export interface ManifestViewerProps {
  /** Already parsed report; replace this object when the build changes. */
  manifest: Manifest;
  components?: Partial<ManifestComponents>;
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
export function ManifestViewer(props: ManifestViewerProps) {
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
  return <ViewerSession key={session.version} {...props} />;
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
}: ManifestViewerProps) {
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
    [index, filters.view, query, filters.severity, filters.confidence],
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

  const columns: TableColumn[] = [
    {
      title: 'Record',
      dataIndex: 'title',
      render: (_, row) => <Button onClick={() => select(row)}>{row.title}</Button>,
    },
    { title: 'Type', dataIndex: 'type', render: (_, row) => row.type },
    { title: 'Status', dataIndex: 'status', render: (_, row) => <Tag>{row.status}</Tag> },
    {
      title: 'Source',
      dataIndex: 'source',
      render: (_, row) => (row.source ? sourceName(index.sources.get(row.source)!) : 'Scan-wide'),
    },
  ];
  const source = row?.source ? index.sources.get(row.source) : undefined;
  const statusOptions = [...new Set(index.rows[filters.view].map((row) => row.status))];

  return (
    <section className={`bh-viewer ${className}`} aria-label="Bloodhound manifest report">
      <header className="bh-heading">
        <strong>Manifest report</strong>
        <Tag>{manifest.status}</Tag>
        <span>
          Scanner {manifest.scannerVersion} · {manifest.dependencyMode}
        </span>
      </header>
      <p className="bh-muted">
        Static findings are candidates. Unresolved references and coverage gaps describe code the
        scan could not fully analyze.
      </p>
      {notices?.map((notice) => (
        <p role="status" key={notice}>
          {notice}
        </p>
      ))}
      <nav className="bh-controls" aria-label="Record categories">
        {views.map((view) => (
          <Button
            key={view}
            disabled={filters.view === view}
            onClick={() => filter({ ...initialFilters, view })}
          >
            {view} ({index.rows[view].length})
          </Button>
        ))}
      </nav>
      <div className="bh-controls">
        <Input
          value={filters.query}
          onChange={(query) => filter({ query })}
          placeholder="Search records, paths or code"
          aria-label="Search records"
        />
        <Select
          value={filters.severity}
          onChange={(severity) => filter({ severity, group: null })}
          aria-label="Status"
          options={[
            { label: 'All statuses', value: '' },
            ...statusOptions.map((value) => ({ label: value, value })),
          ]}
        />
        {filters.view === 'findings' ? (
          <Select
            value={filters.confidence}
            onChange={(confidence) => filter({ confidence, group: null })}
            aria-label="Confidence"
            options={[
              { label: 'All confidence levels', value: '' },
              { label: 'High', value: 'high' },
              { label: 'Medium', value: 'medium' },
            ]}
          />
        ) : null}
        <Select
          value={filters.grouping}
          onChange={(value) => filter({ grouping: value as Grouping, group: null })}
          aria-label="Group by"
          options={groupings.map((value) => ({ label: `By ${value}`, value }))}
        />
      </div>
      <div className="bh-layout">
        <aside className="bh-groups" aria-label="Record groups">
          <Button disabled={filters.group === null} onClick={() => filter({ group: null })}>
            All ({baseRows.length})
          </Button>
          {groups.map((group) => (
            <Button
              key={group.id}
              disabled={filters.group === group.id}
              onClick={() => filter({ group: group.id })}
            >
              {group.name} ({group.count})
            </Button>
          ))}
          {filters.grouping === 'source' ? (
            <Button onClick={() => filter({ includeDependencies: !filters.includeDependencies })}>
              {filters.includeDependencies ? 'Exclude dependencies' : 'Include dependencies'}
            </Button>
          ) : null}
        </aside>
        <div className="bh-results">
          <div className="bh-table-scroll">
            <Table
              columns={columns}
              data={rows.slice(activePage * size, (activePage + 1) * size)}
              rowKey="id"
              pagination={false}
            />
          </div>
          {rows.length === 0 ? <p role="status">No matching records.</p> : null}
          <div className="bh-controls" aria-label="Pagination">
            <Button disabled={activePage === 0} onClick={() => setPage(activePage - 1)}>
              Previous
            </Button>
            <span>
              {activePage + 1} / {pages} · {rows.length} records
            </span>
            <Button disabled={activePage + 1 >= pages} onClick={() => setPage(activePage + 1)}>
              Next
            </Button>
          </div>
          {row ? (
            <section className="bh-detail" aria-label="Record details">
              <div className="bh-heading">
                <strong>{row.title}</strong>
                <Button onClick={() => setSelected(undefined)}>Close details</Button>
              </div>
              <p>
                {row.type} · {row.status}
                {row.confidence ? ` · ${row.confidence} confidence` : ''}
              </p>
              {source ? <p className="bh-path">{source.file ?? source.url ?? source.id}</p> : null}
              {'location' in row.data && row.data.location ? (
                <p>
                  Line {row.data.location.line}, column {row.data.location.column}
                </p>
              ) : null}
              {'detail' in row.data && row.data.detail ? <p>{row.data.detail}</p> : null}
              {'snippet' in row.data ? <pre className="bh-snippet">{row.data.snippet}</pre> : null}
              {source && source.chain.length > 0 ? (
                <div className="bh-controls" aria-label="Entry chain">
                  Entry chain:{' '}
                  {source.chain.map((id) => (
                    <Button key={id} onClick={() => selectSource(id)}>
                      {sourceName(index.sources.get(id)!)}
                    </Button>
                  ))}
                </div>
              ) : null}
              {source ? (
                <div className="bh-controls" aria-label="Dependencies">
                  Dependencies:{' '}
                  {[...(index.children.get(source.id) ?? [])].map((id) => (
                    <Button key={id} onClick={() => selectSource(id)}>
                      {sourceName(index.sources.get(id)!)}
                    </Button>
                  ))}
                </div>
              ) : null}
              <SourcePreview
                key={row.id}
                index={index}
                row={row}
                originals={originals}
                components={ui}
                customLoader={Boolean(loadText)}
              />
              <details>
                <summary>Record JSON</summary>
                <pre className="bh-snippet">{JSON.stringify(row.data, null, 2)}</pre>
              </details>
            </section>
          ) : null}
        </div>
      </div>
    </section>
  );
}

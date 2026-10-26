import type {
  Location,
  Manifest,
  ManifestDependency,
  ManifestFinding,
  ManifestGap,
  ManifestSource,
} from '../../src/types.js';

export type {
  Location,
  Manifest,
  ManifestDependency,
  ManifestFinding,
  ManifestGap,
  ManifestSource,
};
export type View = 'findings' | 'sources' | 'dependencies' | 'gaps';
export type Grouping = 'type' | 'source' | 'folder' | 'entry';
export type RecordData = ManifestFinding | ManifestSource | ManifestDependency | ManifestGap;
export interface RecordRow {
  id: string;
  index: number;
  view: View;
  source?: string;
  type: string;
  title: string;
  status: string;
  confidence: string;
  search: string;
  data: RecordData;
}
export interface Filters {
  view: View;
  query: string;
  severity: string;
  confidence: string;
  grouping: Grouping;
  group: string | null;
  includeDependencies: boolean;
}
export interface Group {
  id: string;
  name: string;
  description: string;
  count: number;
}

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);
const string = (value: unknown): value is string => typeof value === 'string';
const number = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0;
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(string);
const position = (value: unknown): boolean =>
  object(value) &&
  ['line', 'column', 'endLine', 'endColumn'].every((field) => number(value[field])) &&
  (value.original === undefined ||
    (object(value.original) &&
      string(value.original.source) &&
      number(value.original.line) &&
      number(value.original.column)));

/** Checks display inputs only. This viewer does not scan or execute source code. */
export function parseManifest(text: string): { manifest: Manifest; notices: string[] } {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error("This file is not valid JSON. Upload the scanner's manifest.json output.");
  }
  if (!object(value) || value.schemaVersion !== 2) {
    throw new Error(
      'This playground supports Manifest v2. Re-scan with the current SDK to view an older report.',
    );
  }
  if (!string(value.scannerVersion) || !string(value.status) || !string(value.dependencyMode)) {
    throw new Error('The manifest is missing its scanner version, status or dependency mode.');
  }
  if (!strings(value.entries) || !object(value.summary)) {
    throw new Error('The manifest must contain entries and a summary object.');
  }
  for (const field of [
    'sources',
    'findings',
    'errors',
    'warnings',
    'dependencies',
    'unresolved',
    'gaps',
    'bytes',
    'elapsedMs',
  ]) {
    if (!number(value.summary[field]))
      throw new Error(`summary.${field} must be a nonnegative number.`);
  }
  const checks: Record<View, (row: Record<string, unknown>) => boolean> = {
    sources: (r) =>
      string(r.id) &&
      string(r.kind) &&
      string(r.status) &&
      number(r.bytes) &&
      string(r.contentHash) &&
      strings(r.referrers) &&
      strings(r.chain) &&
      (r.url === undefined || string(r.url)) &&
      (r.inline === undefined ||
        (object(r.inline) &&
          string(r.inline.parent) &&
          number(r.inline.line) &&
          number(r.inline.column))),
    findings: (r) =>
      ['source', 'rule', 'severity', 'target', 'operation', 'confidence', 'snippet'].every((k) =>
        string(r[k]),
      ) &&
      position(r.location) &&
      (r.detail === undefined || string(r.detail)),
    dependencies: (r) =>
      ['from', 'kind', 'status'].every((k) => string(r[k])) &&
      position(r.location) &&
      ['to', 'specifier', 'url', 'reason'].every((k) => r[k] === undefined || string(r[k])),
    gaps: (r) =>
      string(r.reason) &&
      number(r.count) &&
      (r.location === undefined || position(r.location)) &&
      ['source', 'detail'].every((k) => r[k] === undefined || string(r[k])),
  };
  for (const [name, check] of Object.entries(checks)) {
    const rows = value[name];
    if (!Array.isArray(rows)) throw new Error(`The manifest is missing the ${name} array.`);
    const invalid = rows.findIndex((row) => !object(row) || !check(row));
    if (invalid !== -1) throw new Error(`${name}[${invalid}] has missing or invalid fields.`);
  }
  const manifest = value as unknown as Manifest;
  const ids = new Set(manifest.sources.map((source) => source.id));
  if (ids.size !== manifest.sources.length) throw new Error('Source IDs must be unique.');
  const references = [
    ...manifest.entries,
    ...manifest.findings.map((f) => f.source),
    ...manifest.dependencies.flatMap((d) => [d.from, ...(d.to ? [d.to] : [])]),
    ...manifest.gaps.flatMap((g) => (g.source ? [g.source] : [])),
    ...manifest.sources.flatMap((s) => [
      ...s.referrers,
      ...s.chain,
      ...(s.inline ? [s.inline.parent] : []),
    ]),
  ];
  const missing = references.find((id) => !ids.has(id));
  if (missing !== undefined)
    throw new Error(`The manifest references a missing source: ${missing}`);
  const notices: string[] = [];
  const actual = {
    sources: manifest.sources.length,
    findings: manifest.findings.length,
    dependencies: manifest.dependencies.length,
    gaps: manifest.gaps.reduce((sum, gap) => sum + gap.count, 0),
    errors: manifest.findings.filter((f) => f.severity === 'error').length,
    warnings: manifest.findings.filter((f) => f.severity === 'warning').length,
    unresolved: manifest.dependencies.filter((d) => d.status === 'unresolved').length,
  };
  if (
    Object.entries(actual).some(
      ([key, count]) => manifest.summary[key as keyof typeof actual] !== count,
    )
  ) {
    notices.push('Summary counts differ from the records. The explorer uses the actual records.');
  }
  return { manifest, notices };
}

export function sourceName(source: ManifestSource): string {
  if (source.inline) return `Inline ${source.kind} · ${source.id}`;
  if (!source.url) return `${source.kind.toUpperCase()} source · ${source.id}`;
  try {
    const url = new URL(source.url);
    const name = url.pathname.split('/').filter(Boolean).at(-1);
    return name ? decodeURIComponent(name) : url.host || source.url;
  } catch {
    return source.url.split('/').at(-1) || source.url;
  }
}

export function sourceFolder(source: ManifestSource, sources: Map<string, ManifestSource>): string {
  const visited = new Set<string>();
  let current = source;
  while (current.inline && !visited.has(current.id)) {
    visited.add(current.id);
    const parent = sources.get(current.inline.parent);
    if (!parent) break;
    current = parent;
  }
  if (!current.url) return 'Anonymous content';
  try {
    const url = new URL(current.url);
    return `${url.protocol}//${url.host}${url.pathname.slice(0, url.pathname.lastIndexOf('/') + 1)}`;
  } catch {
    return current.url.slice(0, current.url.lastIndexOf('/') + 1) || 'Other sources';
  }
}

export class ManifestIndex {
  readonly sources: Map<string, ManifestSource>;
  readonly rows: Record<View, RecordRow[]>;
  readonly byId = new Map<string, RecordRow>();
  readonly outgoing = new Map<string, ManifestDependency[]>();
  readonly incoming = new Map<string, ManifestDependency[]>();
  readonly children = new Map<string, Set<string>>();
  readonly parents = new Map<string, Set<string>>();
  readonly findingCounts = new Map<string, number>();
  readonly folders = new Map<string, string>();
  readonly entryReach = new Map<string, Set<string>>();
  readonly reached = new Set<string>();

  constructor(readonly manifest: Manifest) {
    this.sources = new Map(manifest.sources.map((source) => [source.id, source]));
    for (const source of manifest.sources) {
      this.folders.set(source.id, sourceFolder(source, this.sources));
      for (const parent of source.referrers) this.edge(parent, source.id);
      if (source.inline) this.edge(source.inline.parent, source.id);
    }
    for (const dependency of manifest.dependencies) {
      const outgoing = this.outgoing.get(dependency.from) ?? [];
      outgoing.push(dependency);
      this.outgoing.set(dependency.from, outgoing);
      if (dependency.to) {
        const incoming = this.incoming.get(dependency.to) ?? [];
        incoming.push(dependency);
        this.incoming.set(dependency.to, incoming);
        this.edge(dependency.from, dependency.to);
      }
    }
    for (const entry of manifest.entries) {
      const reach = this.descendants(entry);
      this.entryReach.set(entry, reach);
      for (const id of reach) this.reached.add(id);
    }
    for (const finding of manifest.findings) {
      this.findingCounts.set(finding.source, (this.findingCounts.get(finding.source) ?? 0) + 1);
    }
    this.rows = {
      findings: manifest.findings.map((f, i) =>
        this.row('findings', i, f, f.source, f.rule, f.target, f.severity, f.confidence),
      ),
      sources: manifest.sources.map((s, i) =>
        this.row('sources', i, s, s.id, s.kind, sourceName(s), s.status),
      ),
      dependencies: manifest.dependencies.map((d, i) =>
        this.row(
          'dependencies',
          i,
          d,
          d.from,
          d.kind,
          d.specifier || d.url || d.to || 'Unresolved reference',
          d.status,
        ),
      ),
      gaps: manifest.gaps.map((g, i) =>
        this.row('gaps', i, g, g.source, g.reason, g.reason, 'gap'),
      ),
    };
    for (const rows of Object.values(this.rows)) for (const row of rows) this.byId.set(row.id, row);
  }

  private row(
    view: View,
    index: number,
    data: RecordData,
    source: string | undefined,
    type: string,
    title: string,
    status: string,
    confidence = '',
  ): RecordRow {
    const owner = source ? this.sources.get(source) : undefined;
    const row = {
      id: `${view}:${index}`,
      index,
      view,
      source,
      type,
      title,
      status,
      confidence,
      data,
      search: '',
    };
    row.search = [
      JSON.stringify(data),
      owner?.url,
      owner?.id,
      owner ? sourceName(owner) : 'scan-wide',
    ]
      .join(' ')
      .toLowerCase();
    return row;
  }

  private edge(from: string, to: string) {
    const children = this.children.get(from) ?? new Set<string>();
    children.add(to);
    this.children.set(from, children);
    const parents = this.parents.get(to) ?? new Set<string>();
    parents.add(from);
    this.parents.set(to, parents);
  }

  descendants(id: string): Set<string> {
    const visited = new Set<string>();
    const pending = [id];
    for (let next = pending.pop(); next !== undefined; next = pending.pop()) {
      if (visited.has(next)) continue;
      visited.add(next);
      for (const child of this.children.get(next) ?? [])
        if (!visited.has(child)) pending.push(child);
    }
    return visited;
  }

  baseRows(filters: Filters): RecordRow[] {
    const words = filters.query.toLowerCase().trim().split(/\s+/).filter(Boolean);
    return this.rows[filters.view].filter(
      (row) =>
        words.every((word) => row.search.includes(word)) &&
        (!filters.severity || row.status === filters.severity) &&
        (!filters.confidence || row.confidence === filters.confidence),
    );
  }

  groupKeys(row: RecordRow, grouping: Grouping): string[] {
    if (grouping === 'type') return [row.type];
    if (grouping === 'source') return [row.source ?? 'scan-wide'];
    if (grouping === 'folder') return [row.source ? this.folders.get(row.source)! : 'Scan-wide'];
    if (!row.source) return ['scan-wide'];
    const entries = [...this.entryReach]
      .filter(([, reach]) => reach.has(row.source!))
      .map(([id]) => id);
    return entries.length ? entries : ['unattributed'];
  }

  groups(rows: RecordRow[], grouping: Grouping): Group[] {
    const counts = new Map<string, number>();
    for (const row of rows)
      for (const key of this.groupKeys(row, grouping)) counts.set(key, (counts.get(key) ?? 0) + 1);
    return [...counts]
      .map(([id, count]) => {
        const source =
          grouping === 'source' || grouping === 'entry' ? this.sources.get(id) : undefined;
        return {
          id,
          count,
          name: source
            ? sourceName(source)
            : id === 'scan-wide'
              ? 'Scan-wide'
              : id === 'unattributed'
                ? 'Not linked to an entry'
                : id,
          description: source
            ? source.url || (source.inline ? `Inline in ${source.inline.parent}` : source.id)
            : '',
        };
      })
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
  }

  filter(rows: RecordRow[], filters: Filters): RecordRow[] {
    if (filters.group === null) return rows;
    if (filters.grouping === 'source' && filters.includeDependencies) {
      const reach = this.descendants(filters.group);
      return rows.filter((row) => row.source && reach.has(row.source));
    }
    return rows.filter((row) => this.groupKeys(row, filters.grouping).includes(filters.group!));
  }

  /** Convert inline fragment locations to the outermost available parent. */
  parentLocation(
    sourceId: string,
    location: Location,
  ): { source: ManifestSource; location: Location } | undefined {
    let source = this.sources.get(sourceId);
    let result = { ...location };
    const visited = new Set<string>();
    let moved = false;
    while (source?.inline && !visited.has(source.id)) {
      visited.add(source.id);
      const origin = source.inline;
      const parent = this.sources.get(origin.parent);
      if (!parent) break;
      // Column offsets apply only on the fragment's own first line.
      result = {
        ...result,
        column: result.column + (result.line === 1 ? origin.column : 0),
        endColumn: result.endColumn + (result.endLine === 1 ? origin.column : 0),
        line: origin.line + result.line - 1,
        endLine: origin.line + result.endLine - 1,
      };
      source = parent;
      moved = true;
    }
    return moved && source ? { source, location: result } : undefined;
  }
}

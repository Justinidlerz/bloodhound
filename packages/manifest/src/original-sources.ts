import { AnyMap, traceSegment, type TraceMap } from '@jridgewell/trace-mapping';
import type { SourceMapReference } from './types.js';
import type { Location, ManifestSource } from './manifest.js';
import { safeUrl } from './url.js';

export interface ProjectWorkspace {
  projectRoot: string;
  fileEndpoint: string;
}

export interface OriginalPreview {
  status: 'loading' | 'ready' | 'error';
  text?: string;
  path?: string;
  url?: string;
  location?: Location;
  origin?: 'project' | 'sourcemap' | 'remote';
  error?: string;
}

export interface OriginalView {
  open: boolean;
  radius: number;
  wrap: boolean;
}

const MAX_MAP_BYTES = 64 * 1024 * 1024;
const MAX_SOURCE_BYTES = 8 * 1024 * 1024;

export interface TextResource {
  text: string;
  /** Absolute resource URL after redirects, used to resolve relative source paths. */
  url: string;
}

export interface TextRequest {
  kind: 'source' | 'sourcemap';
  limit: number;
  /** Project-relative path supplied by the plugin, when available. */
  file?: string;
}

/** The host owns authentication and build-artifact routing. */
export type TextLoader = (url: string, request: TextRequest) => Promise<TextResource>;
export const defaultLoadText: TextLoader = (url, request) => fetchText(url, request.limit);

/** Fetch source text without executing it. */
export async function fetchText(
  url: string,
  limit = MAX_SOURCE_BYTES,
): Promise<{ text: string; url: string }> {
  const response = await fetch(url, {
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  if (Number(response.headers.get('content-length')) > limit)
    throw new Error('File exceeds preview size limit');
  const content = await response.blob();
  if (content.size > limit) throw new Error('File exceeds preview size limit');
  return { text: await content.text(), url: response.url };
}

export function projectFileUrl(
  workspace: ProjectWorkspace | undefined,
  path: string | undefined,
): string | undefined {
  if (!workspace || !path) return undefined;
  return `${workspace.fileEndpoint}?path=${encodeURIComponent(path)}`;
}

function fileBase(root: string, path: string): string {
  const normalized = root.replaceAll('\\', '/');
  const base = new URL('file:///');
  base.pathname = `${normalized.startsWith('/') ? '' : '/'}${normalized}/`;
  return new URL(path.replaceAll('\\', '/').split('/').map(encodeURIComponent).join('/'), base)
    .href;
}

/** Session-only links and lazy map caches. No source requests are made until a preview is opened. */
export class OriginalSources {
  readonly previews = new Map<string, OriginalPreview>();
  private readonly overrides = new Map<string, SourceMapReference | null>();
  private readonly maps = new Map<string, Promise<TraceMap>>();
  private readonly files = new Map<string, Promise<{ text: string; url: string }>>();
  private readonly pending = new Map<string, Promise<void>>();
  private revision = 0;

  constructor(
    readonly manifestUrl?: string,
    readonly workspace?: ProjectWorkspace,
    readonly loadText: TextLoader = defaultLoadText,
  ) {}

  reference(source: ManifestSource): SourceMapReference | undefined {
    return this.overrides.has(source.id)
      ? (this.overrides.get(source.id) ?? undefined)
      : source.sourceMap;
  }

  link(source: ManifestSource, input: string): void {
    const value = input.trim();
    if (!value) throw new Error('Enter a source map URL or a project-relative path.');
    const url = safeUrl(value);
    if (!url && (/^[a-z][\w+.-]*:/i.test(value) || value.startsWith('//'))) {
      throw new Error('Use an HTTP(S) URL or a project-relative file path.');
    }
    if (!url && !this.workspace && this.loadText === defaultLoadText)
      throw new Error('Local paths require a project workspace or a custom text loader.');
    this.overrides.set(source.id, url ? { url } : { url: value, file: value });
    this.clear();
  }

  unlink(source: ManifestSource): void {
    this.overrides.set(source.id, null);
    this.clear();
  }

  private clear(): void {
    this.revision++;
    this.previews.clear();
    this.maps.clear();
    this.files.clear();
    this.pending.clear();
  }

  private loadFile(url: string, file?: string): Promise<TextResource> {
    let pending = this.files.get(url);
    if (!pending) {
      pending = this.loadText(url, { kind: 'source', limit: MAX_SOURCE_BYTES, file });
      this.files.set(url, pending);
    }
    return pending;
  }

  private loadMap(reference: SourceMapReference): Promise<TraceMap> {
    const key = JSON.stringify([reference.file, reference.url]);
    let pending = this.maps.get(key);
    if (!pending) {
      pending = this.readMap(reference);
      this.maps.set(key, pending);
    }
    return pending;
  }

  private async readMap(reference: SourceMapReference): Promise<TraceMap> {
    const local = projectFileUrl(this.workspace, reference.file);
    const remote =
      safeUrl(reference.url) ??
      (this.manifestUrl ? safeUrl(new URL(reference.url, this.manifestUrl).href) : undefined);
    if (local) {
      try {
        const data = await this.loadText(local, {
          kind: 'sourcemap',
          limit: MAX_MAP_BYTES,
          file: reference.file,
        });
        return AnyMap(data.text, fileBase(this.workspace!.projectRoot, reference.file!));
      } catch (error) {
        if (!remote) throw error;
      }
    }
    if (!remote && this.loadText === defaultLoadText)
      throw new Error('Open this manifest with --project <directory> or link a source map URL.');
    const data = await this.loadText(remote ?? reference.url, {
      kind: 'sourcemap',
      limit: MAX_MAP_BYTES,
      file: reference.file,
    });
    return AnyMap(data.text, data.url);
  }

  async preview(
    id: string,
    source: ManifestSource,
    location: Location,
    retry = false,
  ): Promise<void> {
    const reference = this.reference(source);
    if (!reference) return;
    const active = this.pending.get(id);
    if (active) return active;
    if (retry) {
      this.previews.delete(id);
      this.maps.delete(JSON.stringify([reference.file, reference.url]));
      this.files.clear();
    }
    if (this.previews.has(id)) return;
    const pending = this.readPreview(id, reference, location);
    this.pending.set(id, pending);
    await pending;
    if (this.pending.get(id) === pending) this.pending.delete(id);
  }

  private async readPreview(
    id: string,
    reference: SourceMapReference,
    location: Location,
  ): Promise<void> {
    const revision = this.revision;
    this.previews.set(id, { status: 'loading' });
    try {
      const map = await this.loadMap(reference);
      const token = traceSegment(map, location.line - 1, location.column);
      if (!token || token.length === 1)
        throw new Error('No original source mapping exists at this recorded position.');
      const sourceIndex = token[1];
      const localPath =
        reference.sourcePaths !== undefined
          ? (reference.sourcePaths[sourceIndex] ?? undefined)
          : map.resolvedSources[sourceIndex]?.startsWith('file:')
            ? map.resolvedSources[sourceIndex]
            : undefined;
      const originalPath =
        localPath ?? map.sources[sourceIndex] ?? map.resolvedSources[sourceIndex];
      const at: Location = {
        line: token[2] + 1,
        column: token[3],
        endLine: token[2] + 1,
        endColumn: token[3],
      };
      let result: OriginalPreview | undefined;
      const local =
        projectFileUrl(this.workspace, localPath) ??
        (this.loadText !== defaultLoadText ? localPath : undefined);
      if (local) {
        const data = await this.loadFile(local, localPath).catch(() => undefined);
        if (data) result = { status: 'ready', text: data.text, origin: 'project' };
      }
      const embedded = map.sourcesContent?.[sourceIndex];
      if (!result && typeof embedded === 'string') {
        if (new TextEncoder().encode(embedded).length > MAX_SOURCE_BYTES)
          throw new Error('Source exceeds preview size limit');
        result = { status: 'ready', text: embedded, origin: 'sourcemap' };
      }
      // Plugin maps provide project paths explicitly. Do not interpret their relative sources as CDN URLs.
      const remote =
        reference.sourcePaths === undefined ||
        safeUrl(map.sources[sourceIndex] ?? '') ||
        safeUrl(map.sourceRoot)
          ? safeUrl(map.resolvedSources[sourceIndex])
          : undefined;
      if (!result && remote) {
        const data = await this.loadFile(remote);
        result = { status: 'ready', text: data.text, origin: 'remote', url: data.url };
      }
      if (!result)
        throw new Error(
          'The map has no embedded source content and no readable source file is associated.',
        );
      if (revision === this.revision)
        this.previews.set(id, {
          ...result,
          path: originalPath ?? 'Original source',
          location: at,
        });
    } catch (error) {
      if (revision === this.revision)
        this.previews.set(id, {
          status: 'error',
          error: error instanceof Error ? error.message : 'Could not load source',
        });
    }
  }
}

import { dirname, isAbsolute, resolve as resolvePath } from 'node:path';
import { scan } from '@bloodhound-rs/core';
import type {
  Manifest,
  Location,
  Rule,
  ResourceKind,
  ScanEntry,
  ScanOptions,
  ScriptMode,
  Source,
} from '@bloodhound-rs/core';

/*
 * The plugin only relies on the subset of the plugin API that Rollup, Rolldown, and Vite share.
 * The structural types below describe that subset so the plugin type-checks against any of them
 * without importing bundler-specific declarations.
 */

/** Emitted chunk as seen in `generateBundle`. Matches Rollup and Rolldown `OutputChunk`. */
export interface BundleChunk {
  type: 'chunk';
  fileName: string;
  code: string;
  isEntry: boolean;
  map?: { toString(): string } | null;
  sourcemapFileName?: string | null;
}

/** Emitted asset as seen in `generateBundle`. Matches Rollup and Rolldown `OutputAsset`. */
export interface BundleAsset {
  type: 'asset';
  fileName: string;
  source: string | Uint8Array;
}

export type Bundle = Record<string, BundleChunk | BundleAsset>;

/** Output options fields read by the plugin. Matches normalized Rollup and Rolldown options. */
export interface BundleOutputOptions {
  format: string;
  dir?: string | undefined;
  file?: string | undefined;
  sourcemapFile?: string | undefined;
}

export interface LogLocation {
  file?: string;
  line: number;
  column: number;
}

export interface PluginLog {
  message: string;
  loc?: LogLocation;
  meta?: Record<string, unknown>;
}

/** Plugin context methods used by the plugin. Satisfied by Rollup, Rolldown, and Vite contexts. */
export interface BundlerContext {
  warn(log: PluginLog | string): void;
  error(error: PluginLog | string): never;
  emitFile(file: { type: 'asset'; fileName: string; source: string }): string;
}

export interface BloodhoundPlugin {
  name: string;
  /** Vite-only: run during `vite build`. Rollup and Rolldown ignore it. */
  apply: 'build';
  /** Vite-only: run after Vite's own output plugins. Rollup and Rolldown ignore it. */
  enforce: 'post';
  generateBundle: {
    order: 'post';
    handler(
      this: BundlerContext,
      outputOptions: BundleOutputOptions,
      bundle: Bundle,
    ): Promise<void>;
  };
}

export type Severity = 'off' | 'warn' | 'error';

/** One reportable item derived from a manifest, with bundle-relative context attached. */
export interface Diagnostic {
  kind: 'global' | 'finding';
  /** Manifest-local source id such as `s1`. */
  sourceId: string;
  /** Emitted file that contains the location, when the source maps to a bundle file. */
  fileName?: string;
  rule: Rule;
  /** Dotted global target paths such as `sharedConfig.version`. */
  targets: string[];
  message: string;
  location: Location;
  /** Original source location when a source map was available. */
  original?: { source: string; line: number; column: number; name?: string };
  detail?: string;
}

export interface BloodhoundPluginOptions {
  /**
   * Base URL used to address emitted files during the scan. Relative values such as `/assets/`
   * are resolved against `https://bundle.local/`. Use the same value as Vite `base` or the
   * production CDN prefix so that script references inside emitted HTML resolve to bundle files.
   */
  publicPath?: string;
  /** Select which emitted files are scan entries. Default: HTML assets, or entry chunks when no HTML is emitted. */
  entries?: (fileName: string, item: BundleChunk | BundleAsset) => boolean;
  /** Script mode for JavaScript entries. Default: derived from the output format. */
  scriptMode?: ScriptMode;
  /** Additional scanner options. In-memory bundle mappings are managed by the plugin. */
  scanOptions?: Omit<ScanOptions, 'resourceMappings'>;
  /** Manifest asset emitted next to the bundle. `false` disables it. Default: `bloodhound.manifest.json`. */
  manifestFileName?: string | false;
  /** Severity for global declarations, writes, definitions, and deletes. Default: `warn`. */
  globals?: Severity;
  /** Severity for DOM and CSS findings. Default: `warn`. */
  findings?: Severity;
  /** Per-rule overrides, applied before the globals/findings category defaults. */
  rules?: Partial<Record<Rule, Severity>>;
  /** Severity when the scan is partial or failed. Default: `warn`. */
  coverage?: Severity;
  /** Global target paths that are expected. Supports exact paths, `prefix.*`, `*` wildcards, and RegExp. */
  allowGlobals?: (string | RegExp)[];
  /** Original source paths to ignore when a source map resolved the location. Default: `[/node_modules/]`. */
  ignoreSources?: (string | RegExp)[];
  /** Final predicate applied after the built-in filters. Return `false` to drop a diagnostic. */
  filter?: (diagnostic: Diagnostic) => boolean;
  /** Receives the manifest and the derived diagnostics after each scan. */
  onManifest?: (
    manifest: Manifest,
    context: { diagnostics: Diagnostic[]; bundle: Bundle; outputOptions: BundleOutputOptions },
  ) => void | Promise<void>;
}

const DEFAULT_BASE = 'https://bundle.local/';
const SCHEME = /^[a-z][a-z0-9+.-]*:/i;

const kindOf = (fileName: string): ResourceKind | 'asset' => {
  const name = fileName.toLowerCase();
  if (/\.(m|c)?js$/.test(name)) return 'js';
  if (/\.html?$/.test(name)) return 'html';
  if (name.endsWith('.css')) return 'css';
  return 'asset';
};

const decode = (value: string | Uint8Array): string =>
  typeof value === 'string' ? value : new TextDecoder('utf-8', { fatal: false }).decode(value);

function baseUrl(publicPath: string | undefined): string {
  const raw = publicPath && publicPath.length > 0 ? publicPath : DEFAULT_BASE;
  const url = SCHEME.test(raw) ? new URL(raw) : new URL(raw, DEFAULT_BASE);
  if (!url.pathname.endsWith('/')) url.pathname += '/';
  return url.href;
}

/** Rollup reports `es`; Rolldown and Vite configs may also spell it `esm` or `module`. */
const scriptModeOf = (format: string): ScriptMode =>
  format === 'es' || format === 'esm' || format === 'module' ? 'module' : 'classic';

function toMatcher(pattern: string | RegExp): (value: string) => boolean {
  if (pattern instanceof RegExp)
    return (value) => {
      pattern.lastIndex = 0;
      return pattern.test(value);
    };
  if (!pattern.includes('*')) return (value) => value === pattern;
  const escaped = pattern.split('*').map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const regex = new RegExp(`^${escaped.join('.*')}$`);
  return (value) => regex.test(value);
}

/**
 * Convert a manifest into diagnostics and apply allow and ignore filters. Exported so callers can
 * reuse the plugin's filtering outside a bundler, for example when post-processing a saved manifest.
 * `fileNames` maps manifest source ids to emitted file names.
 */
export function diagnosticsFromManifest(
  manifest: Manifest,
  options: Pick<BloodhoundPluginOptions, 'allowGlobals' | 'ignoreSources' | 'filter'> = {},
  fileNames: Map<string, string> = new Map(),
): Diagnostic[] {
  const allow = (options.allowGlobals ?? []).map(toMatcher);
  const ignore = (options.ignoreSources ?? [/node_modules/]).map(toMatcher);
  const result: Diagnostic[] = [];
  const sources = new Map(manifest.sources.map((source) => [source.id, source]));
  for (const item of manifest.findings) {
    const kind = [
      'global-write',
      'builtin-mutation',
      'host-api-override',
      'document-write',
    ].includes(item.rule)
      ? 'global'
      : 'finding';
    // Accept both canonical paths and the previous shorthand without window.
    const target = item.target.replace(/^window\./, '');
    if (kind === 'global' && allow.some((match) => match(item.target) || match(target))) continue;
    let location = { ...item.location };
    let source = sources.get(item.source);
    const visited = new Set<string>();
    while (source?.inline && !visited.has(source.id)) {
      visited.add(source.id);
      const origin = source.inline;
      location = {
        ...location,
        line: origin.line + location.line - 1,
        column: location.column + (location.line === 1 ? origin.column : 0),
        endLine: origin.line + location.endLine - 1,
        endColumn: location.endColumn + (location.endLine === 1 ? origin.column : 0),
      };
      source = sources.get(origin.parent);
    }
    result.push({
      kind,
      sourceId: item.source,
      fileName: fileNames.get(item.source),
      rule: item.rule,
      targets: [item.target],
      message: `${item.rule}: ${item.target} (${item.operation})`,
      location,
      original: item.location.original,
      detail: item.detail,
    });
  }
  return result.filter(
    (diagnostic) =>
      !(diagnostic.original && ignore.some((match) => match(diagnostic.original!.source))) &&
      (options.filter ? options.filter(diagnostic) : true),
  );
}

interface Prepared {
  entries: ScanEntry[];
  resourceMappings: Record<string, Source>;
  /** Resolved resource URL to emitted file name. */
  urls: Map<string, string>;
}

function prepare(bundle: Bundle, base: string, options: BloodhoundPluginOptions): Prepared {
  const resourceMappings: Record<string, Source> = {};
  const urls = new Map<string, string>();
  const candidates: { fileName: string; item: BundleChunk | BundleAsset; kind: ResourceKind }[] =
    [];
  for (const [fileName, item] of Object.entries(bundle)) {
    if (fileName === options.manifestFileName) continue;
    const url = new URL(fileName, base).href;
    if (item.type === 'chunk') {
      let content = item.code;
      if (item.map) {
        // Use bundler metadata; an inline/hidden map can also use a virtual
        // identity without assuming any emitted map filename.
        const mapUrl = item.sourcemapFileName
          ? new URL(item.sourcemapFileName, base).href
          : `${url}#bloodhound-source-map`;
        resourceMappings[mapUrl] = {
          type: 'content',
          content: item.map.toString(),
          resourceUrl: mapUrl,
        };
        // The final directive refers to the authoritative in-memory map.
        // Only the scanned copy changes; emitted code and maps stay intact.
        content += `\n//# sourceMappingURL=${mapUrl}\n`;
      }
      resourceMappings[url] = { type: 'content', content, resourceUrl: url };
      urls.set(url, fileName);
      candidates.push({ fileName, item, kind: 'js' });
      continue;
    }
    const kind = kindOf(fileName);
    if (kind === 'asset') {
      if (fileName.endsWith('.map')) {
        resourceMappings[url] = { type: 'content', content: decode(item.source), resourceUrl: url };
      }
      continue;
    }
    resourceMappings[url] = { type: 'content', content: decode(item.source), resourceUrl: url };
    urls.set(url, fileName);
    candidates.push({ fileName, item, kind });
  }
  const selected = options.entries
    ? candidates.filter(({ fileName, item }) => options.entries!(fileName, item))
    : candidates.some(({ kind }) => kind === 'html')
      ? candidates.filter(({ kind }) => kind === 'html')
      : candidates.filter(({ item }) => item.type === 'chunk' && item.isEntry);
  const entries: ScanEntry[] = selected.map(({ fileName, kind }) => {
    const url = new URL(fileName, base).href;
    return { kind, source: resourceMappings[url]!, resourceUrl: url };
  });
  return { entries, resourceMappings, urls };
}

/** Map manifest source ids to emitted file names; inline sources inherit their parent's file. */
function fileNamesOf(manifest: Manifest, urls: Map<string, string>): Map<string, string> {
  const fileNames = new Map<string, string>();
  const pending = [...manifest.sources];
  // Parents normally precede inline children, but resolve iteratively to be safe.
  let progress = true;
  while (pending.length > 0 && progress) {
    progress = false;
    for (let i = pending.length - 1; i >= 0; i--) {
      const source = pending[i]!;
      const url = source.url;
      const fileName =
        (url && urls.get(url)) ||
        (source.inline?.parent ? fileNames.get(source.inline.parent) : undefined);
      if (fileName) {
        fileNames.set(source.id, fileName);
        pending.splice(i, 1);
        progress = true;
      }
    }
  }
  return fileNames;
}

function locate(diagnostic: Diagnostic, outputOptions: BundleOutputOptions): LogLocation {
  if (diagnostic.original) {
    const { source, line, column } = diagnostic.original;
    const outDir = outputOptions.dir ?? (outputOptions.file ? dirname(outputOptions.file) : '');
    const file =
      isAbsolute(source) || SCHEME.test(source) || !diagnostic.fileName
        ? source
        : outputOptions.sourcemapFile
          ? resolvePath(dirname(outputOptions.sourcemapFile), source)
          : resolvePath(outDir, dirname(diagnostic.fileName), source);
    return { file, line, column };
  }
  // Inline scripts and styles report their position inside the containing document.
  const position = diagnostic.location;
  return { file: diagnostic.fileName, line: position.line, column: position.column };
}

function report(
  context: BundlerContext,
  diagnostics: Diagnostic[],
  outputOptions: BundleOutputOptions,
  severity: { globals: Severity; findings: Severity },
  rules: BloodhoundPluginOptions['rules'],
): string[] {
  const failures: string[] = [];
  for (const diagnostic of diagnostics) {
    const level =
      rules?.[diagnostic.rule] ??
      (diagnostic.kind === 'global' ? severity.globals : severity.findings);
    if (level === 'off') continue;
    const loc = locate(diagnostic, outputOptions);
    const where = `${loc.file ?? diagnostic.sourceId}:${loc.line}:${loc.column + 1}`;
    const log: PluginLog = { message: diagnostic.message, meta: { bloodhound: diagnostic } };
    if (loc.file) log.loc = { file: loc.file, line: loc.line, column: loc.column };
    else log.message += ` at ${where}`;
    context.warn(log);
    if (level === 'error') failures.push(`${diagnostic.message} at ${where}`);
  }
  return failures;
}

export function bloodhound(options: BloodhoundPluginOptions = {}): BloodhoundPlugin {
  const manifestFileName = options.manifestFileName ?? 'bloodhound.manifest.json';
  const severity = {
    globals: options.globals ?? 'warn',
    findings: options.findings ?? 'warn',
    coverage: options.coverage ?? 'warn',
  };
  return {
    name: 'bloodhound',
    apply: 'build',
    enforce: 'post',
    generateBundle: {
      // Run after other plugins so HTML and assets they emit are part of the scan.
      order: 'post',
      async handler(outputOptions, bundle) {
        const base = baseUrl(options.publicPath);
        const { entries, resourceMappings, urls } = prepare(bundle, base, {
          ...options,
          manifestFileName: manifestFileName || undefined,
        });
        if (entries.length === 0) {
          this.warn('bloodhound: no scan entries were found in the emitted bundle');
          return;
        }
        const scanOptions = {
          sourceMaps: true,
          // Offline by default: every bundle file is supplied through resourceMappings, and
          // references outside the bundle are reported as gaps instead of fetched.
          access: { protocols: [] },
          ...options.scanOptions,
          scriptMode: options.scriptMode ?? scriptModeOf(outputOptions.format),
          resourceMappings,
        } satisfies ScanOptions;
        const manifest: Manifest = await scan(entries, scanOptions);
        const diagnostics = diagnosticsFromManifest(manifest, options, fileNamesOf(manifest, urls));
        if (manifestFileName) {
          this.emitFile({
            type: 'asset',
            fileName: manifestFileName,
            source: JSON.stringify(manifest, null, 2) + '\n',
          });
        }
        await options.onManifest?.(manifest, { diagnostics, bundle, outputOptions });
        const failures = report(this, diagnostics, outputOptions, severity, options.rules);
        if (manifest.status !== 'completed' && severity.coverage !== 'off') {
          const gaps = manifest.gaps
            .slice(0, 5)
            .map((gap) => `${gap.reason} x${gap.count}`)
            .join(', ');
          const message = `bloodhound: scan ${manifest.status} with ${manifest.summary.unresolved} unresolved dependencies and ${manifest.summary.gaps} coverage gaps${gaps ? `: ${gaps}` : ''}`;
          if (severity.coverage === 'error') failures.push(message);
          else this.warn(message);
        }
        if (failures.length > 0) {
          this.error(
            `bloodhound: ${failures.length} blocking issue(s) found in the emitted bundle\n` +
              failures.map((line) => `  - ${line}`).join('\n'),
          );
        }
      },
    },
  };
}

export default bloodhound;

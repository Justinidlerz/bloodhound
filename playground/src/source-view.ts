import { safeUrl } from './format.js';
import type { Location, ManifestIndex, ManifestSource, RecordRow } from './manifest.js';

/** State of one on-demand source fetch, keyed by URL in the app. */
export interface SourceText {
  status: 'loading' | 'ready' | 'error';
  text?: string;
  error?: string;
}

/** What the embedded viewer shows for the selected record. */
export interface Viewer {
  /** Display source id the viewer was opened for (parent id for inline code). */
  sourceId: string;
  url: string;
  /** Lines of context kept either side of the highlighted range. */
  radius: number;
  wrap: boolean;
}

/** The fetchable file and highlighted range for a record, when one exists. */
export interface ViewerTarget {
  source: ManifestSource;
  url: string;
  location?: Location;
}

/**
 * Inline fragments are viewed in their parent file at the translated
 * position. Records without an HTTP(S) file cannot be fetched and keep the
 * manifest snippet as their only excerpt.
 */
export function viewerTarget(index: ManifestIndex, row: RecordRow): ViewerTarget | undefined {
  const source = row.source ? index.sources.get(row.source) : undefined;
  if (!source) return undefined;
  const location = 'location' in row.data ? row.data.location : undefined;
  if (location) {
    const parent = index.parentLocation(source.id, location);
    if (parent) {
      const url = safeUrl(parent.source.url);
      return url ? { source: parent.source, url, location: parent.location } : undefined;
    }
  }
  const url = safeUrl(source.url);
  if (url) return { source, url, location };
  if (source.inline) {
    const parent = index.sources.get(source.inline.parent);
    const parentUrl = parent && safeUrl(parent.url);
    if (parent && parentUrl) {
      const at = {
        line: source.inline.line,
        column: source.inline.column,
        endLine: source.inline.line,
        endColumn: source.inline.column,
      };
      return { source: parent, url: parentUrl, location: at };
    }
  }
  return undefined;
}

export const DEFAULT_RADIUS = 25;
/** Longest rendered line segment. Minified bundles often have one huge line. */
const MAX_LINE_CHARS = 2000;

export interface ViewLine {
  /** 1-based line number in the fetched text. */
  number: number;
  before: string;
  /** Highlighted text; empty when this line has no highlighted part. */
  hit: string;
  /** The line falls inside the highlighted range (hit can be zero-width). */
  inRange: boolean;
  after: string;
  /** Text exists before/after the rendered segment of this line. */
  cutBefore: boolean;
  cutAfter: boolean;
}

export interface CodeWindow {
  lines: ViewLine[];
  total: number;
  /** 1-based first and last rendered line numbers. */
  from: number;
  to: number;
}

function viewLine(number: number, text: string, location: Location | undefined): ViewLine {
  const line = text.endsWith('\r') ? text.slice(0, -1) : text;
  let hitStart = -1;
  let hitEnd = -1;
  if (location && number >= location.line && number <= location.endLine) {
    hitStart = number === location.line ? location.column : 0;
    hitEnd = number === location.endLine ? location.endColumn : line.length;
    hitStart = Math.min(hitStart, line.length);
    hitEnd = Math.max(hitStart, Math.min(hitEnd, line.length));
  }
  // Keep the rendered segment bounded, centered on the hit when there is one.
  let start = 0;
  if (line.length > MAX_LINE_CHARS) {
    const center = hitStart >= 0 ? hitStart : 0;
    start = Math.max(0, Math.min(center - MAX_LINE_CHARS / 2, line.length - MAX_LINE_CHARS));
  }
  const segment = line.slice(start, start + MAX_LINE_CHARS);
  const localStart = hitStart - start;
  const localEnd = hitEnd - start;
  const hasHit = hitStart >= 0 && localEnd > 0 && localStart < segment.length;
  return {
    number,
    inRange: hitStart >= 0,
    before: hasHit ? segment.slice(0, Math.max(0, localStart)) : segment,
    hit: hasHit ? segment.slice(Math.max(0, localStart), Math.min(segment.length, localEnd)) : '',
    after: hasHit ? segment.slice(Math.min(segment.length, localEnd)) : '',
    cutBefore: start > 0,
    cutAfter: start + segment.length < line.length,
  };
}

/** The window of lines rendered around `location` (or from the top). */
export function codeWindow(
  text: string,
  location: Location | undefined,
  radius: number,
): CodeWindow {
  const all = text.split('\n');
  const total = all.length;
  const from = location ? Math.max(1, location.line - radius) : 1;
  const to = location
    ? Math.min(total, location.endLine + radius)
    : Math.min(total, radius * 2 + 1);
  const lines: ViewLine[] = [];
  for (let n = from; n <= to; n++) lines.push(viewLine(n, all[n - 1] ?? '', location));
  return { lines, total, from, to };
}

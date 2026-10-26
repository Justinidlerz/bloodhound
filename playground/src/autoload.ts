/**
 * Resolution of the `?manifest=` startup parameter.
 *
 * A browser page can fetch an HTTP(S) URL (subject to CORS) or a same-origin
 * path, but it cannot read a local disk file. Local files therefore go
 * through the development server: `pnpm playground --manifest <path>`
 * serves the file at `/__manifest`, and the printed startup URL points the
 * `manifest` parameter at that endpoint.
 */

export type AutoloadTarget =
  | { kind: 'fetch'; url: string; name: string }
  | { kind: 'file' }
  | { kind: 'invalid'; reason: string };

function fileNameOf(url: URL, nameParam: string | null | undefined): string {
  const raw = nameParam || url.pathname.split('/').filter(Boolean).at(-1) || '';
  const base = raw.split(/[\\/]/).filter(Boolean).at(-1) ?? '';
  return base && base.length <= 200 ? base : 'manifest.json';
}

/**
 * Maps a raw `manifest` parameter value to a fetchable URL. `pageHref` is
 * the playground's own URL, used to resolve relative values and to require
 * that non-absolute values stay on the same origin.
 */
export function resolveManifestParam(
  value: string,
  pageHref: string,
  nameParam?: string | null,
): AutoloadTarget {
  const trimmed = value.trim();
  if (!trimmed) return { kind: 'invalid', reason: 'empty manifest parameter' };
  let page: URL;
  try {
    page = new URL(pageHref);
  } catch {
    return { kind: 'invalid', reason: 'unparsable page URL' };
  }
  let url: URL;
  try {
    url = new URL(trimmed, page);
  } catch {
    return { kind: 'invalid', reason: 'unparsable manifest URL' };
  }
  if (url.protocol === 'file:') return { kind: 'file' };
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return { kind: 'invalid', reason: `unsupported protocol ${url.protocol}` };
  }
  if (url.username || url.password) {
    return { kind: 'invalid', reason: 'credentials in the URL are not allowed' };
  }
  return { kind: 'fetch', url: url.href, name: fileNameOf(url, nameParam) };
}

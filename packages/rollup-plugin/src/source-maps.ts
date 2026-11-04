import { createHash } from 'node:crypto';
import { dirname, isAbsolute, posix, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { AnyMap } from '@jridgewell/trace-mapping';
import type { Manifest } from '@bloodhound-rs/core';
import type { Bundle, BundleOutputOptions, BundlerContext } from './index.js';

function projectPath(root: string, file: string): string | undefined {
  const path = relative(root, file);
  return path === '..' || path.startsWith(`..${sep}`) || isAbsolute(path)
    ? undefined
    : path.split(sep).join('/');
}

/** Keep map identity and local paths alongside the resource graph without changing scan results. */
export function attachSourceMaps(
  manifest: Manifest,
  bundle: Bundle,
  output: BundleOutputOptions,
  fileNames: Map<string, string>,
  root: string,
  manifestFileName: string,
  context: BundlerContext,
): void {
  const outDir = resolve(output.dir ?? (output.file ? dirname(output.file) : '.'));
  const emittedMaps = new Set<string>();
  for (const source of manifest.sources) {
    // Inline positions belong to the containing document, not to its bundle's map.
    if (source.inline) continue;
    const name = fileNames.get(source.id);
    if (!name) continue;
    const file = projectPath(root, resolve(outDir, name));
    if (file) source.file = file;
    const chunk = bundle[name];
    if (chunk?.type !== 'chunk' || !chunk.map) continue;
    const text = chunk.map.toString();
    // Rollup/Rolldown resolve source paths from the output chunk, even with a custom map folder.
    const mapBase = output.file
      ? resolve(output.sourcemapFile ?? output.file)
      : resolve(outDir, name);
    const map = AnyMap(text, pathToFileURL(mapBase).href);
    const existingMap =
      chunk.sourcemapFileName && bundle[chunk.sourcemapFileName]?.type === 'asset'
        ? chunk.sourcemapFileName
        : undefined;
    const mapName =
      existingMap ?? `bloodhound/sourcemaps/${createHash('sha256').update(text).digest('hex')}.map`;
    // Inline maps need a companion file for the previewer.
    if (!existingMap && !emittedMaps.has(mapName)) {
      context.emitFile({ type: 'asset', fileName: mapName, source: text });
      emittedMaps.add(mapName);
    }
    const mapFile = projectPath(root, resolve(outDir, mapName));
    source.sourceMap = {
      url: posix
        .relative(posix.dirname(manifestFileName), mapName)
        .split('/')
        .map(encodeURIComponent)
        .join('/'),
      ...(mapFile ? { file: mapFile } : {}),
      sourcePaths: map.resolvedSources.map((url) =>
        url.startsWith('file:') ? (projectPath(root, fileURLToPath(url)) ?? null) : null,
      ),
    };
  }
}

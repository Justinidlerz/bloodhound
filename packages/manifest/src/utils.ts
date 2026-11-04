import type { Location, Manifest, ManifestDependency, ManifestFinding } from './types.js';

/**
 * Remove records and resources owned by source paths matching any exclusion regex.
 * String patterns use JavaScript RegExp syntax. Paths use forward slashes.
 * The input and caller-owned regexes are not mutated. Scan status and costs are retained.
 */
export function filterManifestBySourcePaths(
  manifest: Manifest,
  excludePatterns: readonly (string | RegExp)[],
): Manifest {
  const patterns = excludePatterns.map((pattern) =>
    typeof pattern === 'string' ? new RegExp(pattern) : new RegExp(pattern.source, pattern.flags),
  );
  const matches = (path: string | null | undefined): boolean => {
    if (path == null) return false;
    const normalized = path.replaceAll('\\', '/');
    return patterns.some((pattern) => {
      pattern.lastIndex = 0;
      return pattern.test(normalized);
    });
  };
  const ignoresLocation = (location: Location | undefined) => matches(location?.original?.source);

  const removed = new Set<string>();
  const inlineChildren = new Map<string, string[]>();
  for (const source of manifest.sources) {
    const paths = source.sourceMap?.sourcePaths;
    if (
      matches(source.file) ||
      matches(source.url) ||
      (paths?.length && paths.every((path) => path !== null && matches(path)))
    ) {
      removed.add(source.id);
    }
    if (source.inline) {
      const children = inlineChildren.get(source.inline.parent) ?? [];
      children.push(source.id);
      inlineChildren.set(source.inline.parent, children);
    }
  }
  const pending = [...removed];
  for (const id of pending) {
    for (const child of inlineChildren.get(id) ?? []) {
      if (!removed.has(child)) {
        removed.add(child);
        pending.push(child);
      }
    }
  }

  const findings: ManifestFinding[] = [];
  for (const finding of manifest.findings) {
    if (removed.has(finding.source)) continue;
    let filtered = finding;
    if (finding.writes?.length) {
      const writes = finding.writes.filter((write) => !ignoresLocation(write.location));
      const first = writes[0];
      if (!first) continue;
      if (writes.length !== finding.writes.length) {
        filtered = {
          ...finding,
          ...first,
          detail: first.detail,
          polyfill: first.polyfill,
          confidence: writes.some((write) => write.confidence === 'medium') ? 'medium' : 'high',
          writes,
        };
      }
    } else if (ignoresLocation(finding.location)) {
      continue;
    }
    if (ignoresLocation(filtered.declaration?.location)) {
      const { declaration, ...rest } = filtered;
      filtered = rest;
    }
    findings.push(filtered);
  }

  const dependencies = manifest.dependencies.filter(
    (dependency) =>
      !removed.has(dependency.from) &&
      !(dependency.to && removed.has(dependency.to)) &&
      !matches(dependency.url) &&
      !matches(dependency.specifier) &&
      !ignoresLocation(dependency.location),
  );
  const gaps = manifest.gaps.filter(
    (gap) =>
      !(gap.source && removed.has(gap.source)) &&
      // A grouped gap only stores its first location, not every occurrence's source path.
      !(gap.count === 1 && ignoresLocation(gap.location)),
  );
  const entries = manifest.entries.filter((id) => !removed.has(id));

  const referrers = (edges: ManifestDependency[]) => {
    const result = new Map<string, Set<string>>();
    for (const edge of edges) {
      if (!edge.to) continue;
      const parents = result.get(edge.to) ?? new Set<string>();
      parents.add(edge.from);
      result.set(edge.to, parents);
    }
    return result;
  };
  const originalReferrers = referrers(manifest.dependencies);
  const retainedReferrers = referrers(dependencies);
  const sources = manifest.sources
    .filter((source) => !removed.has(source.id))
    .map((source) => {
      const parents = new Set(
        source.referrers.filter(
          (id) =>
            !removed.has(id) &&
            (!originalReferrers.get(source.id)?.has(id) ||
              retainedReferrers.get(source.id)?.has(id)),
        ),
      );
      for (const id of retainedReferrers.get(source.id) ?? []) parents.add(id);
      if (source.inline) parents.add(source.inline.parent);
      return {
        ...source,
        referrers: [...parents],
        chain: [source.id],
        ...(source.sourceMap && {
          sourceMap: {
            ...source.sourceMap,
            ...(source.sourceMap.sourcePaths && {
              // Keep sourcemap source indexes aligned for previews of retained records.
              sourcePaths: source.sourceMap.sourcePaths.map((path) =>
                matches(path) ? null : path,
              ),
            }),
          },
        }),
      };
    });

  const byId = new Map(sources.map((source) => [source.id, source]));
  const children = new Map<string, string[]>();
  for (const source of sources) {
    for (const id of source.referrers) {
      const next = children.get(id) ?? [];
      next.push(source.id);
      children.set(id, next);
    }
  }
  const visited = new Set(entries);
  const queue = [...visited];
  for (const id of queue) {
    const parent = byId.get(id)!;
    for (const childId of children.get(id) ?? []) {
      if (visited.has(childId)) continue;
      visited.add(childId);
      byId.get(childId)!.chain = [...parent.chain, childId];
      queue.push(childId);
    }
  }

  return {
    ...manifest,
    entries,
    sources,
    findings,
    dependencies,
    gaps,
    summary: {
      ...manifest.summary,
      sources: sources.length,
      findings: findings.length,
      errors: findings.filter((finding) => finding.severity === 'error').length,
      warnings: findings.filter((finding) => finding.severity === 'warning').length,
      dependencies: dependencies.length,
      unresolved: dependencies.filter((dependency) => dependency.status === 'unresolved').length,
      gaps: gaps.reduce((sum, gap) => sum + gap.count, 0),
    },
  };
}

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

export const releasePlanPath = 'artifacts/release-plan.json';

/** Read the workspace configured in pnpm-workspace.yaml, excluding private packages. */
export function workspacePackages() {
  const listed = JSON.parse(
    execFileSync('pnpm', ['list', '--recursive', '--depth', '-1', '--json'], {
      encoding: 'utf8',
    }),
  );
  return listed
    .map(({ path }) => {
      const directory = relative(process.cwd(), path).split(sep).join('/') || '.';
      if (directory === '..' || directory.startsWith('../')) {
        throw new Error(`Workspace package outside repository: ${directory}`);
      }
      const manifest = JSON.parse(readFileSync(join(path, 'package.json'), 'utf8'));
      return { directory, manifest };
    })
    .filter(({ manifest }) => !manifest.private);
}

/** Publish runtime dependencies before their consumers; dev dependencies do not impose order. */
export function dependencyOrder(packages) {
  const byName = new Map(packages.map((entry) => [entry.manifest.name, entry]));
  const visiting = new Set();
  const visited = new Set();
  const ordered = [];
  function visit(entry) {
    const { name, dependencies, optionalDependencies } = entry.manifest;
    if (visited.has(name)) return;
    if (visiting.has(name)) throw new Error(`Circular runtime workspace dependency: ${name}`);
    visiting.add(name);
    for (const name of Object.keys({ ...dependencies, ...optionalDependencies })) {
      const dependency = byName.get(name);
      if (dependency) visit(dependency);
    }
    visiting.delete(name);
    visited.add(name);
    ordered.push(entry);
  }
  for (const entry of packages) visit(entry);
  return ordered;
}

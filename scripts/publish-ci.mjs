import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout } from 'node:timers/promises';

const directory = dirname(fileURLToPath(import.meta.url));
const { registry, packages } = JSON.parse(
  readFileSync(resolve(directory, 'release-manifest.json'), 'utf8'),
);
const tag = process.env.NPM_TAG;
const dryRun = process.argv.includes('--dry-run');
if (!dryRun && !tag) throw new Error('NPM_TAG is required');
if (!Array.isArray(packages) || packages.length === 0) throw new Error('Missing release packages');
const names = new Set();
for (const entry of packages) {
  if (
    names.has(entry.name) ||
    basename(entry.filename) !== entry.filename ||
    !entry.filename.endsWith('.tgz')
  ) {
    throw new Error('Invalid release manifest');
  }
  names.add(entry.name);
  const actual = `sha512-${createHash('sha512')
    .update(readFileSync(resolve(directory, entry.filename)))
    .digest('base64')}`;
  if (actual !== entry.integrity) throw new Error(`Archive integrity mismatch: ${entry.name}`);
}
function publishedIntegrity(entry) {
  try {
    return JSON.parse(
      execFileSync(
        'npm',
        [
          'view',
          `${entry.name}@${entry.version}`,
          'dist.integrity',
          '--json',
          '--registry',
          registry,
          '--prefer-online',
        ],
        { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 60000 },
      ),
    );
  } catch (error) {
    let result;
    try {
      result = JSON.parse(error.stdout?.toString() ?? '');
    } catch {
      /* Preserve the original error below. */
    }
    if (result?.error?.code === 'E404') return null;
    throw error;
  }
}
const results = [];
function record(entry, status) {
  results.push({ ...entry, tag, status });
  writeFileSync(
    resolve(directory, 'publish-results.json'),
    JSON.stringify({ registry, packages: results }, null, 2) + '\n',
  );
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(
      process.env.GITHUB_STEP_SUMMARY,
      `- ${entry.name}@${entry.version}: ${status} (tag: ${tag}).\n`,
    );
  }
}
if (!dryRun && process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    '### npm publishing results\n\nPlatform packages are published and verified in the registry before core.\n\n',
  );
}
for (const entry of packages) {
  if (dryRun) {
    console.log(`Publish ${entry.name}@${entry.version} from ${entry.filename}`);
    continue;
  }
  try {
    const integrity = publishedIntegrity(entry);
    if (integrity && integrity !== entry.integrity)
      throw new Error(`Published content differs: ${entry.name}@${entry.version}`);
    if (integrity) {
      record(entry, 'already-published');
      console.log(`Verified published ${entry.name}@${entry.version}`);
      continue;
    }
    execFileSync(
      'npm',
      [
        'publish',
        resolve(directory, entry.filename),
        '--ignore-scripts',
        '--access',
        'public',
        '--provenance',
        '--tag',
        tag,
        '--registry',
        registry,
      ],
      { stdio: 'inherit' },
    );
    const deadline = Date.now() + 600_000;
    while (true) {
      const published = publishedIntegrity(entry);
      if (published) {
        if (published !== entry.integrity)
          throw new Error(`Published content differs: ${entry.name}@${entry.version}`);
        break;
      }
      if (Date.now() >= deadline)
        throw new Error(`Registry visibility timeout: ${entry.name}@${entry.version}`);
      await setTimeout(15_000);
    }
    record(entry, 'published');
    console.log(`Verified published ${entry.name}@${entry.version}`);
  } catch (error) {
    record(entry, 'failed');
    throw error;
  }
}

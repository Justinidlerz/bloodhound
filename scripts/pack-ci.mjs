import { createHash } from 'node:crypto';
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import { execFileSync } from 'node:child_process';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { dependencyOrder, releasePlanPath, workspacePackages } from './workspace-packages.mjs';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const targets = JSON.parse(readFileSync('.github/native-targets.json', 'utf8'));
const expected = targets.map(({ platform }) => `${pkg.napi.binaryName}.${platform}.node`).sort();
const actual = readdirSync('dist')
  .filter((name) => name.endsWith('.node'))
  .sort();
if (JSON.stringify(actual) !== JSON.stringify(expected)) {
  throw new Error(
    `Native artifact mismatch: expected ${expected.join(', ')}, found ${actual.join(', ')}`,
  );
}
const { version } = await import(pathToFileURL(resolve('dist/index.js')));
if (version().scanner !== pkg.version) throw new Error('Native and npm versions differ');
const destination = resolve('artifacts');
mkdirSync(destination, { recursive: true });
const packages = [];
function pack(directory, manifest, required, nativeFiles) {
  const packed = JSON.parse(
    execFileSync(
      'pnpm',
      ['--config.ignore-scripts=true', 'pack', '--json', '--pack-destination', destination],
      { cwd: directory, encoding: 'utf8' },
    ),
  );
  if (packed.name !== manifest.name || packed.version !== manifest.version) {
    throw new Error(`Packed metadata mismatch: ${manifest.name}`);
  }
  const paths = packed.files.map(({ path }) => path);
  for (const path of [...required, 'LICENSE', 'package.json']) {
    if (!paths.includes(path)) throw new Error(`Packed file missing: ${manifest.name}/${path}`);
  }
  if (
    JSON.stringify(paths.filter((path) => path.endsWith('.node')).sort()) !==
    JSON.stringify([...nativeFiles].sort())
  ) {
    throw new Error(`Unexpected native binaries in ${manifest.name}`);
  }
  const archive = readFileSync(packed.filename);
  const entry = {
    name: manifest.name,
    version: manifest.version,
    filename: basename(packed.filename),
    integrity: `sha512-${createHash('sha512').update(archive).digest('base64')}`,
    bytes: archive.length,
  };
  packages.push(entry);
  return packed;
}
for (const { platform } of targets) {
  const directory = join('npm', platform);
  const manifest = JSON.parse(readFileSync(join(directory, 'package.json'), 'utf8'));
  const binary = `${pkg.napi.binaryName}.${platform}.node`;
  if (
    manifest.name !== `${pkg.name}-${platform}` ||
    manifest.version !== pkg.version ||
    manifest.main !== binary
  ) {
    throw new Error(`Platform metadata mismatch: ${platform}`);
  }
  copyFileSync(join('dist', binary), join(directory, binary));
  copyFileSync('LICENSE', join(directory, 'LICENSE'));
  pack(directory, manifest, [binary], [binary]);
}
const platformPackages = [...packages];
const packed = pack(
  '.',
  pkg,
  ['dist/index.js', 'dist/index.d.ts', 'dist/native.js', 'dist/native.d.ts', 'dist/types.d.ts'],
  [],
);
const published = JSON.parse(
  execFileSync('tar', ['-xOf', packed.filename, 'package/package.json'], { encoding: 'utf8' }),
);
const dependencies = Object.fromEntries(
  platformPackages.map(({ name, version }) => [name, version]),
);
if (
  Object.keys(published.optionalDependencies ?? {}).length !== targets.length ||
  Object.entries(dependencies).some(
    ([name, version]) => published.optionalDependencies[name] !== version,
  )
) {
  throw new Error('Core must depend on the exact versions of all platform packages');
}
const alreadyPacked = new Set(packages.map(({ name }) => name));
for (const { directory, manifest } of dependencyOrder(workspacePackages())) {
  if (alreadyPacked.has(manifest.name)) continue;
  const entryPoints = [manifest.main, manifest.types]
    .filter(Boolean)
    .map((path) => path.replace(/^\.\//, ''));
  pack(directory, manifest, entryPoints, []);
  alreadyPacked.add(manifest.name);
}
let releasePackages = packages;
if (existsSync(releasePlanPath)) {
  const plan = JSON.parse(readFileSync(releasePlanPath, 'utf8'));
  if (plan.mode !== 'check' && plan.packages.length > 0) {
    const selected = new Map(plan.packages.map(({ name, version }) => [name, version]));
    releasePackages = packages.filter(({ name }) => selected.has(name));
    if (
      releasePackages.length !== selected.size ||
      releasePackages.some(({ name, version }) => selected.get(name) !== version)
    ) {
      throw new Error('Packed packages do not match the prepared release plan');
    }
  }
}
writeFileSync(
  join(destination, 'release-manifest.json'),
  JSON.stringify({ registry: pkg.publishConfig.registry, packages: releasePackages }, null, 2) +
    '\n',
);
copyFileSync('scripts/publish-ci.mjs', join(destination, 'publish-ci.mjs'));
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Package artifacts\n\n${packages.map(({ name, version, bytes }) => `- ${name}@${version}: ${bytes} bytes`).join('\n')}\n`,
  );
}
console.log(JSON.stringify(packages, null, 2));

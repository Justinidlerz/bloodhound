import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { dependencyOrder, releasePlanPath, workspacePackages } from './workspace-packages.mjs';

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));
const mode = process.env.RELEASE_MODE ?? 'check';
if (!['check', 'beta', 'latest'].includes(mode)) throw new Error('Invalid RELEASE_MODE');
if (readJson('.changeset/config.json').commit !== false) {
  throw new Error('Changesets automatic commits must be disabled');
}
const before = new Map(
  workspacePackages().map(({ manifest }) => [manifest.name, manifest.version]),
);
const pending = readdirSync('.changeset').filter(
  (name) => name.endsWith('.md') && name !== 'README.md',
);
if (mode !== 'check' && pending.length > 0) {
  execFileSync('pnpm', ['exec', 'changeset', 'version'], { stdio: 'inherit' });
}
const workspace = workspacePackages();
const changed = workspace.filter(
  ({ manifest }) => mode !== 'check' && manifest.version !== before.get(manifest.name),
);
const hasRelease = changed.length > 0;
if (hasRelease && mode === 'beta') {
  const parts = ['PR_NUMBER', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'].map((name) => {
    const value = process.env[name];
    if (!/^[1-9]\d*$/.test(value ?? '')) throw new Error(`Missing positive integer ${name}`);
    return value;
  });
  for (const { manifest } of changed) {
    if (manifest.version.includes('-'))
      throw new Error('PR beta requires a stable Changesets version');
    manifest.version += `-beta.${parts.join('.')}`;
  }
}
if (process.env.GITHUB_REPOSITORY) {
  const repository = process.env.GITHUB_REPOSITORY;
  if (!/^[\w-]+\/[\w.-]+$/.test(repository)) throw new Error('Invalid GITHUB_REPOSITORY');
  for (const { manifest } of workspace) {
    manifest.repository = {
      ...(typeof manifest.repository === 'object' ? manifest.repository : {}),
      type: 'git',
      url: `git+https://github.com/${repository}.git`,
    };
  }
}
for (const { directory, manifest } of workspace) {
  writeFileSync(join(directory, 'package.json'), JSON.stringify(manifest, null, 2) + '\n');
}
execFileSync(process.execPath, ['scripts/sync-native-version.mjs'], { stdio: 'inherit' });
execFileSync(process.execPath, ['scripts/sync-platform-packages.mjs'], { stdio: 'inherit' });

const pkg = readJson('package.json');
const prepared = workspacePackages();
const changedNames = new Set(changed.map(({ manifest }) => manifest.name));
const packages = dependencyOrder(prepared)
  .filter(({ manifest }) => changedNames.has(manifest.name))
  .map(({ directory, manifest }) => ({
    directory,
    name: manifest.name,
    version: manifest.version,
  }));
const metadataFiles = [
  'package.json',
  'pnpm-lock.yaml',
  'pnpm-workspace.yaml',
  'native/Cargo.toml',
  'Cargo.lock',
];
for (const { directory } of prepared) {
  for (const name of ['package.json', 'README.md', 'CHANGELOG.md']) {
    const file = join(directory, name);
    if (existsSync(file) && !metadataFiles.includes(file)) metadataFiles.push(file);
  }
}
const tags = packages
  .filter(({ name }) => !Object.hasOwn(pkg.optionalDependencies ?? {}, name))
  .map(({ directory, name, version }) =>
    directory === '.' ? `v${version}` : `${name}@${version}`,
  );
mkdirSync(dirname(releasePlanPath), { recursive: true });
writeFileSync(
  releasePlanPath,
  JSON.stringify({ mode, packages, tags, metadataFiles }, null, 2) + '\n',
);

const outputs = {
  has_release: String(hasRelease),
  version: pkg.version,
  npm_tag: mode === 'beta' ? 'beta' : 'latest',
  matrix: JSON.stringify({ include: readJson('.github/native-targets.json') }),
};
if (process.env.GITHUB_OUTPUT) {
  for (const [key, value] of Object.entries(outputs))
    appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
}
if (process.env.RUNNER_TEMP) {
  const files = [...metadataFiles, '.changeset', releasePlanPath];
  execFileSync('tar', ['-czf', join(process.env.RUNNER_TEMP, 'release-state.tgz'), ...files]);
}
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Release plan\n\n- Pending release: ${hasRelease}\n- npm tag: ${outputs.npm_tag}\n${packages.map(({ name, version }) => `- ${name}@${version}\n`).join('')}`,
  );
}
console.log(JSON.stringify(outputs, null, 2));

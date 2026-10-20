import { appendFileSync, existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));
const mode = process.env.RELEASE_MODE ?? 'check';
if (!['check', 'beta', 'latest'].includes(mode)) throw new Error('Invalid RELEASE_MODE');
if (readJson('.changeset/config.json').commit !== false) {
  throw new Error('Changesets automatic commits must be disabled');
}
const before = readJson('package.json');
const pending = readdirSync('.changeset').filter(
  (name) => name.endsWith('.md') && name !== 'README.md',
);
if (mode !== 'check' && pending.length > 0) {
  execFileSync('pnpm', ['exec', 'changeset', 'version'], { stdio: 'inherit' });
}
const pkg = readJson('package.json');
const hasRelease = mode !== 'check' && pkg.version !== before.version;
if (hasRelease && mode === 'beta') {
  const parts = ['PR_NUMBER', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'].map((name) => {
    const value = process.env[name];
    if (!/^[1-9]\d*$/.test(value ?? '')) throw new Error(`Missing positive integer ${name}`);
    return value;
  });
  if (pkg.version.includes('-')) throw new Error('PR beta requires a stable Changesets version');
  pkg.version += `-beta.${parts.join('.')}`;
}
if (process.env.GITHUB_REPOSITORY) {
  const repository = process.env.GITHUB_REPOSITORY;
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error('Invalid GITHUB_REPOSITORY');
  pkg.repository = { type: 'git', url: `git+https://github.com/${repository}.git` };
}
writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
execFileSync(process.execPath, ['scripts/sync-native-version.mjs'], { stdio: 'inherit' });

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
  const files = ['package.json', 'pnpm-lock.yaml', 'native/Cargo.toml', 'Cargo.lock', '.changeset'];
  if (existsSync('CHANGELOG.md')) files.push('CHANGELOG.md');
  execFileSync('tar', ['-czf', join(process.env.RUNNER_TEMP, 'release-state.tgz'), ...files]);
}
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Release plan\n\n- Package: ${pkg.name}@${pkg.version}\n- Pending release: ${hasRelease}\n- npm tag: ${outputs.npm_tag}\n`,
  );
}
console.log(JSON.stringify(outputs, null, 2));

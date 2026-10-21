import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { copy, fixture, readJson, root, run, workspace, write } from './helpers.mjs';
const success = (result) => assert.equal(result.status, 0, result.stderr || result.stdout);
const failure = (result, pattern) => {
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, pattern);
};

test('clean-dist preserves collected binaries and removes stale bindings and directories', (t) => {
  const dir = fixture(t);
  copy(dir, 'scripts/clean-dist.mjs');
  success(run(dir, 'scripts/clean-dist.mjs'));
  write(dir, 'dist/foreign.node', 'native');
  write(dir, 'dist/index.js', 'stale');
  write(dir, 'dist/subdir/types.d.ts', 'stale');
  success(run(dir, 'scripts/clean-dist.mjs'));
  assert.deepEqual(readdirSync(join(dir, 'dist')), ['foreign.node']);
  assert.equal(readFileSync(join(dir, 'dist/foreign.node'), 'utf8'), 'native');
});
test('schema generator reproduces both public schemas without changing their definitions', (t) => {
  const dir = fixture(t);
  copy(dir, 'scripts/generate-schema.mjs');
  write(dir, 'schemas/.keep', '');
  success(run(dir, 'scripts/generate-schema.mjs'));
  for (const name of ['report', 'index'])
    assert.deepEqual(
      readJson(join(dir, `schemas/${name}.schema.json`)),
      readJson(join(root, `schemas/${name}.schema.json`)),
    );
});
function nativeFixture(t, version = '1.2.3-beta.4') {
  const dir = fixture(t);
  copy(dir, 'scripts/sync-native-version.mjs');
  write(dir, 'package.json', { version });
  write(
    dir,
    'native/Cargo.toml',
    '[package]\nname = "scanner_fixture"\nversion = "1.0.0"\n[dependencies]\nother = "1.0.0"\n',
  );
  write(
    dir,
    'Cargo.lock',
    'version = 4\n\n[[package]]\nname = "other"\nversion = "1.0.0"\n\n[[package]]\nname = "scanner_fixture"\nversion = "1.0.0"\ndependencies = ["other"]\n',
  );
  return dir;
}
test('native version sync changes only the named crate and supports prereleases', (t) => {
  const dir = nativeFixture(t);
  success(run(dir, 'scripts/sync-native-version.mjs'));
  assert.match(readFileSync(join(dir, 'native/Cargo.toml'), 'utf8'), /version = "1\.2\.3-beta\.4"/);
  const lock = readFileSync(join(dir, 'Cargo.lock'), 'utf8');
  assert.match(lock, /name = "other"\nversion = "1\.0\.0"/);
  assert.match(lock, /name = "scanner_fixture"\nversion = "1\.2\.3-beta\.4"/);
  const previous = readFileSync(join(dir, 'native/Cargo.toml'), 'utf8');
  success(run(dir, 'scripts/sync-native-version.mjs'));
  assert.equal(readFileSync(join(dir, 'native/Cargo.toml'), 'utf8'), previous);
});
test('native version sync supports CRLF manifests', (t) => {
  const dir = nativeFixture(t, '2.0.0');
  for (const path of ['native/Cargo.toml', 'Cargo.lock'])
    write(dir, path, readFileSync(join(dir, path), 'utf8').replaceAll('\n', '\r\n'));
  success(run(dir, 'scripts/sync-native-version.mjs'));
  assert.match(readFileSync(join(dir, 'Cargo.lock'), 'utf8'), /version = "2\.0\.0"/);
});
for (const [name, mutate, message] of [
  [
    'invalid semver',
    (dir) => write(dir, 'package.json', { version: 'not-a-version' }),
    /semantic package version/,
  ],
  [
    'missing native metadata',
    (dir) => write(dir, 'native/Cargo.toml', '[package]\nname = "scanner_fixture"\n'),
    /name\/version not found/,
  ],
  [
    'missing lock entry',
    (dir) => write(dir, 'Cargo.lock', 'version = 4\n'),
    /Expected one lockfile entry/,
  ],
  [
    'duplicate lock entries',
    (dir) => write(dir, 'Cargo.lock', readFileSync(join(dir, 'Cargo.lock'), 'utf8').repeat(2)),
    /found 2/,
  ],
  [
    'missing lock version',
    (dir) => write(dir, 'Cargo.lock', '[[package]]\nname = "scanner_fixture"\n'),
    /lockfile version not found/,
  ],
])
  test(`native version sync rejects ${name} before writing`, (t) => {
    const dir = nativeFixture(t);
    mutate(dir);
    const before = readFileSync(join(dir, 'native/Cargo.toml'), 'utf8');
    failure(run(dir, 'scripts/sync-native-version.mjs'), message);
    assert.equal(readFileSync(join(dir, 'native/Cargo.toml'), 'utf8'), before);
  });
test('platform sync uses napi metadata for every build target', (t) => {
  const dir = workspace(t);
  success(run(dir, 'scripts/sync-platform-packages.mjs'));
  const pkg = readJson(join(dir, 'package.json'));
  for (const { platform } of readJson(join(dir, '.github/native-targets.json'))) {
    const generated = readJson(join(dir, `npm/${platform}/package.json`));
    assert.equal(generated.name, `${pkg.name}-${platform}`);
    assert.equal(generated.version, pkg.version);
    assert.deepEqual(generated.files, [generated.main]);
    assert.equal(generated.license, 'MIT');
    const [os, cpu, abi] = platform.split('-');
    assert.deepEqual(generated.os, [os]);
    assert.deepEqual(generated.cpu, [cpu]);
    if (abi === 'gnu') assert.deepEqual(generated.libc, ['glibc']);
    assert.ok(existsSync(join(dir, `npm/${platform}/README.md`)));
  }
});
for (const [name, mutate, error] of [
  ['different target matrix', (p) => p.napi.targets.pop(), /targets must match/],
  [
    'non-workspace optional version',
    (p) => {
      p.optionalDependencies[Object.keys(p.optionalDependencies)[0]] = '^1.0.0';
    },
    /Expected optional workspace dependency/,
  ],
  [
    'extra optional package',
    (p) => {
      p.optionalDependencies.extra = 'workspace:*';
    },
    /Optional dependencies must match/,
  ],
])
  test(`platform sync rejects ${name}`, (t) => {
    const dir = workspace(t);
    const pkg = readJson(join(dir, 'package.json'));
    mutate(pkg);
    write(dir, 'package.json', pkg);
    failure(run(dir, 'scripts/sync-platform-packages.mjs'), error);
  });
for (const mode of ['check', 'beta', 'latest'])
  test(`release preparation handles ${mode} and synchronizes all package versions`, (t) => {
    const dir = workspace(t);
    const pkg = readJson(join(dir, 'package.json'));
    write(
      dir,
      '.changeset/test-release.md',
      `---\n'${pkg.name}': patch\n---\nExercise coordinated release versions.\n`,
    );
    const output = join(dir, 'github-output');
    const summary = join(dir, 'summary');
    success(
      run(dir, 'scripts/prepare-release.mjs', {
        env: {
          RELEASE_MODE: mode,
          PR_NUMBER: '7',
          GITHUB_RUN_ID: '42',
          GITHUB_RUN_ATTEMPT: '2',
          GITHUB_REPOSITORY: 'example/scanner',
          GITHUB_OUTPUT: output,
          GITHUB_STEP_SUMMARY: summary,
        },
      }),
    );
    const parts = pkg.version.split('.').map(Number);
    parts[2]++;
    const expected =
      mode === 'check' ? pkg.version : parts.join('.') + (mode === 'beta' ? '-beta.7.42.2' : '');
    const prepared = readJson(join(dir, 'package.json'));
    assert.equal(prepared.version, expected);
    assert.equal(prepared.repository.url, 'git+https://github.com/example/scanner.git');
    for (const { platform } of readJson(join(dir, '.github/native-targets.json')))
      assert.equal(readJson(join(dir, `npm/${platform}/package.json`)).version, expected);
    assert.match(readFileSync(output, 'utf8'), new RegExp(`has_release=${mode !== 'check'}`));
    assert.match(
      readFileSync(output, 'utf8'),
      new RegExp(`npm_tag=${mode === 'beta' ? 'beta' : 'latest'}`),
    );
    assert.equal(existsSync(join(dir, '.changeset/test-release.md')), mode === 'check');
  });
test('release preparation without a changeset never publishes', (t) => {
  const dir = workspace(t);
  const output = join(dir, 'github-output');
  success(
    run(dir, 'scripts/prepare-release.mjs', {
      env: { RELEASE_MODE: 'latest', GITHUB_OUTPUT: output },
    }),
  );
  assert.match(readFileSync(output, 'utf8'), /has_release=false/);
});
for (const [name, env, config, pattern] of [
  ['unknown mode', { RELEASE_MODE: 'production' }, null, /Invalid RELEASE_MODE/],
  ['automatic commits', {}, true, /automatic commits must be disabled/],
  ['invalid repository', { GITHUB_REPOSITORY: '../bad' }, null, /Invalid GITHUB_REPOSITORY/],
  ['missing beta identifiers', { RELEASE_MODE: 'beta' }, null, /Missing positive integer/],
])
  test(`release preparation rejects ${name}`, (t) => {
    const dir = workspace(t);
    const pkg = readJson(join(dir, 'package.json'));
    write(dir, '.changeset/test-release.md', `---\n'${pkg.name}': patch\n---\nRelease test.\n`);
    if (config !== null) {
      const value = readJson(join(dir, '.changeset/config.json'));
      value.commit = config;
      write(dir, '.changeset/config.json', value);
    }
    failure(run(dir, 'scripts/prepare-release.mjs', { env }), pattern);
  });

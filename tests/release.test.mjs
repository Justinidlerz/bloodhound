import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { copy, fixture, readJson, run, workspace, write } from './helpers.mjs';
const success = (result) => assert.equal(result.status, 0, result.stderr || result.stdout);
const failure = (result, pattern) => {
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, pattern);
};
const integrity = (bytes) => `sha512-${createHash('sha512').update(bytes).digest('base64')}`;
function packageFixture(t) {
  const dir = workspace(t);
  const pkg = readJson(join(dir, 'package.json'));
  write(
    dir,
    'dist/index.js',
    `export function version(){return {scanner:${JSON.stringify(pkg.version)}}}`,
  );
  for (const file of ['dist/index.d.ts', 'dist/native.js', 'dist/native.d.ts', 'dist/types.d.ts'])
    write(dir, file, 'export {};\n');
  for (const { platform } of readJson(join(dir, '.github/native-targets.json')))
    write(dir, `dist/${pkg.napi.binaryName}.${platform}.node`, `binary fixture for ${platform}`);
  return dir;
}
test('packaging creates one binary per platform, a binary-free core, and an ordered integrity manifest', (t) => {
  const dir = packageFixture(t);
  const summary = join(dir, 'summary');
  success(run(dir, 'scripts/pack-ci.mjs', { env: { GITHUB_STEP_SUMMARY: summary } }));
  const manifest = readJson(join(dir, 'artifacts/release-manifest.json'));
  const pkg = readJson(join(dir, 'package.json'));
  const targets = readJson(join(dir, '.github/native-targets.json'));
  assert.deepEqual(
    manifest.packages.map((p) => p.name),
    [...targets.map(({ platform }) => `${pkg.name}-${platform}`), pkg.name],
  );
  for (const item of manifest.packages) {
    const bytes = readFileSync(join(dir, 'artifacts', item.filename));
    assert.equal(item.integrity, integrity(bytes));
    assert.equal(item.bytes, bytes.length);
    assert.equal(item.version, pkg.version);
  }
  assert.equal(
    readFileSync(join(dir, 'artifacts/publish-ci.mjs'), 'utf8'),
    readFileSync(join(dir, 'scripts/publish-ci.mjs'), 'utf8'),
  );
  assert.match(readFileSync(summary, 'utf8'), /Package artifacts/);
});
for (const [name, modify, pattern] of [
  [
    'missing binary',
    (dir) => {
      const file = readdirSync(join(dir, 'dist')).find((name) => name.endsWith('.node'));
      rmSync(join(dir, 'dist', file));
    },
    /Native artifact mismatch/,
  ],
  [
    'unexpected binary',
    (dir) => write(dir, 'dist/unexpected.node', 'extra'),
    /Native artifact mismatch/,
  ],
  [
    'native version mismatch',
    (dir) => write(dir, 'dist/index.js', 'export const version=()=>({scanner:"0.0.0-wrong"})'),
    /Native and npm versions differ/,
  ],
  [
    'platform metadata mismatch',
    (dir) => {
      const [{ platform }] = readJson(join(dir, '.github/native-targets.json'));
      const path = `npm/${platform}/package.json`;
      const pkg = readJson(join(dir, path));
      pkg.version = '0.0.0-wrong';
      write(dir, path, pkg);
    },
    /Platform metadata mismatch/,
  ],
  ['missing declarations', (dir) => rmSync(join(dir, 'dist/types.d.ts')), /Packed file missing/],
  [
    'binary leakage into core',
    (dir) => {
      const pkg = readJson(join(dir, 'package.json'));
      pkg.files.push('dist/*.node');
      write(dir, 'package.json', pkg);
    },
    /Unexpected native binaries/,
  ],
  [
    'unlocked optional versions',
    (dir) => {
      const pkg = readJson(join(dir, 'package.json'));
      for (const name of Object.keys(pkg.optionalDependencies))
        pkg.optionalDependencies[name] = '*';
      write(dir, 'package.json', pkg);
    },
    /exact versions/,
  ],
])
  test(`packaging rejects ${name}`, (t) => {
    const dir = packageFixture(t);
    modify(dir);
    failure(run(dir, 'scripts/pack-ci.mjs'), pattern);
    assert.equal(existsSync(join(dir, 'artifacts/release-manifest.json')), false);
  });
function publishFixture(t, config = {}) {
  const dir = fixture(t);
  copy(dir, 'scripts/publish-ci.mjs');
  const packages = ['platform-a', 'platform-b', 'core'].map((name) => {
    const bytes = Buffer.from(`archive for ${name}`);
    const filename = `${name}-1.2.3.tgz`;
    write(dir, `scripts/${filename}`, bytes);
    return {
      name: `@fixture/${name}`,
      version: '1.2.3',
      filename,
      integrity: integrity(bytes),
      bytes: bytes.length,
    };
  });
  write(dir, 'scripts/release-manifest.json', { registry: 'https://registry.invalid/', packages });
  write(dir, 'mock-config.json', config);
  const preload = write(
    dir,
    'mock-npm.mjs',
    `
import { mock } from 'node:test';
import { readFileSync, writeFileSync } from 'node:fs';
import { basename } from 'node:path';
const config=JSON.parse(readFileSync('mock-config.json','utf8'));
const {packages}=JSON.parse(readFileSync('scripts/release-manifest.json','utf8'));
const calls=[]; const published=new Map(Object.entries(config.existing??{}));
const pending=new Map(); let clock=0;
Date.now=()=>clock;
mock.module('node:timers/promises',{namedExports:{setTimeout:async(ms)=>{clock+=ms;}}});
function fail(code){const error=new Error(code);error.stdout=JSON.stringify({error:{code}});throw error;}
mock.module('node:child_process',{namedExports:{execFileSync(command,args){
  if(command!=='npm')throw Error('Unexpected command: '+command);
  calls.push(args);writeFileSync('commands.json',JSON.stringify(calls));
  if(args[0]==='view'){
    if(config.viewError)fail(config.viewError);
    const name=args[1].slice(0,args[1].lastIndexOf('@'));
    if(pending.has(name)){
      const count=pending.get(name);pending.set(name,count+1);
      if(config.neverVisible || count<(config.delayViews??0))fail('E404');
    }
    if(!published.has(name))fail('E404');
    return JSON.stringify(published.get(name));
  }
  if(args[0]==='publish'){
    const entry=packages.find(p=>p.filename===basename(args[1]));if(!entry)throw Error('Unknown archive');
    if(config.publishError && (!config.failPackage || config.failPackage===entry.name))fail(config.publishError);
    published.set(entry.name,config.publishedIntegrity??entry.integrity);pending.set(entry.name,0);
    return '';
  }
  throw Error('Unexpected npm operation');
}}});
`,
  );
  return {
    dir,
    packages,
    preload,
    execute: (extra = {}) =>
      run(dir, 'scripts/publish-ci.mjs', { preload, env: { NPM_TAG: 'beta' }, ...extra }),
    commands: () =>
      existsSync(join(dir, 'commands.json')) ? readJson(join(dir, 'commands.json')) : [],
  };
}
test('publishing dry-run validates every archive without contacting npm', (t) => {
  const fixture = publishFixture(t);
  const result = fixture.execute({ args: ['--dry-run'], env: {} });
  success(result);
  assert.equal(fixture.commands().length, 0);
  assert.ok(result.stdout.indexOf('@fixture/platform-a') < result.stdout.indexOf('@fixture/core'));
});
for (const [name, modify, pattern] of [
  [
    'tampered archive',
    (f) => write(f.dir, `scripts/${f.packages[0].filename}`, 'tampered'),
    /integrity mismatch/,
  ],
  [
    'path traversal',
    (f) => {
      const manifest = readJson(join(f.dir, 'scripts/release-manifest.json'));
      manifest.packages[0].filename = '../escape.tgz';
      write(f.dir, 'scripts/release-manifest.json', manifest);
    },
    /Invalid release manifest/,
  ],
  [
    'duplicate names',
    (f) => {
      const manifest = readJson(join(f.dir, 'scripts/release-manifest.json'));
      manifest.packages[1].name = manifest.packages[0].name;
      write(f.dir, 'scripts/release-manifest.json', manifest);
    },
    /Invalid release manifest/,
  ],
  [
    'missing release packages',
    (f) =>
      write(f.dir, 'scripts/release-manifest.json', {
        registry: 'https://registry.invalid/',
        packages: [],
      }),
    /Missing release packages/,
  ],
])
  test(`publishing rejects ${name} before any npm command`, (t) => {
    const fixture = publishFixture(t);
    modify(fixture);
    failure(fixture.execute(), pattern);
    assert.equal(fixture.commands().length, 0);
  });
test('publishing requires a dist-tag outside dry-run', (t) => {
  const fixture = publishFixture(t);
  failure(fixture.execute({ env: {} }), /NPM_TAG is required/);
  assert.equal(fixture.commands().length, 0);
});
test('publishing submits platforms before core and verifies public availability', (t) => {
  const fixture = publishFixture(t);
  const summary = join(fixture.dir, 'summary');
  success(fixture.execute({ env: { NPM_TAG: 'beta', GITHUB_STEP_SUMMARY: summary } }));
  const calls = fixture.commands();
  const publishes = calls.filter((args) => args[0] === 'publish');
  assert.equal(publishes.length, fixture.packages.length);
  assert.deepEqual(
    publishes.map((args) => args[1].split(/[\\/]/).at(-1)),
    fixture.packages.map((p) => p.filename),
  );
  for (const args of publishes) {
    assert.ok(args.includes('--ignore-scripts'));
    assert.ok(args.includes('--provenance'));
    assert.equal(args[args.indexOf('--tag') + 1], 'beta');
    assert.equal(args[args.indexOf('--registry') + 1], 'https://registry.invalid/');
  }
  assert.deepEqual(
    calls.map((args) => args[0]),
    ['view', 'publish', 'view', 'view', 'publish', 'view', 'view', 'publish', 'view'],
  );
  const results = readJson(join(fixture.dir, 'scripts/publish-results.json'));
  assert.deepEqual(
    results.packages.map((p) => p.status),
    ['published', 'published', 'published'],
  );
  assert.deepEqual(
    results.packages.map((p) => p.integrity),
    fixture.packages.map((p) => p.integrity),
  );
  assert.match(readFileSync(summary, 'utf8'), /verified in the registry before core/);
});
test('publishing retries skip identical public versions and submit only missing archives', (t) => {
  const fixture = publishFixture(t);
  write(fixture.dir, 'mock-config.json', {
    existing: { [fixture.packages[0].name]: fixture.packages[0].integrity },
  });
  success(fixture.execute());
  assert.equal(fixture.commands().filter((args) => args[0] === 'publish').length, 2);
  assert.equal(
    readJson(join(fixture.dir, 'scripts/publish-results.json')).packages[0].status,
    'already-published',
  );
});
for (const [name, config, pattern] of [
  [
    'existing content mismatch',
    { existing: { '@fixture/platform-a': 'different' } },
    /Published content differs/,
  ],
  ['registry authorization error', { viewError: 'E403' }, /E403/],
  ['publish authorization error', { publishError: 'E403' }, /E403/],
  ['duplicate staged version', { publishError: 'EPUBLISHCONFLICT' }, /EPUBLISHCONFLICT/],
  ['publishing network failure', { publishError: 'ECONNRESET' }, /ECONNRESET/],
  ['registry visibility timeout', { neverVisible: true }, /Registry visibility timeout/],
  [
    'post-publish integrity mismatch',
    { publishedIntegrity: 'different' },
    /Published content differs/,
  ],
])
  test(`publishing stops before core on ${name}`, (t) => {
    const fixture = publishFixture(t, config);
    failure(fixture.execute(), pattern);
    assert.ok(
      !fixture
        .commands()
        .some((args) => args[0] === 'publish' && args[1].endsWith('core-1.2.3.tgz')),
    );
    assert.equal(
      readJson(join(fixture.dir, 'scripts/publish-results.json')).packages.at(-1).status,
      'failed',
    );
  });
test('partial publishing failure retains successful package results for recovery', (t) => {
  const fixture = publishFixture(t, { publishError: 'E403', failPackage: '@fixture/platform-b' });
  failure(fixture.execute(), /E403/);
  assert.deepEqual(
    readJson(join(fixture.dir, 'scripts/publish-results.json')).packages.map((p) => p.status),
    ['published', 'failed'],
  );
});
test('latest publishing preserves the requested tag without invoking staging or approval', (t) => {
  const fixture = publishFixture(t);
  success(fixture.execute({ env: { NPM_TAG: 'latest' } }));
  assert.ok(
    fixture
      .commands()
      .every(
        (args) =>
          args[0] === 'view' ||
          (args[0] === 'publish' && args[args.indexOf('--tag') + 1] === 'latest'),
      ),
  );
});
test('fully published artifacts require no publishing', (t) => {
  const fixture = publishFixture(t);
  write(fixture.dir, 'mock-config.json', {
    existing: Object.fromEntries(fixture.packages.map((p) => [p.name, p.integrity])),
  });
  success(fixture.execute());
  assert.ok(fixture.commands().every((args) => args[0] === 'view'));
  assert.ok(
    readJson(join(fixture.dir, 'scripts/publish-results.json')).packages.every(
      (p) => p.status === 'already-published',
    ),
  );
});

test('publishing waits for delayed registry visibility before submitting the next package', (t) => {
  const fixture = publishFixture(t, { delayViews: 2 });
  success(fixture.execute());
  const calls = fixture.commands();
  const positions = calls.flatMap((args, index) => (args[0] === 'publish' ? [index] : []));
  assert.deepEqual(positions, [1, 6, 11]);
  assert.equal(calls.length, 15);
});

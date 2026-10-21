import { spawnSync } from 'node:child_process';
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
export const root = fileURLToPath(new URL('../', import.meta.url));
export const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));
export function fixture(t) {
  const dir = mkdtempSync(join(tmpdir(), 'bloodhound-unit-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}
export function write(dir, path, content) {
  const target = join(dir, path);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(
    target,
    typeof content === 'string' || Buffer.isBuffer(content)
      ? content
      : JSON.stringify(content, null, 2) + '\n',
  );
  return target;
}
export function copy(dir, path) {
  const target = join(dir, path);
  mkdirSync(dirname(target), { recursive: true });
  cpSync(join(root, path), target, { recursive: true });
}
export function run(dir, script, { args = [], env = {}, preload } = {}) {
  const childEnv = { ...process.env };
  for (const key of Object.keys(childEnv)) {
    if (
      key.startsWith('GITHUB_') ||
      [
        'RUNNER_TEMP',
        'RELEASE_MODE',
        'PR_NUMBER',
        'NPM_TAG',
        'NPM_TOKEN',
        'NODE_AUTH_TOKEN',
        'NODE_OPTIONS',
      ].includes(key)
    )
      delete childEnv[key];
  }
  const config = write(dir, 'empty.npmrc', 'registry=https://registry.npmjs.org/\n');
  return spawnSync(
    process.execPath,
    [
      ...(preload ? ['--experimental-test-module-mocks', '--import', preload] : []),
      join(dir, script),
      ...args,
    ],
    {
      cwd: dir,
      encoding: 'utf8',
      timeout: 60000,
      env: { ...childEnv, npm_config_userconfig: config, ...env },
    },
  );
}
export function workspace(t) {
  const dir = fixture(t);
  for (const path of [
    'package.json',
    'pnpm-workspace.yaml',
    'pnpm-lock.yaml',
    'Cargo.lock',
    'native/Cargo.toml',
    '.changeset/config.json',
    '.github/native-targets.json',
    'scripts',
    'LICENSE',
  ])
    copy(dir, path);
  const targets = readJson(join(dir, '.github/native-targets.json'));
  for (const { platform } of targets) copy(dir, `npm/${platform}/package.json`);
  symlinkSync(
    join(root, 'node_modules'),
    join(dir, 'node_modules'),
    process.platform === 'win32' ? 'junction' : 'dir',
  );
  return dir;
}

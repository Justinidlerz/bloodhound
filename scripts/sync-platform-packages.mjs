import { readFileSync, writeFileSync } from 'node:fs';
import { NapiCli } from '@napi-rs/cli';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const targets = JSON.parse(readFileSync('.github/native-targets.json', 'utf8'));
const expectedTargets = targets.map(({ target }) => target).sort();
if (JSON.stringify([...pkg.napi.targets].sort()) !== JSON.stringify(expectedTargets)) {
  throw new Error('napi targets must match the native build matrix');
}
const expectedDependencies = Object.fromEntries(
  targets.map(({ platform }) => [`${pkg.name}-${platform}`, 'workspace:*']),
);
for (const [name, range] of Object.entries(expectedDependencies)) {
  if (pkg.optionalDependencies?.[name] !== range) {
    throw new Error(`Expected optional workspace dependency ${name}`);
  }
}
if (Object.keys(pkg.optionalDependencies).length !== targets.length) {
  throw new Error('Optional dependencies must match the native build matrix');
}
const previousWasi = new Map(
  targets
    .filter(({ wasi }) => wasi)
    .map(({ platform }) => [
      platform,
      JSON.parse(readFileSync(`npm/${platform}/package.json`, 'utf8')),
    ]),
);
await new NapiCli().createNpmDirs({ npmDir: 'npm' });
for (const { platform, wasi } of targets) {
  if (!wasi) continue;
  const path = `npm/${platform}/package.json`;
  const manifest = JSON.parse(readFileSync(path, 'utf8'));
  manifest.imports = { '#binding': `./${manifest.main}` };
  manifest.main = 'index.cjs';
  delete manifest.browser;
  manifest.files = manifest.files.filter((file) => !file.includes('browser'));
  manifest.files.push('index.cjs', 'types.d.ts');
  const previousDependencies = previousWasi.get(platform).dependencies ?? {};
  for (const name of ['@napi-rs/wasm-runtime', 'undici']) {
    if (previousDependencies[name]) manifest.dependencies[name] = previousDependencies[name];
  }
  if (!manifest.dependencies.undici) throw new Error('WASI HTTP runtime dependency is required');
  manifest.dependencies['@bloodhound-rs/manifest'] = pkg.dependencies['@bloodhound-rs/manifest'];
  writeFileSync(path, JSON.stringify(manifest, null, 2) + '\n');
}

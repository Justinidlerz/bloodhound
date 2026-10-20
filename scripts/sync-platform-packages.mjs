import { readFileSync } from 'node:fs';
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
await new NapiCli().createNpmDirs({ npmDir: 'npm' });

import { appendFileSync, mkdirSync, readFileSync, readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

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
const { defaults } = await import(pathToFileURL(resolve('dist/index.js')));
if (defaults().versions.scanner !== pkg.version) throw new Error('Native and npm versions differ');
mkdirSync('artifacts', { recursive: true });
const [packed] = JSON.parse(
  execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', 'artifacts'], {
    encoding: 'utf8',
  }),
);
const paths = new Set(packed.files.map(({ path }) => path));
for (const path of [
  'dist/index.js',
  'dist/index.d.ts',
  'dist/types.d.ts',
  'schemas/report.schema.json',
  ...expected.map((name) => `dist/${name}`),
]) {
  if (!paths.has(path)) throw new Error(`Packed file missing: ${path}`);
}
if (packed.name !== pkg.name || packed.version !== pkg.version)
  throw new Error('Packed metadata mismatch');
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Package artifact\n\n- ${pkg.name}@${pkg.version}\n- Native targets: ${targets.length}\n- Archive: ${packed.filename}\n- Integrity: ${packed.integrity}\n`,
  );
}
console.log(
  JSON.stringify(
    {
      filename: packed.filename,
      version: packed.version,
      platforms: targets.map(({ platform }) => platform),
    },
    null,
    2,
  ),
);

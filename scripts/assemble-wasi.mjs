import { copyFileSync, existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const source = 'artifacts/wasi';
const targets = JSON.parse(readFileSync('.github/native-targets.json', 'utf8'));
if (!existsSync(source) && process.argv.includes('--if-present')) process.exit(0);
for (const { platform, wasi } of targets) {
  if (!wasi) continue;
  const directory = join('npm', platform);
  const manifest = JSON.parse(readFileSync(join(directory, 'package.json'), 'utf8'));
  for (const file of manifest.files) {
    const path =
      file === 'index.cjs'
        ? 'scripts/wasi/index.cjs'
        : file === 'types.d.ts'
          ? 'dist/types.d.ts'
          : join(source, file);
    if (!existsSync(path)) throw new Error(`Packed file missing: ${manifest.name}/${file}`);
    copyFileSync(path, join(directory, file));
  }
  copyFileSync('LICENSE', join(directory, 'LICENSE'));
}

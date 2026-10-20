import { readFileSync, writeFileSync } from 'node:fs';

const { version } = JSON.parse(readFileSync('package.json', 'utf8'));
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error('Expected a semantic package version');
}
const manifestPath = 'native/Cargo.toml';
const manifest = readFileSync(manifestPath, 'utf8');
const packageSection = manifest.match(/^\[package\]\r?\n[\s\S]*?(?=^\[|(?![\s\S]))/m)?.[0];
const name = packageSection?.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
if (!name || !/^version\s*=\s*"[^"]+"/m.test(packageSection)) {
  throw new Error('Native package name/version not found');
}
const lock = readFileSync('Cargo.lock', 'utf8');
let matches = 0;
const updatedLock = lock.replace(
  /^\[\[package\]\]\r?\n[\s\S]*?(?=^\[\[package\]\]|(?![\s\S]))/gm,
  (block) => {
    if (block.match(/^name = "([^"]+)"/m)?.[1] !== name) return block;
    matches++;
    if (!/^version = "[^"]+"/m.test(block)) throw new Error('Native lockfile version not found');
    return block.replace(/^version = "[^"]+"/m, `version = "${version}"`);
  },
);
if (matches !== 1) throw new Error(`Expected one lockfile entry for ${name}, found ${matches}`);
writeFileSync(
  manifestPath,
  manifest.replace(
    packageSection,
    packageSection.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`),
  ),
);
writeFileSync('Cargo.lock', updatedLock);

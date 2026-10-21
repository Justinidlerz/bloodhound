import { readFile } from 'node:fs/promises';
import { scanJs } from '@bloodhound-rs/core';

const code = await readFile(new URL('./fixtures/app.js', import.meta.url), 'utf8');
const manifest = await scanJs(code, {
  scriptMode: 'classic',
  executionEnvironment: 'window',
  dependencyMode: 'single',
});
process.stdout.write(JSON.stringify(manifest) + '\n');

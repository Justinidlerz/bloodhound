// Select Bloodhound's WASI binding before other native addons are imported.
import { readFileSync } from 'node:fs';

const previous = process.env.NAPI_RS_FORCE_WASI;
try {
  process.env.NAPI_RS_FORCE_WASI = 'error';
  const binding = await import('../dist/native.js');
  if (binding.__napiBindingTarget === 'native') throw new Error('WASI binding was not loaded');
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  if (binding.version().scanner !== pkg.version) throw new Error('WASI and npm versions differ');
} finally {
  if (previous === undefined) delete process.env.NAPI_RS_FORCE_WASI;
  else process.env.NAPI_RS_FORCE_WASI = previous;
}

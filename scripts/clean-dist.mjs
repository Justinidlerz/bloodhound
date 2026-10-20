import { existsSync, readdirSync, rmSync } from 'node:fs';
const output = new URL('../dist/', import.meta.url);
// Preserve native artifacts collected from the other CI platform jobs.
if (existsSync(output)) {
  for (const entry of readdirSync(output)) {
    if (!entry.endsWith('.node')) rmSync(new URL(entry, output), { recursive: true, force: true });
  }
}

import { open, realpath } from 'node:fs/promises';
import { isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Resolve symlinks before reading so a source-map path cannot escape the selected directory. */
export async function readProjectFile(root, input, maxBytes = 64 * 1024 * 1024) {
  const path = input.startsWith('file:') ? fileURLToPath(input) : input;
  const real = await realpath(resolve(root, path));
  const local = relative(root, real);
  if (local === '..' || local.startsWith(`..${sep}`) || isAbsolute(local)) {
    throw new Error('File is outside the project directory');
  }
  const handle = await open(real, 'r');
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.size > maxBytes) throw new Error('File is unavailable or too large');
    return await handle.readFile();
  } finally {
    await handle.close();
  }
}

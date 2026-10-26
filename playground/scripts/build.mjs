import { cp, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { rolldown } from 'rolldown';

export const root = fileURLToPath(new URL('../', import.meta.url));
export const output = fileURLToPath(new URL('../dist/', import.meta.url));

export async function build() {
  await mkdir(output, { recursive: true });
  const bundle = await rolldown({
    input: {
      app: `${root}src/app.ts`,
      'manifest-worker': `${root}src/manifest-worker.ts`,
    },
    platform: 'browser',
  });
  try {
    await bundle.write({
      dir: output,
      format: 'esm',
      entryFileNames: '[name].js',
      chunkFileNames: 'chunks/[name]-[hash].js',
      sourcemap: true,
    });
    await Promise.all(
      ['index.html', 'styles.css', 'icon.svg'].map((name) =>
        cp(`${root}${name}`, `${output}${name}`),
      ),
    );
  } finally {
    await bundle.close();
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await build();
  console.log('Playground built in playground/dist');
}

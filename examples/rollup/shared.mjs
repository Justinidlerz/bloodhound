// Shared by rollup.config.mjs and rolldown.config.mjs: the plugin options are identical.

/** Emits a page that references the entry chunk, so the scan starts from HTML like a real deploy. */
export function emitHtml() {
  return {
    name: 'example-html',
    generateBundle(_options, bundle) {
      const entry = Object.values(bundle).find((item) => item.type === 'chunk' && item.isEntry);
      this.emitFile({
        type: 'asset',
        fileName: 'index.html',
        source: `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Bloodhound bundler example</title>
    <style>:root { --brand: #c00; } body { margin: 0; }</style>
  </head>
  <body>
    <div id="app"></div>
    <script>window.__BOOT__ = Date.now();</script>
    <script type="module" src="/${entry.fileName}"></script>
  </body>
</html>
`,
      });
    },
  };
}

export const input = new URL('./src/main.js', import.meta.url).pathname;

export const output = (bundler) => ({
  dir: new URL(`./dist/${bundler}`, import.meta.url).pathname,
  format: 'es',
  entryFileNames: 'assets/[name]-[hash].js',
  chunkFileNames: 'assets/[name]-[hash].js',
  // Hidden maps keep production output clean; the plugin still uses them to map back to src/.
  sourcemap: 'hidden',
});

export const bloodhoundOptions = {
  // Root-relative `/assets/...` references in index.html resolve against this origin.
  publicPath: 'https://example.test/',
  allowGlobals: ['__APP_VERSION__', '__BOOT__', 'dataLayer', 'dataLayer.*'],
  // BLOODHOUND_STRICT=1 fails the build on any remaining global mutation.
  globals: process.env.BLOODHOUND_STRICT ? 'error' : 'warn',
  findings: 'warn',
  onManifest(manifest, { diagnostics }) {
    console.log(
      `bloodhound: ${manifest.summary.sources} sources, ${manifest.summary.findings} findings, ${diagnostics.length} reported after filters`,
    );
  },
};

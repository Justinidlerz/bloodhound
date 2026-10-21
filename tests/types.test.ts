import {
  scan,
  scanJs,
  scanHtml,
  scanResource,
  resumeScan,
  compactReport,
  GlobalIndex,
  type Manifest,
  type Report,
  type Checkpoint,
  type ScanOptions,
} from '@bloodhound-rs/core';

const html: Promise<Manifest> = scanHtml('<script>window.x=1</script>');
const js: Promise<Manifest> = scanJs('globalThis.x=1', { scriptMode: 'module' });
const single: Promise<Manifest> = scanResource({
  kind: 'css',
  source: { type: 'content', content: 'body { color: red }' },
});
const batch: Promise<Report> = scan([{ kind: 'js', source: { type: 'file', path: './app.js' } }], {
  reportFormat: 'full',
});
const full: Promise<Report> = scanHtml('', { reportFormat: 'full' });
declare const options: ScanOptions;
const dynamic: Promise<Manifest | Report> = scanJs('', options);
declare const checkpoint: Checkpoint;
const resumed: Promise<Manifest> = resumeScan(checkpoint);
const resumedFull: Promise<Report> = resumeScan(checkpoint, { reportFormat: 'full' });
new GlobalIndex(await full);
const projected: Manifest = compactReport(await full);
// @ts-expect-error Compact manifests do not contain the full read-access index.
new GlobalIndex(await html);
void [html, js, single, batch, full, dynamic, resumed, resumedFull, projected];

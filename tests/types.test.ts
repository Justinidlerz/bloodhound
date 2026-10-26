import {
  scan,
  scanJs,
  scanHtml,
  defaults,
  version,
  type Manifest,
  type ScanOptions,
  type Rule,
} from '@bloodhound-rs/core';

const options: ScanOptions = {
  scriptMode: 'module',
  dependencyMode: 'recursive',
  budgets: { maxResources: 10 },
  access: { protocols: [] },
};
const html: Promise<Manifest> = scanHtml('<script>window.x=1</script>');
const js: Promise<Manifest> = scanJs('window.x=1', options);
const batch: Promise<Manifest> = scan(
  [{ kind: 'js', source: { type: 'file', path: './app.js' } }],
  options,
);
const rule: Rule = (await js).findings[0]!.rule;
// @ts-expect-error Invalid modes must be rejected by TypeScript.
scanJs('', { scriptMode: 'auto' });
// @ts-expect-error Removed report formats are not accepted.
scanJs('', { reportFormat: 'full' });
void [html, batch, rule, defaults().budgets.maxResources, version().schema];

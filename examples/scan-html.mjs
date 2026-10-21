import { readFile } from 'node:fs/promises';
import { scanHtml } from '@bloodhound-rs/core';

const page = new URL('./fixtures/html/index.html', import.meta.url);
const html = await readFile(page, 'utf8');
// This local fixture uses relative URLs. Its file URL supplies their base.
// The SDK discovers base elements, import maps, and dependencies in the HTML.
const manifest = await scanHtml(html, { resourceUrl: page.href });
process.stdout.write(JSON.stringify(manifest) + '\n');

import { scanHtml } from '@bloodhound-rs/core';

// Usage: node examples/scan-stdin.mjs [original-page-url] < page.html
// Optional persistence belongs to the caller: append > manifest.json.
process.stdin.setEncoding('utf8');
let html = '';
for await (const chunk of process.stdin) html += chunk;
const manifest = await scanHtml(html, { resourceUrl: process.argv[2] });
process.stdout.write(JSON.stringify(manifest) + '\n');

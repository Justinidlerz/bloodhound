import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { watch } from 'node:fs';
import { basename, extname, resolve, sep } from 'node:path';
import { parseArgs } from 'node:util';
import { build, output, root } from './build.mjs';

const { values } = parseArgs({
  options: {
    port: { type: 'string', default: '4173' },
    // Local manifest file served read-only at /__manifest for preloading,
    // or a remote http(s) manifest URL the page fetches itself.
    manifest: { type: 'string' },
    'manifest-url': { type: 'string' },
  },
});
const port = Number(values.port);
if (!Number.isInteger(port) || port < 0 || port > 65535) {
  throw new Error('--port must be an integer from 0 to 65535');
}
const manifestPath = values.manifest ? resolve(values.manifest) : undefined;
if (manifestPath && !(await stat(manifestPath).catch(() => undefined))?.isFile()) {
  throw new Error(`--manifest file not found: ${values.manifest}`);
}
const MAX_MANIFEST_BYTES = 64 * 1024 * 1024;
let manifestUrl;
if (values['manifest-url']) {
  const url = new URL(values['manifest-url']);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new Error('--manifest-url must be an http(s) URL without credentials');
  }
  manifestUrl = url.href;
}
await build();

const mimeTypes = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
};
const server = createServer(async (request, response) => {
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Referrer-Policy', 'no-referrer');
  response.setHeader(
    'Content-Security-Policy',
    // connect-src allows fetching a source file as text when the user picks
    // "View source". Fetched text is rendered, never executed.
    "default-src 'self'; script-src 'self'; style-src 'self'; worker-src 'self'; connect-src 'self' http: https:; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
  );
  if (!['GET', 'HEAD'].includes(request.method ?? '')) {
    response.writeHead(405, { Allow: 'GET, HEAD' }).end('Method not allowed');
    return;
  }
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (manifestPath && pathname === '/__manifest') {
      // Read fresh on every request so a regenerated manifest is picked up
      // by a page refresh.
      const info = await stat(manifestPath);
      if (info.size > MAX_MANIFEST_BYTES) {
        response.writeHead(413).end('Manifest larger than 64 MiB');
        return;
      }
      const content = await readFile(manifestPath);
      response.writeHead(200, { 'Content-Type': 'application/json; charset=utf-8' });
      response.end(request.method === 'HEAD' ? undefined : content);
      return;
    }
    const path = resolve(output, `.${pathname === '/' ? '/index.html' : pathname}`);
    if (!path.startsWith(resolve(output) + sep) || !(await stat(path)).isFile()) {
      response.writeHead(404).end('Not found');
      return;
    }
    const content = await readFile(path);
    response.writeHead(200, {
      'Content-Type': mimeTypes[extname(path)] ?? 'application/octet-stream',
    });
    response.end(request.method === 'HEAD' ? undefined : content);
  } catch {
    response.writeHead(404).end('Not found');
  }
});
server.on('error', (error) => {
  console.error(error.message);
  shutdown(1);
});

let timer;
let pending = false;
let building = false;
async function rebuild() {
  pending = true;
  if (building) return;
  building = true;
  while (pending) {
    pending = false;
    try {
      await build();
      console.log('Playground updated. Reload the page to use the new build.');
    } catch (error) {
      console.error('Build failed:', error.message);
    }
  }
  building = false;
}
const watchers = [
  watch(`${root}src`, { recursive: true }, schedule),
  watch(`${root}index.html`, schedule),
  watch(`${root}styles.css`, schedule),
  watch(`${root}icon.svg`, schedule),
];
function schedule() {
  clearTimeout(timer);
  timer = setTimeout(rebuild, 120);
}
function shutdown(code = 0) {
  clearTimeout(timer);
  watchers.forEach((watcher) => watcher.close());
  server.close(() => process.exit(code));
}
process.on('SIGINT', () => shutdown());
process.on('SIGTERM', () => shutdown());
server.listen(port, '127.0.0.1', () => {
  const baseUrl = `http://127.0.0.1:${server.address().port}`;
  const preload = manifestPath
    ? `${baseUrl}/?manifest=${encodeURIComponent('/__manifest')}&name=${encodeURIComponent(basename(manifestPath))}`
    : manifestUrl
      ? `${baseUrl}/?manifest=${encodeURIComponent(manifestUrl)}`
      : undefined;
  console.log(`Bloodhound Playground: ${preload ?? baseUrl}`);
  console.log(
    preload
      ? 'The manifest loads automatically. Press Ctrl+C to stop.'
      : 'Drop a Manifest v2 JSON file into the page. Press Ctrl+C to stop.',
  );
});

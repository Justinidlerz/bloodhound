import assert from 'node:assert/strict';
import { once, getEventListeners } from 'node:events';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';
import { scan, scanJs, scanHtml, version, defaults } from '@bloodhound-rs/core';

const offline = { access: { protocols: [] }, scriptMode: 'module' };
const source = (content, resourceUrl) => ({ type: 'content', content, resourceUrl });
async function server(t, handler) {
  const instance = createServer(handler);
  instance.listen(0, '127.0.0.1');
  await once(instance, 'listening');
  t.after(async () => {
    instance.closeAllConnections();
    await new Promise((resolve) => instance.close(resolve));
  });
  return { instance, url: `http://127.0.0.1:${instance.address().port}` };
}

test('SDK exports version and isolated defaults', () => {
  assert.equal(version().schema, 2);
  assert.match(version().parser, /^oxc-/);
  const first = defaults();
  first.budgets.maxResources = 0;
  assert.ok(defaults().budgets.maxResources > 0);
});

test('text, content, file and file URL inputs preserve UTF-16 locations', async (t) => {
  const dir = await mkdtemp(join(tmpdir(), 'bloodhound-sdk-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const path = join(dir, 'space name.js');
  const code = '/*😀*/ window.shared=1;';
  await writeFile(path, code);
  for (const input of [
    code,
    source(code),
    { type: 'file', path },
    { type: 'url', url: pathToFileURL(path).href },
  ]) {
    const manifest = await scanJs(input, { scriptMode: 'module' });
    assert.equal(manifest.status, 'completed');
    assert.equal(manifest.schemaVersion, 2);
    assert.equal(manifest.findings.length, 1);
    assert.equal(manifest.findings[0].target, 'window.shared');
    assert.equal(manifest.findings[0].location.column, 7);
    assert.deepEqual(manifest.sources[0].chain, manifest.entries);
  }
});

test('HTML recursively discovers scripts and lists CSS without CSS rule scans', async () => {
  const html =
    '<base href="/assets/"><script type="importmap">{"imports":{"pkg":"./pkg.js"}}</script><script type="module">import "pkg"; window.boot=1</script><link rel="stylesheet" href="app.css"><style>body{--theme:red}</style><div style="color:red"></div>';
  const manifest = await scanHtml(html, {
    ...offline,
    resourceUrl: 'https://example.test/page.html',
    resourceMappings: { 'https://example.test/assets/pkg.js': source('window.library=1') },
  });
  assert.equal(manifest.status, 'completed');
  assert.deepEqual(manifest.findings.map((f) => f.target).sort(), [
    'window.boot',
    'window.library',
  ]);
  assert.equal(manifest.dependencies.filter((d) => d.reason === 'css-not-analyzed').length, 2);
  assert.equal(manifest.sources.length, 3);
  assert.equal(manifest.sources.find((s) => s.url?.endsWith('/pkg.js')).chain.length, 3);
});

test('single mode records dependencies without following inline or external resources', async () => {
  const manifest = await scanHtml('<script>window.x=1</script><script src="a.js"></script>', {
    ...offline,
    resourceUrl: 'https://example.test/',
    dependencyMode: 'single',
  });
  assert.equal(manifest.sources.length, 1);
  assert.equal(manifest.findings.length, 0);
  assert.ok(manifest.dependencies.every((d) => d.reason === 'single-mode'));
});

test('logical URLs and finite dynamic import candidates use offline mappings', async () => {
  const manifest = await scanJs('import(flag ? "./a.js" : "./b.js")', {
    ...offline,
    resourceUrl: 'https://example.test/root.js',
    resourceMappings: {
      'https://example.test/a.js': source('window.a=1'),
      'https://example.test/b.js': source('window.b=1'),
    },
  });
  assert.equal(manifest.status, 'completed');
  assert.equal(manifest.sources[0].url, 'https://example.test/root.js');
  assert.equal(manifest.findings.length, 2);
});

test('parse, resolution, access and resource budget failures stay visible', async () => {
  assert.equal((await scanJs('const = ;', offline)).status, 'failed');
  const missing = await scanJs('import "unmapped"; import "./missing.js";', {
    ...offline,
    resourceUrl: 'https://example.test/a.js',
  });
  assert.equal(missing.status, 'partial');
  assert.equal(missing.summary.unresolved, 2);
  const bounded = await scanHtml('<script>window.a=1</script><script>window.b=1</script>', {
    ...offline,
    budgets: { maxResources: 2 },
  });
  assert.equal(bounded.sources.length, 2);
  assert.ok(bounded.dependencies.some((d) => d.reason === 'resource-count-budget'));
  await assert.rejects(scan([], offline), /at-least-one-entry/);
  await assert.rejects(scanJs('', { budgets: { concurrency: 0 } }), /invalid-budget/);
});

test('source maps resolve from offline content and attach original positions', async () => {
  const manifest = await scanJs('window.x=1;\n//# sourceMappingURL=a.js.map', {
    ...offline,
    resourceUrl: 'https://example.test/a.js',
    sourceMaps: true,
    resourceMappings: {
      'https://example.test/a.js.map': source(
        JSON.stringify({ version: 3, sources: ['original.ts'], names: [], mappings: 'AAGE' }),
      ),
    },
  });
  assert.deepEqual(manifest.findings[0].location.original, {
    source: 'original.ts',
    line: 4,
    column: 2,
  });
});

test('redirected network entry resolves child imports against its final URL', async (t) => {
  const { url } = await server(t, (req, res) => {
    if (req.url === '/start') {
      res.writeHead(301, { location: '/dir/main.js' });
      res.end();
    } else if (req.url === '/dir/main.js') res.end('import "./child.js";');
    else if (req.url === '/dir/child.js') res.end('window.child=1');
    else {
      res.statusCode = 404;
      res.end();
    }
  });
  const manifest = await scanJs({ type: 'url', url: `${url}/start` }, { scriptMode: 'module' });
  assert.equal(manifest.status, 'completed');
  assert.equal(manifest.findings[0].target, 'window.child');
});

test('AbortSignal cancellation detaches listeners after completion and failure', async () => {
  const controller = new AbortController();
  await scanJs('window.x=1', { ...offline, signal: controller.signal });
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  await assert.rejects(scan([], { signal: controller.signal }));
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  controller.abort();
  const cancelled = await scanJs('window.x=1', { ...offline, signal: controller.signal });
  assert.equal(cancelled.status, 'cancelled');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('parallel SDK calls keep state isolated', async () => {
  const manifests = await Promise.all(
    Array.from({ length: 12 }, (_, i) => scanJs(`window.value${i}=1`, offline)),
  );
  manifests.forEach((m, i) =>
    assert.deepEqual(
      m.findings.map((f) => f.target),
      [`window.value${i}`],
    ),
  );
});

test('cancellation during an HTTP read returns a cancelled manifest', async (t) => {
  const controller = new AbortController();
  const { url } = await server(t, (_req, res) => {
    res.writeHead(200, { 'content-type': 'text/javascript' });
    res.write('window.x=1;');
    controller.abort();
    setTimeout(() => res.end('window.y=2;'), 20);
  });
  const manifest = await scanJs(
    { type: 'url', url },
    {
      scriptMode: 'module',
      signal: controller.signal,
      budgets: { resourceTimeoutMs: 1000 },
    },
  );
  assert.equal(manifest.status, 'cancelled');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

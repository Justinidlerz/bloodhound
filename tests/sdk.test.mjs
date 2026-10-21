import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';
import {
  scan,
  scanJs,
  scanHtml,
  scanResource,
  resumeScan,
  validateReport,
  compactReport,
  engineVersion,
  defaults,
  GlobalIndex,
  diffReports,
  indexArtifact,
  adaptReport,
  runtimeCandidates,
} from '@bloodhound-rs/core';

const context = {
  reportFormat: 'full',
  scriptMode: 'module',
  executionEnvironment: 'window',
  scopeKey: 'sdk-test',
};
const source = (content, resourceUrl = 'https://example.test/app.js') => ({
  type: 'content',
  content,
  resourceUrl,
});
const entry = (kind, code, url) => ({ kind, source: source(code, url) });
async function temporary(t) {
  const path = await mkdtemp(join(tmpdir(), 'bloodhound-sdk-'));
  t.after(() => rm(path, { recursive: true, force: true }));
  return path;
}
async function server(t, handler) {
  const instance = createServer(handler);
  instance.listen(0, '127.0.0.1');
  await once(instance, 'listening');
  t.after(async () => {
    instance.closeAllConnections();
    await new Promise((resolve) => instance.close(resolve));
  });
  return {
    instance,
    url: `http://127.0.0.1:${instance.address().port}`,
    host: `127.0.0.1:${instance.address().port}`,
  };
}
const target = (report, name) =>
  report.globalTargets.find(
    (item) => item.namespace === 'global-object' && item.pathSegments.join('.') === name,
  );

test('SDK exports compiled versions and isolated default objects', () => {
  const first = defaults();
  assert.equal(first.versions.parser, engineVersion());
  first.budgets.maxResources = 0;
  assert.ok(defaults().budgets.maxResources > 0);
});
test('scanJs accepts text and explicit file, URL, and content sources with UTF-16 positions', async (t) => {
  const directory = await temporary(t);
  const file = join(directory, 'space name.js');
  const code = '// 😀\nglobalThis.shared = 1;';
  await writeFile(file, code);
  for (const input of [
    code,
    { type: 'file', path: file },
    { type: 'url', url: pathToFileURL(file).href },
    source(code),
  ]) {
    const report = await scanJs(input, context);
    validateReport(report);
    assert.equal(report.scanStatus, 'completed');
    assert.ok(target(report, 'shared'));
    const access = report.globalAccesses.find((a) => a.operation === 'write');
    assert.equal(access.location.start, 6);
    assert.equal(access.location.line, 2);
  }
});
test('logical resourceUrl overrides a local source address', async (t) => {
  const directory = await temporary(t);
  const file = join(directory, 'a.js');
  await writeFile(file, 'globalThis.x=1');
  const report = await scanJs(
    { type: 'file', path: file },
    {
      ...context,
      resourceUrl: 'https://example.test/release/a.js',
    },
  );
  assert.equal(report.resources[0].finalUrl, 'https://example.test/release/a.js');
});
test('scanResource enforces single-resource scope even if recursive is requested', async () => {
  const report = await scanResource(entry('js', "import './missing.js';globalThis.x=1"), {
    ...context,
    dependencyMode: 'recursive',
  });
  assert.equal(report.configuration.dependencyMode, 'single');
  assert.equal(report.references[0].resolution, 'not-followed');
  assert.equal(report.scanStatus, 'completed');
});
test('scanHtml reads base, import maps, script modes, styles, and inline locations', async () => {
  const html =
    '<!doctype html><base href="https://example.test/assets/"><script type="importmap">{"imports":{"config":"./config.js"}}</script><script type="module">import {key} from "config";globalThis[key]=1;</script><style>body{--theme:red}</style>';
  const report = await scanHtml(source(html, 'https://example.test/page.html'), {
    reportFormat: 'full',
    resourceMappings: {
      'https://example.test/assets/config.js': source(
        'export const key="shared"',
        'https://example.test/assets/config.js',
      ),
    },
  });
  assert.equal(report.configuration.context.executionEnvironment, 'window');
  assert.ok(target(report, 'shared'));
  assert.ok(report.findings.some((finding) => finding.ruleId === 'custom-property'));
  assert.ok(report.globalAccesses.some((a) => a.location.container));
  assert.equal(report.scanStatus, 'completed');
});
test('scan accepts multiple language entries and reports independent parse failures', async () => {
  const report = await scan(
    [
      entry('js', 'const = ;', 'https://example.test/bad.js'),
      entry('css', 'body{all:unset}', 'https://example.test/style.css'),
      entry('html', '<script>globalThis.shared=1</script>', 'https://example.test/page.html'),
    ],
    context,
  );
  validateReport(report);
  assert.equal(report.scanStatus, 'partial');
  assert.equal(report.coverage.failed, 1);
  assert.ok(target(report, 'shared'));
  assert.ok(report.findings.some((f) => f.kind === 'css'));
});
test('invalid SDK requests reject instead of crashing or producing complete reports', async () => {
  for (const run of [
    () => scan([], context),
    () => scanJs(source(''), { ...context, scriptMode: 'invalid' }),
    () => scanJs(source(''), { ...context, budgets: { concurrency: 0 } }),
    () => scanResource(entry('unknown', ''), context),
    () => resumeScan({ checkpointVersion: 99 }, context),
  ]) {
    await assert.rejects(async () => run());
  }
  assert.throws(() => validateReport({}), /invalid-report/);
  assert.throws(() => new GlobalIndex({}), /invalid-report/);
});
test('GlobalIndex methods, report adapters, and runtime candidates work through napi', async () => {
  const report = await scanJs(source('globalThis.shared=1;globalThis.shared="s";shared;'), context);
  const index = new GlobalIndex(report);
  const [item] = index.findTargets(['shared'], 'global-object', context.scopeKey);
  assert.ok(item);
  assert.equal(index.sources(item.targetId).length, 2);
  assert.ok(index.accesses(report.analysisInstances[0].analysisId).length > 0);
  assert.deepEqual(
    index.resourceAccesses(report.resources[0].resourceId),
    index.accesses(report.analysisInstances[0].analysisId),
  );
  assert.equal(index.entryPaths(item.targetId, { limit: 1 }).paths.length, 1);
  assert.equal(index.conflicts()[0].certainty, 'static-candidate');
  assert.ok(index.resolveUnbound().some((u) => u.candidates.includes(item.targetId)));
  const copy = index.report;
  copy.globalTargets.length = 0;
  assert.ok(index.report.globalTargets.length > 0);
  assert.equal(indexArtifact(report, 'external').reportId, 'external');
  assert.equal(adaptReport(report, 'build', { release: 'r1' }).scope.release, 'r1');
  assert.equal(adaptReport(report, 'page').reportType, 'page');
  assert.ok(
    runtimeCandidates(report, { pathSegments: ['shared'] }).every(
      (a) => a.attribution === 'static_candidate',
    ),
  );
  assert.throws(() => index.entryPaths(item.targetId, []), /must-be-object/);
  assert.throws(() => adaptReport(report, 'invalid'), /invalid-report-type/);
});
test('diffReports compares versions without treating shifted lines as new accesses', async () => {
  const before = await scanJs(source('globalThis.x=1'), context);
  const after = await scanJs(source('\n\nglobalThis.x=1'), context);
  assert.deepEqual(diffReports(before, after), { added: [], removed: [], changed: [] });
  const changed = await scanJs(source('globalThis.x="s"'), context);
  assert.equal(diffReports(before, changed).changed.length, 1);
});
test('report, JSONL, and checkpoint files are usable by SDK consumers', async (t) => {
  const directory = await temporary(t);
  const checkpointPath = join(directory, 'checkpoint.json');
  const reportPath = join(directory, 'report.json');
  const jsonlPath = join(directory, 'report.jsonl');
  const report = await scanJs(source('globalThis.x=1'), {
    ...context,
    checkpointPath,
    reportPath,
    jsonlPath,
  });
  assert.deepEqual(JSON.parse(await readFile(reportPath, 'utf8')), report);
  assert.equal(
    JSON.parse((await readFile(jsonlPath, 'utf8')).trim().split('\n').at(-1)).type,
    'summary',
  );
  const checkpoint = JSON.parse(await readFile(checkpointPath, 'utf8'));
  const resumed = await resumeScan(checkpoint, { reportFormat: 'full' });
  validateReport(resumed);
  assert.ok(target(resumed, 'x'));
  await assert.rejects(resumeScan(checkpoint, { scopeKey: 'different' }), /configuration-mismatch/);
});
test('pre-aborted signals preserve existing handlers and remove SDK listeners', async () => {
  const controller = new AbortController();
  let priorCalls = 0;
  const handler = () => {
    priorCalls++;
  };
  controller.signal.onabort = handler;
  controller.abort();
  const report = await scanJs(source('globalThis.x=1'), { ...context, signal: controller.signal });
  assert.equal(report.runStatus, 'cancelled');
  assert.equal(controller.signal.onabort, handler);
  assert.equal(priorCalls, 1);
});
test(
  'in-flight cancellation is isolated and removes its event listener',
  { timeout: 10000 },
  async (t) => {
    let respond;
    let received;
    const ready = new Promise((resolve) => {
      received = resolve;
    });
    const remote = await server(t, (request, response) => {
      respond = () => response.end('globalThis.cancelled=1');
      received();
    });
    const controller = new AbortController();
    let added = 0;
    let removed = 0;
    const signal = controller.signal;
    const add = signal.addEventListener.bind(signal);
    const remove = signal.removeEventListener.bind(signal);
    signal.addEventListener = (...args) => {
      added++;
      return add(...args);
    };
    signal.removeEventListener = (...args) => {
      removed++;
      return remove(...args);
    };
    const pending = scanJs(
      { type: 'url', url: remote.url },
      {
        ...context,
        signal,
        budgets: { retries: 0, resourceTimeoutMs: 2000 },
      },
    );
    await ready;
    controller.abort();
    respond();
    const report = await pending;
    assert.equal(report.runStatus, 'cancelled');
    assert.equal(added, 1);
    assert.equal(removed, 1);
    assert.equal((await scanJs(source('globalThis.independent=1'), context)).runStatus, 'finished');
  },
);
test(
  'HTTP redirects recheck access and do not forward another host credentials',
  { timeout: 10000 },
  async (t) => {
    let destinationHeaders;
    const destination = await server(t, (req, res) => {
      destinationHeaders = req.headers;
      res.end('globalThis.redirected=1');
    });
    let initialHeader;
    const origin = await server(t, (req, res) => {
      initialHeader = req.headers.authorization;
      res.writeHead(302, { location: `${destination.url}/target.js` });
      res.end();
    });
    const options = {
      ...context,
      requestHeaders: { [origin.host]: { Authorization: 'test-only-token' } },
      budgets: { retries: 0 },
    };
    const report = await scanJs({ type: 'url', url: origin.url }, options);
    assert.equal(initialHeader, 'test-only-token');
    assert.equal(destinationHeaders.authorization, undefined);
    assert.equal(report.resources[0].finalUrl, `${destination.url}/target.js`);
    assert.ok(target(report, 'redirected'));
    assert.ok(!JSON.stringify(report).includes('test-only-token'));
    destinationHeaders = undefined;
    const denied = await scanJs(
      { type: 'url', url: origin.url },
      { ...options, access: { hosts: [origin.host] } },
    );
    assert.equal(denied.scanStatus, 'failed');
    assert.ok(denied.unresolved.some((u) => u.detail?.includes('host-not-allowed')));
    assert.equal(destinationHeaders, undefined);
  },
);
test(
  'HTTP retries, redirect limits, byte limits, and timeouts are bounded',
  { timeout: 10000 },
  async (t) => {
    let retries = 0;
    const remote = await server(t, (req, res) => {
      if (req.url === '/retry' && retries++ === 0) {
        res.writeHead(503);
        res.end();
      } else if (req.url === '/loop') {
        res.writeHead(302, { location: '/loop' });
        res.end();
      } else if (req.url === '/slow') {
        /* Closed by test cleanup after the SDK timeout. */
      } else {
        res.end('globalThis.loaded=1');
      }
    });
    assert.equal(
      (
        await scanJs(
          { type: 'url', url: `${remote.url}/retry` },
          { ...context, budgets: { retries: 1 } },
        )
      ).scanStatus,
      'completed',
    );
    assert.equal(retries, 2);
    for (const [path, budgets, detail] of [
      ['/loop', { retries: 0, maxRedirects: 1 }, 'redirect-budget'],
      ['/large', { retries: 0, maxResourceBytes: 1 }, 'resource-byte-budget'],
      ['/slow', { retries: 0, resourceTimeoutMs: 50, timeoutMs: 2000 }, 'resource-load-failed'],
    ]) {
      const report = await scanJs(
        { type: 'url', url: `${remote.url}${path}` },
        { ...context, budgets },
      );
      assert.equal(report.scanStatus, 'failed');
      assert.ok(
        report.unresolved.some((u) => u.reason.includes(detail) || u.detail?.includes(detail)),
      );
    }
  },
);
test(
  'explicit HTTP proxy routes requests without persisting proxy credentials',
  { timeout: 10000 },
  async (t) => {
    let requested;
    const proxy = await server(t, (req, res) => {
      requested = req.url;
      res.end('globalThis.proxied=1');
    });
    const report = await scanJs(
      { type: 'url', url: 'http://example.test/proxy.js' },
      {
        ...context,
        proxy: proxy.url,
        budgets: { retries: 0 },
      },
    );
    assert.equal(requested, 'http://example.test/proxy.js');
    assert.ok(target(report, 'proxied'));
    assert.equal(report.configuration.proxy, undefined);
  },
);

test('large reports round-trip through indexing, queries, adapters, and checkpoints', async (t) => {
  const directory = await temporary(t);
  const checkpointPath = join(directory, 'checkpoint.json');
  const report = await scanJs(source('globalThis.shared=1'), { ...context, checkpointPath });
  const sample = report.globalAccesses.find((access) => access.operation === 'write');
  const nodes = (value) =>
    1 +
    (value && typeof value === 'object'
      ? Object.values(value).reduce((n, v) => n + nodes(v), 0)
      : 0);
  // Reproduce napi-rs' aggregate conversion limit with ordinary independent records.
  const copies = Math.ceil(1_000_000 / nodes(sample));
  for (let i = 0; i < copies; i++) {
    report.globalAccesses.push({ ...structuredClone(sample), accessId: `large-access-${i}` });
  }
  assert.ok(nodes(report) > 1_000_000);
  validateReport(report);
  const index = new GlobalIndex(report);
  const [shared] = index.findTargets(['shared'], 'global-object');
  const count = report.globalAccesses.length;
  assert.equal(index.report.globalAccesses.length, count);
  assert.equal(index.sources(shared.targetId).length, copies + 1);
  assert.equal(index.accesses(sample.analysisId).length, count);
  assert.equal(index.resourceAccesses(report.resources[0].resourceId).length, count);
  assert.equal(index.conflicts()[0].accessIds.length, copies + 1);
  assert.equal(indexArtifact(report).globalAccesses.length, count);
  assert.equal(adaptReport(report, 'page').reportType, 'page');
  assert.equal(runtimeCandidates(report, {}).length, count);
  const diff = diffReports(report, report);
  assert.equal(diff.changed.length, 0);
  assert.equal(diff.added.length, copies + 1);
  assert.equal(diff.removed.length, copies + 1);
  const checkpoint = JSON.parse(await readFile(checkpointPath, 'utf8'));
  checkpoint.report = report;
  for (const resumed of [
    await resumeScan(checkpoint, { reportFormat: 'full' }),
    await scan([], { resume: checkpoint, reportFormat: 'full' }),
  ]) {
    validateReport(resumed);
    assert.ok(target(resumed, 'shared'));
  }
});

test('report conversion still rejects cyclic, deeply nested, and malformed records', async () => {
  const base = await scanJs(source('globalThis.shared=1'), context);
  const cyclic = {};
  cyclic.self = cyclic;
  const deep = {};
  let cursor = deep;
  for (let i = 0; i < 150; i++) cursor = cursor.child = {};
  // A small shared-object graph must not expand exponentially during conversion.
  let shared = {};
  for (let i = 0; i < 22; i++) shared = { left: shared, right: shared };
  for (const [value, pattern] of [
    [cyclic, /maximum JSON nesting depth/],
    [deep, /maximum JSON nesting depth/],
    [shared, /maximum JSON value count/],
  ]) {
    const report = { ...base, globalAccesses: [value] };
    assert.throws(() => new GlobalIndex(report), pattern);
    assert.throws(() => validateReport(report), pattern);
  }
  for (const globalAccesses of [{}, 'invalid', [null]]) {
    assert.throws(() => new GlobalIndex({ ...base, globalAccesses }));
  }
});

test('text input returns a compact manifest by default, with optional full indexing', async () => {
  const code = 'globalThis.history.replaceState = function() {}; globalThis.history.replaceState;';
  const options = { scriptMode: 'classic', executionEnvironment: 'window' };
  const manifest = await scanJs(code, options);
  assert.equal(manifest.format, 'compact');
  assert.equal(manifest.scanStatus, 'completed');
  assert.equal(manifest.globals.length, 1);
  assert.deepEqual(manifest.globals[0].targets[0].path, ['history', 'replaceState']);
  assert.equal(manifest.globals[0].valueType, 'function');
  assert.ok(manifest.summary.omittedReadAccesses > 0);
  assert.equal(manifest.sources[0].url, undefined);
  const report = await scanJs(code, { ...options, reportFormat: 'full' });
  validateReport(report);
  const projected = compactReport(report);
  assert.deepEqual(projected.globals, manifest.globals);
  assert.deepEqual(projected.sources, manifest.sources);
  assert.ok(new GlobalIndex(report).findTargets(['history', 'replaceState']).length);
  assert.doesNotMatch(JSON.stringify(manifest), /contentHash|scopeKey|fingerprint|memory:|file:/);
  await assert.rejects(scanJs(code, { reportFormat: 'invalid' }), /invalid-report-format/);
});

test('HTML text needs no address for inline code and flags relative dependencies without a base', async () => {
  const manifest = await scanHtml(
    '<script>window.inlineValue=1</script><script src="./relative.js"></script>',
  );
  assert.equal(manifest.scanStatus, 'partial');
  assert.ok(manifest.globals.some((g) => g.targets.some((t) => t.path[0] === 'inlineValue')));
  assert.ok(manifest.dependencies.some((d) => d.reason === 'missing-base-url'));
  assert.equal(manifest.sources[0].url, undefined);
  assert.ok(manifest.globals[0].location.container);
  assert.doesNotMatch(JSON.stringify(manifest), /memory:|file:/);
});

test('HTML text resolves automatic import maps with absolute bases or supplied document URLs', async () => {
  const dependency = 'https://example.test/assets/config.js';
  for (const [base, options] of [
    ['https://example.test/assets/', {}],
    ['./assets/', { resourceUrl: 'https://example.test/page.html' }],
  ]) {
    const manifest = await scanHtml(
      `<base href="${base}"><script type="importmap">{"imports":{"config":"./config.js"}}</script><script type="module">import {key} from "config";window[key]=1</script>`,
      {
        ...options,
        resourceMappings: {
          [dependency]: { type: 'content', content: 'export const key="shared"' },
        },
      },
    );
    assert.equal(manifest.scanStatus, 'completed');
    assert.ok(manifest.sources.some((s) => s.url === dependency));
    assert.ok(manifest.globals.some((g) => g.targets.some((t) => t.path[0] === 'shared')));
  }
});

test('stdin example emits only one JSON document and leaves file persistence to its caller', async (t) => {
  const directory = await temporary(t);
  const result = spawnSync(
    process.execPath,
    [fileURLToPath(new URL('../examples/scan-stdin.mjs', import.meta.url))],
    { cwd: directory, input: '<script>window.example=1</script>', encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, '');
  const manifest = JSON.parse(result.stdout);
  assert.equal(manifest.format, 'compact');
  assert.equal(manifest.globals[0].targets[0].path[0], 'example');
  assert.equal(result.stdout.trim().split('\n').length, 1);
  assert.deepEqual(await readdir(directory), []);
});

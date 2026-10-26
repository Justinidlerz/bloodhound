import assert from 'node:assert/strict';
import { mkdtemp, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { rolldown } from 'rolldown';
import { rollup } from 'rollup';
import { bloodhound, diagnosticsFromManifest } from '../dist/index.js';

const MANIFEST = 'bloodhound.manifest.json';

async function project(t) {
  // Rolldown resolves symlinked temp directories, so compare against the real path.
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'bloodhound-plugin-')));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await writeFile(
    join(dir, 'main.js'),
    [
      "import { mount } from './widget.js';",
      "window.__APP_VERSION__ = '1.0.0';",
      "globalThis.sharedConfig = { endpoint: '/api' };",
      'mount();',
      "if (location.search) import('./lazy.js').then((m) => m.start());",
      '',
    ].join('\n'),
  );
  await writeFile(
    join(dir, 'widget.js'),
    "export function mount() { document.title = 'ready'; }\n",
  );
  await writeFile(join(dir, 'lazy.js'), 'export function start() { delete window.__legacy; }\n');
  return dir;
}

function emitHtml() {
  return {
    name: 'test-html',
    generateBundle(_options, bundle) {
      const entry = Object.values(bundle).find((item) => item.type === 'chunk' && item.isEntry);
      this.emitFile({
        type: 'asset',
        fileName: 'index.html',
        source: `<!doctype html><style>:root{--x:1}</style><script>window.__BOOT__=1</script><script type="module" src="/${entry.fileName}"></script>`,
      });
    },
  };
}

/** Builds in memory with either bundler and collects warnings through `onLog`. */
const bundlers = {
  rollup: async ({ dir, plugins, output }) => {
    const logs = [];
    const build = await rollup({
      input: join(dir, 'main.js'),
      plugins,
      onLog: (level, log) => logs.push({ level, log }),
    });
    const result = await build.generate({ format: 'es', dir: join(dir, 'dist'), ...output });
    await build.close();
    return { output: result.output, logs };
  },
  rolldown: async ({ dir, plugins, output }) => {
    const logs = [];
    const build = await rolldown({
      input: join(dir, 'main.js'),
      plugins,
      onLog: (level, log) => logs.push({ level, log }),
    });
    const result = await build.generate({ format: 'es', dir: join(dir, 'dist'), ...output });
    await build.close();
    return { output: result.output, logs };
  },
};

const warnings = (logs) =>
  logs
    .filter(({ level, log }) => level === 'warn' && (log.plugin === 'bloodhound' || !log.plugin))
    .map(({ log }) => log);
const manifestOf = (output) => JSON.parse(output.find((item) => item.fileName === MANIFEST).source);

for (const [name, run] of Object.entries(bundlers)) {
  test(`${name}: per-rule severity overrides category settings`, async (t) => {
    const dir = await project(t);
    await writeFile(join(dir, 'main.js'), 'window.fetch = () => Promise.resolve();');
    await assert.rejects(
      run({
        dir,
        plugins: [
          bloodhound({
            globals: 'off',
            rules: { 'host-api-override': 'error' },
          }),
        ],
      }),
      /host-api-override/,
    );
  });

  test(`${name}: scans entry chunks, emits the manifest, and maps warnings to source files`, async (t) => {
    const dir = await project(t);
    let seen;
    const { output, logs } = await run({
      dir,
      output: { sourcemap: 'hidden' },
      plugins: [
        bloodhound({
          allowGlobals: ['__APP_VERSION__'],
          onManifest: (manifest, context) => {
            seen = { manifest, diagnostics: context.diagnostics };
          },
        }),
      ],
    });
    const manifest = manifestOf(output);
    assert.equal(manifest.schemaVersion, 2);
    assert.equal(manifest.sources.filter((s) => s.kind === 'js').length, 2, 'entry + lazy chunk');
    assert.ok(manifest.findings.every((f) => f.location.original));
    assert.deepEqual(seen.manifest, manifest);
    const globals = seen.diagnostics.filter((d) => d.kind === 'global');
    assert.deepEqual(globals.map((d) => `${d.rule} ${d.targets.join(',')}`).sort(), [
      'global-write window.__legacy',
      'global-write window.sharedConfig',
    ]);
    const sharedConfig = globals.find((d) => d.targets.includes('window.sharedConfig'));
    assert.equal(sharedConfig.original.line, 3);
    const warned = warnings(logs);
    const log = warned.find((item) => item.message.includes('sharedConfig'));
    assert.ok(log, JSON.stringify(warned));
    assert.equal(log.loc.file, join(dir, 'main.js'));
    assert.equal(log.loc.line, 3);
    assert.equal(log.loc.column, 0);
    assert.ok(!warned.some((item) => item.message.includes('__APP_VERSION__')));
    assert.ok(
      output.find((item) => item.fileName.endsWith('.js.map')),
      'hidden maps still emitted',
    );
    assert.ok(!output.find((item) => item.type === 'chunk' && /sourceMappingURL/.test(item.code)));
  });

  test(`${name}: scans emitted HTML as the entry and locates inline findings in the document`, async (t) => {
    const dir = await project(t);
    const { output, logs } = await run({
      dir,
      output: { sourcemap: true },
      plugins: [emitHtml(), bloodhound({ publicPath: 'https://cdn.example/', findings: 'warn' })],
    });
    const manifest = manifestOf(output);
    assert.deepEqual(manifest.entries, ['s1']);
    assert.equal(manifest.sources[0].url, 'https://cdn.example/index.html');
    assert.ok(
      manifest.dependencies.some((d) => d.kind === 'style' && d.reason === 'css-not-analyzed'),
    );
    assert.ok(
      manifest.dependencies.every((d) => d.status !== 'unresolved'),
      JSON.stringify(manifest.dependencies),
    );
    const boot = warnings(logs).find((item) => item.message.includes('__BOOT__'));
    assert.deepEqual(boot.loc, { file: 'index.html', line: 1, column: 50 });
    const css = warnings(logs).find((item) => item.message.includes('custom-property'));
    assert.equal(css, undefined);
  });

  test(`${name}: error severity fails the build after the manifest is emitted`, async (t) => {
    const dir = await project(t);
    let emitted = false;
    await assert.rejects(
      run({
        dir,
        plugins: [
          bloodhound({
            globals: 'error',
            allowGlobals: ['__APP_VERSION__', 'document.*'],
            onManifest: () => {
              emitted = true;
            },
          }),
        ],
      }),
      (error) => {
        assert.match(error.message, /2 blocking issue\(s\)/);
        assert.match(error.message, /sharedConfig/);
        assert.match(error.message, /__legacy/);
        return true;
      },
    );
    assert.ok(emitted);
  });

  test(`${name}: manifest can be disabled and references outside the bundle are not fetched`, async (t) => {
    const dir = await project(t);
    await writeFile(join(dir, 'main.js'), "import 'https://cdn.example/vendor.js';\nwindow.a=1;\n");
    const { output, logs } = await run({
      dir,
      plugins: [
        {
          name: 'externalize',
          resolveId: (id) => (id.startsWith('https:') ? { id, external: true } : null),
        },
        bloodhound({ manifestFileName: false, globals: 'off', coverage: 'warn' }),
      ],
    });
    assert.ok(!output.find((item) => item.fileName === MANIFEST));
    const coverage = warnings(logs).find((item) => /scan partial/.test(item.message));
    assert.match(coverage.message, /load-failed/);
  });
}

test('diagnosticsFromManifest applies allow, ignore, and filter rules', () => {
  const manifest = {
    sources: [{ id: 's1' }],
    findings: [
      {
        source: 's1',
        rule: 'global-write',
        target: 'window.dataLayer',
        operation: 'write',
        location: { line: 1, column: 0, endColumn: 1 },
      },
      {
        source: 's1',
        rule: 'global-write',
        target: 'window.vendor',
        operation: 'write',
        location: {
          line: 2,
          column: 0,
          endColumn: 1,
          original: { source: '../node_modules/x.js', line: 1, column: 0 },
        },
      },
      {
        source: 's1',
        rule: 'global-write',
        target: 'window.app.state',
        operation: 'define',
        location: { line: 3, column: 0, endColumn: 1 },
      },
      {
        source: 's1',
        rule: 'dom-insertion',
        target: 'document.body',
        operation: 'insert',
        location: { line: 4, column: 0, endColumn: 1 },
      },
    ],
  };
  const all = diagnosticsFromManifest(manifest, { ignoreSources: [] }, new Map([['s1', 'a.js']]));
  assert.deepEqual(
    all.map((d) => d.rule),
    ['global-write', 'global-write', 'global-write', 'dom-insertion'],
  );
  assert.equal(all[0].fileName, 'a.js');
  const filtered = diagnosticsFromManifest(manifest, {
    allowGlobals: ['dataLayer*', /^app\./],
    ignoreSources: [],
    filter: (d) => d.kind === 'global',
  });
  assert.deepEqual(
    filtered.map((d) => d.targets.join('.')),
    ['window.vendor'],
  );
  assert.deepEqual(
    diagnosticsFromManifest(manifest, {
      ignoreSources: ['*node_modules*'],
      filter: (d) => d.kind === 'global',
    }).map((d) => d.targets[0]),
    ['window.dataLayer', 'window.app.state'],
  );
});

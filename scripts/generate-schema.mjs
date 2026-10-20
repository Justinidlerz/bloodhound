import { writeFileSync } from 'node:fs';
const str = { type: 'string' },
  num = { type: 'number' },
  strings = { type: 'array', items: str },
  obj = { type: 'object' };
const record = (properties, required = Object.keys(properties)) => ({
  type: 'object',
  properties,
  required,
  additionalProperties: false,
});
const arr = (items) => ({ type: 'array', items });
const ref = (n) => ({ $ref: `#/$defs/${n}` });
const defs = {
  location: record(
    {
      start: num,
      end: num,
      line: num,
      column: num,
      endLine: num,
      endColumn: num,
      offsetEncoding: { const: 'utf16' },
      original: record({ source: str, line: num, column: num, name: str }, [
        'source',
        'line',
        'column',
      ]),
      container: record(
        { resourceUrl: str, offset: num, line: num, column: num, approximate: { type: 'boolean' } },
        ['resourceUrl', 'offset', 'line', 'column'],
      ),
    },
    ['start', 'end', 'line', 'column', 'endLine', 'endColumn', 'offsetEncoding'],
  ),
  context: record(
    {
      scriptMode: { enum: ['classic', 'module', 'unknown'] },
      executionEnvironment: { enum: ['window', 'worker', 'unknown'] },
      scopeKey: str,
      versions: { type: 'object', additionalProperties: str },
      baseUrl: str,
      importMap: obj,
      importMapBaseUrl: str,
      publicPath: str,
    },
    ['scriptMode', 'executionEnvironment', 'scopeKey', 'versions'],
  ),
  resource: record(
    {
      resourceId: str,
      requestedUrl: str,
      finalUrl: str,
      contentHash: str,
      kind: { enum: ['js', 'html', 'css', 'asset'] },
      bytes: num,
      scmId: str,
      inline: record({ parentUrl: str, offset: num, approximate: { type: 'boolean' } }, [
        'parentUrl',
        'offset',
      ]),
    },
    ['resourceId', 'requestedUrl', 'finalUrl', 'contentHash', 'kind', 'bytes'],
  ),
  analysis: record({
    analysisId: str,
    resourceId: str,
    contextHash: str,
    context: ref('context'),
    scanStatus: { enum: ['completed', 'partial', 'failed'] },
    exports: obj,
    sourceMapStatus: { enum: ['absent', 'mapped', 'unavailable'] },
  }),
  reference: record(
    {
      referenceId: str,
      fromAnalysisId: str,
      toAnalysisIds: strings,
      kind: str,
      specifier: str,
      resolvedUrl: str,
      resolution: { enum: ['resolved', 'candidate', 'unresolved', 'not-followed'] },
      location: ref('location'),
      condition: str,
      attributes: { type: 'object', additionalProperties: str },
      reason: str,
    },
    ['referenceId', 'fromAnalysisId', 'toAnalysisIds', 'kind', 'resolution', 'location'],
  ),
  target: record({
    targetId: str,
    scopeKey: str,
    namespace: str,
    pathSegments: strings,
    keyKind: str,
  }),
  access: record({
    accessId: str,
    analysisId: str,
    targetRefs: strings,
    operation: str,
    valueType: str,
    location: ref('location'),
    evidence: strings,
    fingerprint: str,
  }),
  finding: record({
    findingId: str,
    ruleId: str,
    kind: { enum: ['global', 'dom', 'css'] },
    analysisId: str,
    accessRefs: strings,
    location: ref('location'),
    details: obj,
    fingerprint: str,
  }),
  unresolved: record(
    {
      analysisId: str,
      referenceId: str,
      location: ref('location'),
      reason: str,
      affectedCapabilities: strings,
      detail: str,
      start: num,
      end: num,
    },
    ['reason', 'affectedCapabilities'],
  ),
};
const collections = {
  resources: arr(ref('resource')),
  analysisInstances: arr(ref('analysis')),
  references: arr(ref('reference')),
  globalTargets: arr(ref('target')),
  globalAccesses: arr(ref('access')),
  findings: arr(ref('finding')),
  unresolved: arr(ref('unresolved')),
};
const report = {
  $schema: 'http://json-schema.org/draft-07/schema#',
  $id: 'https://bloodhound.dev/schemas/report-v1.json',
  ...record({
    schemaVersion: { const: 1 },
    indexSchemaVersion: { const: 1 },
    scannerVersion: str,
    scanId: str,
    scopeKey: str,
    scanStatus: { enum: ['completed', 'partial', 'failed'] },
    runStatus: { enum: ['finished', 'cancelled'] },
    startedAt: str,
    finishedAt: str,
    configuration: record({
      context: ref('context'),
      budgets: obj,
      dependencyMode: { enum: ['single', 'recursive'] },
    }),
    entryAnalysisIds: strings,
    ...collections,
    coverage: record({
      analyzed: num,
      failed: num,
      unresolved: num,
      bytes: num,
      dependencyMode: { enum: ['single', 'recursive'] },
      sourceMapped: num,
      elapsedMs: num,
    }),
  }),
  $defs: defs,
};
const index = {
  $schema: report.$schema,
  $id: 'https://bloodhound.dev/schemas/index-v1.json',
  ...record({
    indexSchemaVersion: { const: 1 },
    reportId: str,
    scopeKey: str,
    entryAnalysisIds: strings,
    ...collections,
  }),
  $defs: defs,
};
writeFileSync('schemas/report.schema.json', JSON.stringify(report, null, 2) + '\n');
writeFileSync('schemas/index.schema.json', JSON.stringify(index, null, 2) + '\n');

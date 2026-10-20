import { fileURLToPath } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { scanJs, GlobalIndex } from '@bloodhound-rs/core';

const inputPath = fileURLToPath(new URL('./fixtures/app.js', import.meta.url));
const outputDirectory = new URL('../outputs/example/', import.meta.url);
await mkdir(outputDirectory, { recursive: true });
const reportPath = fileURLToPath(new URL('report.json', outputDirectory));

const report = await scanJs(inputPath, {
  scriptMode: 'classic',
  executionEnvironment: 'window',
  dependencyMode: 'single',
  reportPath,
});

console.log('扫描状态：', report.scanStatus);
console.log('报告路径：', reportPath);
console.log('候选问题：', report.findings);
console.log('覆盖缺口：', report.unresolved);

const index = new GlobalIndex(report);
for (const target of index.findTargets(['sharedConfig'], 'global-object')) {
  const accesses = index.sources(target.targetId).map(({ access, resource }) => ({
    operation: access.operation,
    valueType: access.valueType,
    resourceUrl: resource.finalUrl,
    line: access.location.line,
    column: access.location.column,
  }));
  console.log('sharedConfig 的读写位置（行号从 1 开始，列号从 0 开始）：');
  console.table(accesses);
}
console.log('冲突候选：', index.conflicts());

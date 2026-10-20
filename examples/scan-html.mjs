import { fileURLToPath } from 'node:url';
import { mkdir } from 'node:fs/promises';
import { scanHtml, GlobalIndex } from '@bloodhound/core';

const inputPath = fileURLToPath(new URL('./fixtures/html/index.html', import.meta.url));
const outputDirectory = new URL('../outputs/example-html/', import.meta.url);
await mkdir(outputDirectory, { recursive: true });
const reportPath = fileURLToPath(new URL('report.json', outputDirectory));

// base、Import Map、脚本模式和外链资源均从 HTML 读取。
// 唯一的可选参数用于保存报告，不影响资源解析。
const report = await scanHtml(inputPath, { reportPath });

console.log('扫描状态：', report.scanStatus);
console.log('报告路径：', reportPath);
console.log('发现的资源：');
console.table(report.resources.map(({ kind, finalUrl }) => ({ kind, url: finalUrl })));
console.log('依赖解析结果：');
console.table(
  report.references.map(({ kind, specifier, resolvedUrl, resolution }) => ({
    kind,
    specifier,
    resolvedUrl,
    resolution,
  })),
);

const index = new GlobalIndex(report);
for (const target of index.findTargets(['sharedConfig'], 'global-object')) {
  console.log('HTML 内联脚本与外链模块中的 sharedConfig 读写：');
  console.table(
    index.sources(target.targetId).map(({ access, resource }) => ({
      operation: access.operation,
      valueType: access.valueType,
      resourceUrl: resource.finalUrl,
      line: access.location.line,
      column: access.location.column,
    })),
  );
}
console.log('冲突候选：', index.conflicts());
console.log('覆盖缺口：', report.unresolved);

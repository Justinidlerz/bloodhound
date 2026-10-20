export type ResourceKind = 'js' | 'html' | 'css' | 'asset';
export type ScriptMode = 'classic' | 'module' | 'unknown';
export type Environment = 'window' | 'worker' | 'unknown';
export type ScanStatus = 'completed' | 'partial' | 'failed';
export type Source =
  | { type: 'file'; path: string }
  | { type: 'url'; url: string }
  | { type: 'content'; content: string; resourceUrl: string };
export interface ImportMap {
  imports?: Record<string, string | null>;
  scopes?: Record<string, Record<string, string | null>>;
}
export interface Context {
  scriptMode: ScriptMode;
  executionEnvironment: Environment;
  baseUrl?: string;
  importMap?: ImportMap;
  importMapBaseUrl?: string;
  scopeKey: string;
  publicPath?: string;
  versions: Record<string, string>;
}
export interface Budgets {
  concurrency: number;
  analysisConcurrency: number;
  timeoutMs: number;
  resourceTimeoutMs: number;
  maxResources: number;
  maxBytes: number;
  maxResourceBytes: number;
  maxBufferedBytes: number;
  maxRecords: number;
  maxDepth: number;
  maxSteps: number;
  maxCandidates: number;
  maxSummaryPasses: number;
  maxRedirects: number;
  retries: number;
}
export interface Location {
  start: number;
  end: number;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  offsetEncoding: 'utf16';
  original?: { source: string; line: number; column: number; name?: string };
  container?: {
    resourceUrl: string;
    offset: number;
    line: number;
    column: number;
    approximate?: boolean;
  };
}
export interface Resource {
  resourceId: string;
  requestedUrl: string;
  finalUrl: string;
  contentHash: string;
  kind: ResourceKind;
  bytes: number;
  scmId?: string;
  inline?: { parentUrl: string; offset: number; approximate?: boolean };
}
export interface AnalysisInstance {
  analysisId: string;
  resourceId: string;
  contextHash: string;
  context: Context;
  scanStatus: ScanStatus;
  exports: Record<string, unknown>;
  sourceMapStatus: 'absent' | 'mapped' | 'unavailable';
}
export interface Reference {
  referenceId: string;
  fromAnalysisId: string;
  toAnalysisIds: string[];
  kind: string;
  specifier?: string;
  resolvedUrl?: string;
  resolution: 'resolved' | 'candidate' | 'unresolved' | 'not-followed';
  location: Location;
  condition?: string;
  attributes?: Record<string, string>;
  reason?: string;
}
export interface Target {
  targetId: string;
  scopeKey: string;
  namespace: string;
  pathSegments: string[];
  keyKind: string;
}
export interface GlobalAccess {
  accessId: string;
  analysisId: string;
  targetRefs: string[];
  operation: string;
  valueType: string;
  location: Location;
  evidence: string[];
  fingerprint: string;
}
export interface Finding {
  findingId: string;
  ruleId: string;
  kind: 'global' | 'dom' | 'css';
  analysisId: string;
  accessRefs: string[];
  location: Location;
  details: Record<string, unknown>;
  fingerprint: string;
}
export interface Unresolved {
  analysisId?: string;
  referenceId?: string;
  location?: Location;
  reason: string;
  affectedCapabilities: string[];
  detail?: string;
}
export interface PendingResource {
  kind: ResourceKind;
  source: Source;
  resourceUrl: string;
  context: Context;
  depth: number;
  incoming: string[];
  inline?: { parentUrl: string; offset: number; parentContent: string };
}
export interface Checkpoint {
  checkpointVersion: 1;
  configHash: string;
  entries: ScanEntry[];
  pending: PendingResource[];
  report: Report;
  options?: Pick<ScanOptions, 'access' | 'resourceMappings' | 'sourceMaps'>;
}
export interface ScanEntry {
  kind: ResourceKind;
  source: Source;
  resourceUrl?: string;
}
export interface Report {
  schemaVersion: 1;
  indexSchemaVersion: 1;
  scannerVersion: string;
  scanId: string;
  scopeKey: string;
  scanStatus: ScanStatus;
  runStatus: 'finished' | 'cancelled';
  startedAt: string;
  finishedAt: string;
  configuration: { context: Context; budgets: Budgets; dependencyMode: 'single' | 'recursive' };
  entryAnalysisIds: string[];
  resources: Resource[];
  analysisInstances: AnalysisInstance[];
  references: Reference[];
  globalTargets: Target[];
  globalAccesses: GlobalAccess[];
  findings: Finding[];
  unresolved: Unresolved[];
  coverage: {
    analyzed: number;
    failed: number;
    unresolved: number;
    bytes: number;
    dependencyMode: 'single' | 'recursive';
    sourceMapped: number;
    elapsedMs: number;
  };
}
export interface ScanOptions {
  scriptMode?: ScriptMode;
  executionEnvironment?: Environment;
  /** 离线资源的逻辑地址；直接扫描 URL 时无需重复设置。 */
  resourceUrl?: string;
  /** 补充文档基准；HTML 中的有效 base 标签会被自动读取。 */
  baseUrl?: string;
  /** 补充页面上下文，供单独扫描 JS 等场景使用；HTML 内的 Import Map 自动读取。 */
  importMap?: ImportMap;
  /** 外部传入 importMap 中相对地址的解析基准。 */
  importMapBaseUrl?: string;
  publicPath?: string;
  scopeKey?: string;
  scope?: Record<string, unknown>;
  versions?: Record<string, string>;
  dependencyMode?: 'single' | 'recursive';
  budgets?: Partial<Budgets>;
  signal?: AbortSignal;
  access?: { protocols?: string[]; hosts?: string[]; fileRoots?: string[] };
  /** 用本地文件或源码替代已解析的资源 URL；不是 HTML Import Map，普通页面扫描无需设置。 */
  resourceMappings?: Record<string, Source>;
  requestHeaders?: Record<string, Record<string, string>>;
  proxy?: string;
  jsonlPath?: string;
  reportPath?: string;
  checkpointPath?: string;
  resumePath?: string;
  sourceMaps?: boolean;
  resume?: Checkpoint;
}
export type StreamRecord =
  | { type: 'resetAnalysis'; value: { analysisId: string } }
  | {
      type: keyof Pick<
        Report,
        | 'resources'
        | 'analysisInstances'
        | 'references'
        | 'globalTargets'
        | 'globalAccesses'
        | 'findings'
        | 'unresolved'
      >;
      value: unknown;
    }
  | {
      type: 'summary';
      value: Omit<
        Report,
        | 'resources'
        | 'analysisInstances'
        | 'references'
        | 'globalTargets'
        | 'globalAccesses'
        | 'findings'
        | 'unresolved'
      >;
    };

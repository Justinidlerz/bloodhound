import type { Environment, ResourceKind, ScanStatus, ScanTimings, ScriptMode } from './types.js';

/** Lines are one-based; columns are zero-based UTF-16 code units. */
export interface ManifestLocation {
  line: number;
  column: number;
  endLine?: number;
  endColumn: number;
  original?: { source: string; line: number; column: number; name?: string };
  container?: { line: number; column: number; approximate?: boolean };
}

/** A source analyzed in one execution context; IDs are local to this manifest. */
export interface ManifestSource {
  id: string;
  kind: ResourceKind;
  status: ScanStatus;
  url?: string;
  inline?: { parent?: string; parentUrl?: string; offset: number; approximate?: boolean };
  scriptMode?: ScriptMode;
  environment?: Environment;
  /** Present when multiple isolated global environments occur in the scan. */
  scope?: number;
  sourceMapStatus?: 'mapped' | 'unavailable';
}

export interface Manifest {
  timings?: ScanTimings;
  format: 'compact';
  schemaVersion: 1;
  scanStatus: ScanStatus;
  runStatus: 'finished' | 'cancelled';
  certainty: 'static-candidate';
  summary: {
    resources: number;
    analyses: number;
    dependencies: number;
    globalMutations: number;
    /** Findings excluding mutations already listed in globals. */
    findings: number;
    unresolved: number;
    omittedReadAccesses: number;
    bytes: number;
    elapsedMs: number;
    dependencyMode: 'single' | 'recursive';
  };
  entries: string[];
  sources: ManifestSource[];
  dependencies: {
    from: string;
    to?: string[];
    kind: string;
    status: 'resolved' | 'candidate' | 'unresolved' | 'not-followed';
    specifier?: string;
    url?: string;
    reason?: string;
    condition?: string;
    location: ManifestLocation;
  }[];
  globals: {
    source: string;
    targets: { namespace: string; path: string[]; keyKind?: string }[];
    operation: string;
    valueType?: string;
    location: ManifestLocation;
  }[];
  findings: {
    source: string;
    rule: string;
    details?: Record<string, unknown>;
    location: ManifestLocation;
  }[];
  /** Repeated gaps grouped by source, reason, capabilities, and detail. */
  unresolved: {
    source?: string;
    reason: string;
    count: number;
    capabilities: string[];
    detail?: string;
    exampleLocation?: ManifestLocation;
  }[];
}

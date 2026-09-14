export interface ScanOptions {
  extensions?: string[]
  overrideRules?: string[]
  metadataOnly?: boolean
  selectedFilesOnly?: boolean
  skipHidden?: boolean
  standardSkips?: boolean
  ignorePolicy?: 'repository' | 'none' | 'gitCompatible'
  hashFileContents?: boolean
  maxFileBytes?: number
  maxEntries?: number
  maxTotalBytes?: number
  maxDepth?: number
  parallelism?: number
  compact?: boolean
  signal?: AbortSignal
}

export interface WatchPlan {
  changed?: string[]
  removed?: string[]
  fullRescan?: boolean
  rejectedEvents?: number
}

export interface ScannedFile {
  relative: string
  bytes: number
  content_hash?: string
  binary_checked: boolean
}

export interface ScanReport {
  files: ScannedFile[]
  skipped: Array<{ relative: string; kind: string; detail_hash?: string }>
  warnings: Array<{ relative?: string; message_hash: string }>
  ignore_sources: Array<{ kind: string; repository_relative?: string; content_hash: string }>
  revision: string
  descriptor?: { version: number; policy: string }
  complete: boolean
  termination?: string
  selection_portable: boolean
}

export interface CompactScannedFile {
  relative: string
  bytes: number
  content?: {
    content_hash?: string
    content_fingerprint?: string
    binary_checked: boolean
  }
}

export interface CompactScanReport {
  root: string
  files: CompactScannedFile[]
  skipped: Array<{ relative: string; kind: string; detail?: string }>
  warnings: Array<{ relative?: string; message: string }>
  ignore_sources: Array<{ kind: string; location: string; content_hash: string }>
  revision: string
  descriptor?: { version: number; policy: string }
  complete: boolean
  termination?: string
  portable: boolean
}

export interface ScanCache {
  format_version: number
  root: string
  entries: Array<{
    relative: string
    bytes: number
    content_hash: string
    content_fingerprint?: string
    binary_checked: boolean
  }>
}

export interface ScanDiagnostics {
  rustCoreVersion: string
  packageVersion: string
  target: string
  muslSupported: boolean
  muslBuild: boolean
  supportedTargets: string[]
}

export declare class CancellationToken {
  constructor()
  cancel(): void
  isCancelled(): boolean
}

export declare class ScanSession {
  constructor(root: string, options?: ScanOptions)
  static open(root: string, options?: ScanOptions): Promise<ScanSession>
  static openSync(root: string, options?: ScanOptions): ScanSession
  snapshot(options?: { compact?: boolean }): ScanReport | CompactScanReport
  exportCache(): ScanCache
  updateReason(): string | undefined
  applyWatchPlan(plan: WatchPlan, options?: { compact?: boolean; signal?: AbortSignal }): Promise<ScanReport | CompactScanReport>
  applyWatchPlanSync(plan: WatchPlan, options?: { compact?: boolean; signal?: AbortSignal }): ScanReport | CompactScanReport
  files(options?: { batchSize?: number; signal?: AbortSignal }): AsyncGenerator<ScannedFile[]>
  close(): void
  [Symbol.dispose](): void
}

export declare function scanRepository(root: string, options: ScanOptions & { compact: true }): Promise<CompactScanReport>
export declare function scanRepository(root: string, options?: ScanOptions): Promise<ScanReport>
export declare function scanRepositorySync(root: string, options: ScanOptions & { compact: true }): CompactScanReport
export declare function scanRepositorySync(root: string, options?: ScanOptions): ScanReport
export declare function scanPaths(root: string, options?: ScanOptions): Promise<string[]>
export declare function scanPathsSync(root: string, options?: ScanOptions): string[]
export declare function exportScanCache(root: string, options?: ScanOptions): Promise<ScanCache>
export declare function exportScanCacheSync(root: string, options?: ScanOptions): ScanCache
export declare function scanDiagnostics(): ScanDiagnostics

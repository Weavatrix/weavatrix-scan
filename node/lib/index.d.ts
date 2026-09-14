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
  snapshot(options?: { compact?: boolean }): ScanReport | object
  applyWatchPlan(plan: WatchPlan, options?: { compact?: boolean }): ScanReport | object
  files(options?: { batchSize?: number; signal?: AbortSignal }): AsyncGenerator<ScannedFile[]>
}

export declare function scanRepository(root: string, options?: ScanOptions): Promise<ScanReport>
export declare function scanRepositorySync(root: string, options?: ScanOptions): ScanReport
export declare function scanDiagnostics(): ScanDiagnostics

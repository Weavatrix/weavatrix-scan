export interface ScanOptions {
  extensions?: string[]
  overrideRules?: string[]
  metadataOnly?: boolean
  selectedFilesOnly?: boolean
  skipHidden?: boolean
  maxFileBytes?: number
  maxEntries?: number
  maxTotalBytes?: number
  maxDepth?: number
  parallelism?: number
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
  complete: boolean
  termination?: string
  selection_portable: boolean
}

export declare function scanRepository(root: string, options?: ScanOptions): Promise<ScanReport>
export declare function scanRepositorySync(root: string, options?: ScanOptions): ScanReport

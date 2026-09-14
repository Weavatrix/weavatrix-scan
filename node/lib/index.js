'use strict'

const native = require('../index.js')

function encode(options) {
  if (options == null) {
    return { json: undefined, token: undefined, release() {} }
  }
  const { signal, ...rest } = options
  const { token, release } = bindSignal(signal)
  return { json: JSON.stringify(rest), token, release }
}

function bindSignal(signal) {
  if (signal == null) {
    return { token: undefined, release() {} }
  }
  const token = new native.CancellationToken()
  if (signal.aborted) {
    token.cancel()
    return { token, release() {} }
  }
  const handler = () => token.cancel()
  signal.addEventListener('abort', handler)
  return {
    token,
    release() {
      signal.removeEventListener('abort', handler)
    },
  }
}

async function scanRepository(root, options) {
  const { json, token, release } = encode(options)
  try {
    return JSON.parse(await native.scanRepository(root, json, token))
  } finally {
    release()
  }
}

function scanRepositorySync(root, options) {
  const { json, token, release } = encode(options)
  try {
    return JSON.parse(native.scanRepositorySync(root, json, token))
  } finally {
    release()
  }
}

async function scanPaths(root, options) {
  const { json, token, release } = encode(options)
  try {
    return await native.scanPaths(root, json, token)
  } finally {
    release()
  }
}

function scanPathsSync(root, options) {
  const { json, token, release } = encode(options)
  try {
    return native.scanPathsSync(root, json, token)
  } finally {
    release()
  }
}

async function exportScanCache(root, options) {
  const { json, token, release } = encode(options)
  try {
    return JSON.parse(await native.exportScanCache(root, json, token))
  } finally {
    release()
  }
}

function exportScanCacheSync(root, options) {
  const { json, token, release } = encode(options)
  try {
    return JSON.parse(native.exportScanCacheSync(root, json, token))
  } finally {
    release()
  }
}

function scanDiagnostics() {
  return JSON.parse(native.scanDiagnostics())
}

class ScanSession {
  constructor(root, options) {
    if (typeof root === 'string') {
      const { json, token, release } = encode(options)
      try {
        this._native = native.ScanSession.open(root, json, token)
      } finally {
        release()
      }
      return
    }
    this._native = root
  }

  static openSync(root, options) {
    return new ScanSession(root, options)
  }

  static async open(root, options) {
    const { json, token, release } = encode(options)
    try {
      return new ScanSession(await native.openScanSession(root, json, token))
    } finally {
      release()
    }
  }

  snapshot(options) {
    return JSON.parse(this._native.snapshot(Boolean(options && options.compact)))
  }

  exportCache() {
    return JSON.parse(this._native.exportCache())
  }

  updateReason() {
    return this._native.updateReason() ?? undefined
  }

  applyWatchPlanSync(plan, options) {
    const compact = Boolean(options && options.compact)
    const { token, release } = bindSignal(options && options.signal)
    try {
      return JSON.parse(this._native.applyWatchPlan(JSON.stringify(plan ?? {}), compact, token))
    } finally {
      release()
    }
  }

  async applyWatchPlan(plan, options) {
    const compact = Boolean(options && options.compact)
    const { token, release } = bindSignal(options && options.signal)
    try {
      return JSON.parse(
        await this._native.applyWatchPlanAsync(JSON.stringify(plan ?? {}), compact, token),
      )
    } finally {
      release()
    }
  }

  async *files(options) {
    const batchSize = Math.max(1, Number((options && options.batchSize) || 512))
    const cursor = JSON.parse(this._native.fileCursor())
    let offset = 0
    while (offset < cursor.count) {
      if (options && options.signal && options.signal.aborted) {
        return
      }
      const page = JSON.parse(this._native.filesPage(cursor.generation, offset, batchSize))
      if (page.files.length === 0) {
        return
      }
      yield page.files
      offset += page.files.length
    }
  }

  close() {
    this._native = undefined
  }

  [Symbol.dispose]() {
    this.close()
  }
}

module.exports = {
  scanRepository,
  scanRepositorySync,
  scanPaths,
  scanPathsSync,
  exportScanCache,
  exportScanCacheSync,
  scanDiagnostics,
  ScanSession,
  CancellationToken: native.CancellationToken,
}

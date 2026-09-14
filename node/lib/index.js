'use strict'

const native = require('../index.js')

function encode(options) {
  if (options == null) {
    return { json: undefined, token: undefined }
  }
  const { signal, ...rest } = options
  const token = tokenFromSignal(signal)
  return { json: JSON.stringify(rest), token }
}

function tokenFromSignal(signal) {
  if (signal == null) {
    return undefined
  }
  const token = new native.CancellationToken()
  if (signal.aborted) {
    token.cancel()
    return token
  }
  signal.addEventListener('abort', () => token.cancel(), { once: true })
  return token
}

async function scanRepository(root, options) {
  const { json, token } = encode(options)
  return JSON.parse(await native.scanRepository(root, json, token))
}

function scanRepositorySync(root, options) {
  const { json, token } = encode(options)
  return JSON.parse(native.scanRepositorySync(root, json, token))
}

function scanDiagnostics() {
  return JSON.parse(native.scanDiagnostics())
}

class ScanSession {
  constructor(root, options) {
    const { json, token } = encode(options)
    this._native = native.ScanSession.open(root, json, token)
  }

  snapshot(options) {
    return JSON.parse(this._native.snapshot(Boolean(options && options.compact)))
  }

  applyWatchPlan(plan, options) {
    return JSON.parse(
      this._native.applyWatchPlan(JSON.stringify(plan ?? {}), Boolean(options && options.compact)),
    )
  }

  async *files(options) {
    const batchSize = Math.max(1, Number((options && options.batchSize) || 512))
    let offset = 0
    const total = this._native.fileCount()
    while (offset < total) {
      if (options && options.signal && options.signal.aborted) {
        return
      }
      const batch = JSON.parse(this._native.filesPage(offset, batchSize))
      if (batch.length === 0) {
        return
      }
      yield batch
      offset += batch.length
    }
  }
}

module.exports = {
  scanRepository,
  scanRepositorySync,
  scanDiagnostics,
  ScanSession,
  CancellationToken: native.CancellationToken,
}

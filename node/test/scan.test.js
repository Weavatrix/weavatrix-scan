'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { getEventListeners } = require('node:events')
const {
  exportScanCacheSync,
  scanDiagnostics,
  scanPaths,
  scanPathsSync,
  scanRepository,
  scanRepositorySync,
  ScanSession,
} = require('../lib/index.js')

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'weavatrix-scan-node-'))
  fs.mkdirSync(path.join(root, 'src'))
  fs.writeFileSync(path.join(root, 'src', 'a.js'), 'export const a = 1\n')
  fs.writeFileSync(path.join(root, 'src', 'b.rs'), 'pub fn b() {}\n')
  fs.mkdirSync(path.join(root, 'node_modules'))
  fs.writeFileSync(path.join(root, 'node_modules', 'ignored.js'), 'ignored\n')
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  return root
}

test('scans asynchronously without blocking the JavaScript API contract', async (t) => {
  const report = await scanRepository(fixture(t), { extensions: ['js', 'rs'] })
  assert.equal(report.complete, true)
  assert.deepEqual(report.files.map((file) => file.relative), ['src/a.js', 'src/b.rs'])
  assert.ok(report.revision)
})

test('scanPaths returns sorted selected relatives without a manifest', async (t) => {
  const root = fixture(t)
  const expected = scanRepositorySync(root, { metadataOnly: true, selectedFilesOnly: true })
    .files.map((file) => file.relative)
  assert.deepEqual(scanPathsSync(root), expected)
  assert.deepEqual(await scanPaths(root), expected)
})

test('metadata-only sync scan avoids content hashes', (t) => {
  const report = scanRepositorySync(fixture(t), { metadataOnly: true, selectedFilesOnly: true })
  assert.equal(report.files.length, 2)
  assert.equal(report.files.every((file) => file.content_hash == null), true)
  assert.equal(report.skipped.length, 0)
})

test('session pages files and abort stops a scan', async (t) => {
  const root = fixture(t)
  const session = await ScanSession.open(root, { extensions: ['js', 'rs'], selectedFilesOnly: true })
  const pages = []
  for await (const batch of session.files({ batchSize: 1 })) {
    pages.push(batch)
  }
  assert.equal(pages.length, 2)
  const diagnostics = scanDiagnostics()
  assert.equal(diagnostics.muslSupported, false)
  assert.ok(diagnostics.packageVersion)
  const controller = new AbortController()
  controller.abort()
  const cancelled = await scanRepository(root, { signal: controller.signal })
  assert.equal(cancelled.complete, false)
  assert.equal(cancelled.termination, 'Cancelled')
})

test('compact is a manifest and cache is a separate export', (t) => {
  const root = fixture(t)
  const compact = scanRepositorySync(root, { metadataOnly: true, compact: true })
  assert.equal(Array.isArray(compact.files), true)
  assert.ok(compact.revision)
  assert.equal(compact.complete, true)
  assert.equal(compact.termination == null, true)
  const cache = exportScanCacheSync(root, { metadataOnly: true })
  assert.equal(Array.isArray(cache.entries), true)
  assert.equal(cache.entries.length, 0)
})

test('files iterator does not mix generations after a watch update', async (t) => {
  const root = fixture(t)
  const session = new ScanSession(root, { extensions: ['js', 'rs'], selectedFilesOnly: true })
  const iterator = session.files({ batchSize: 1 })
  const first = await iterator.next()
  assert.equal(first.done, false)
  const removed = first.value[0].relative
  fs.rmSync(path.join(root, ...removed.split('/')))
  await session.applyWatchPlan({ removed: [removed] })
  await assert.rejects(() => iterator.next())
  const remaining = []
  for await (const batch of session.files({ batchSize: 8 })) {
    remaining.push(...batch.map((file) => file.relative))
  }
  assert.deepEqual(
    remaining,
    ['src/a.js', 'src/b.rs'].filter((relative) => relative !== removed),
  )
  session.close()
})

test('successful scans remove abort listeners', async (t) => {
  const root = fixture(t)
  const controller = new AbortController()
  await scanRepository(root, { metadataOnly: true, signal: controller.signal })
  await scanRepository(root, { metadataOnly: true, signal: controller.signal })
  await scanRepository(root, { metadataOnly: true, signal: controller.signal })
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0)
})

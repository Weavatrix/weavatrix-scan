'use strict'

const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const test = require('node:test')
const { scanDiagnostics, scanRepository, scanRepositorySync, ScanSession } = require('../lib/index.js')

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

test('metadata-only sync scan avoids content hashes', (t) => {
  const report = scanRepositorySync(fixture(t), { metadataOnly: true, selectedFilesOnly: true })
  assert.equal(report.files.length, 2)
  assert.equal(report.files.every((file) => file.content_hash == null), true)
  assert.equal(report.skipped.length, 0)
})

test('session pages files and abort stops a scan', async (t) => {
  const root = fixture(t)
  const session = new ScanSession(root, { extensions: ['js', 'rs'], selectedFilesOnly: true })
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

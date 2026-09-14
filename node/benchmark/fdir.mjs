import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { performance } from 'node:perf_hooks'
import { createRequire } from 'node:module'
import { fdir } from 'fdir'

const require = createRequire(import.meta.url)
const { scanPathsSync, scanRepositorySync } = require('../lib/index.js')
const files = Number(process.env.WEAVATRIX_BENCH_FILES ?? 20_000)
const rounds = Number(process.env.WEAVATRIX_BENCH_ROUNDS ?? 7)
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'weavatrix-scan-bench-'))

function median(samples) {
  const sorted = [...samples].sort((left, right) => left - right)
  return sorted[Math.floor(sorted.length / 2)]
}

function runOnce(run) {
  const started = performance.now()
  const result = run()
  const elapsed = performance.now() - started
  if (result.length !== files) throw new Error(`parity failure: ${result.length} != ${files}`)
  return elapsed
}

async function runAsyncOnce(run) {
  const started = performance.now()
  const result = await run()
  const elapsed = performance.now() - started
  if (result.length !== files) throw new Error(`parity failure: ${result.length} != ${files}`)
  return elapsed
}

function measurePair(left, right) {
  const leftSamples = []
  const rightSamples = []
  for (let round = 0; round < rounds + 2; round += 1) {
    let leftElapsed
    let rightElapsed
    if (round % 2 === 0) {
      leftElapsed = runOnce(left)
      rightElapsed = runOnce(right)
    } else {
      rightElapsed = runOnce(right)
      leftElapsed = runOnce(left)
    }
    if (round >= 2) {
      leftSamples.push(leftElapsed)
      rightSamples.push(rightElapsed)
    }
  }
  return [median(leftSamples), median(rightSamples)]
}

async function measureAsyncPair(left, right) {
  const leftSamples = []
  const rightSamples = []
  for (let round = 0; round < rounds + 2; round += 1) {
    let leftElapsed
    let rightElapsed
    if (round % 2 === 0) {
      leftElapsed = await runAsyncOnce(left)
      rightElapsed = await runAsyncOnce(right)
    } else {
      rightElapsed = await runAsyncOnce(right)
      leftElapsed = await runAsyncOnce(left)
    }
    if (round >= 2) {
      leftSamples.push(leftElapsed)
      rightSamples.push(rightElapsed)
    }
  }
  return [median(leftSamples), median(rightSamples)]
}

try {
  const directories = Math.ceil(files / 200)
  for (let directory = 0; directory < directories; directory += 1) {
    const target = path.join(root, `d${directory}`)
    fs.mkdirSync(target)
    const count = Math.min(200, files - directory * 200)
    for (let index = 0; index < count; index += 1) {
      fs.writeFileSync(path.join(target, `f${index}.txt`), 'x')
    }
  }

  const oursPaths = () => scanPathsSync(root)
  const fdirPaths = () => new fdir()
    .withRelativePaths()
    .crawl(root)
    .sync()
    .map((file) => file.replaceAll('\\', '/'))
    .sort()
  const ours = () => scanRepositorySync(root, { metadataOnly: true, selectedFilesOnly: true })
    .files.map((file) => ({ relative: file.relative, bytes: file.bytes }))
  const competitor = () => fdirPaths()
    .map((relative) => ({ relative, bytes: fs.statSync(path.join(root, relative)).size }))
  const competitorAsync = async () => {
    const paths = fdirPaths()
    const workers = Math.min(8, os.availableParallelism?.() || 8)
    const sized = new Array(paths.length)
    let next = 0
    await Promise.all(Array.from({ length: workers }, async () => {
      while (next < paths.length) {
        const index = next
        next += 1
        const relative = paths[index]
        const bytes = (await fs.promises.stat(path.join(root, relative))).size
        sized[index] = { relative, bytes }
      }
    }))
    return sized
  }
  const oursAsync = async () => ours()
  if (JSON.stringify(oursPaths()) !== JSON.stringify(fdirPaths())) {
    throw new Error('exact relative-path parity failed')
  }
  const oursParity = ours()
  const competitorParity = competitor()
  if (JSON.stringify(oursParity) !== JSON.stringify(competitorParity)) {
    throw new Error('exact path-and-size parity failed')
  }
  if (JSON.stringify(await competitorAsync()) !== JSON.stringify(oursParity)) {
    throw new Error('exact async-stat path-and-size parity failed')
  }

  const [weavatrixPathsMs, fdirPathsMs] = measurePair(oursPaths, fdirPaths)
  const [weavatrixMs, fdirMs] = measurePair(ours, competitor)
  const [weavatrixAsyncMs, fdirAsyncStatMs] = await measureAsyncPair(oursAsync, competitorAsync)
  console.log(JSON.stringify({
    files,
    rounds,
    runtime: process.versions.bun ? `bun ${process.versions.bun}` : `node ${process.version}`,
    note: 'statSync row is historical-shaped; asyncStat is bounded parallel fs.promises.stat. Neither is a content workload.',
    results: [
      {
        contract: 'sorted relative paths via scanPaths; no portable manifest',
        weavatrixMs: weavatrixPathsMs,
        fdirMs: fdirPathsMs,
        ratio: fdirPathsMs / weavatrixPathsMs,
      },
      {
        contract: 'metadata-only traversal with sorted relative paths and byte sizes',
        weavatrixMs,
        fdirMs,
        ratio: fdirMs / weavatrixMs,
      },
      {
        contract: 'identical {relative, bytes} array; fdir paths plus bounded parallel async stat',
        weavatrixMs: weavatrixAsyncMs,
        fdirAsyncStatMs,
        ratio: fdirAsyncStatMs / weavatrixAsyncMs,
      },
    ],
  }, null, 2))
} finally {
  fs.rmSync(root, { recursive: true, force: true })
}

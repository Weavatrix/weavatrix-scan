# weavatrix-scan

A deterministic, path-safe repository scanner — written in Rust, exposed to
Node.js and Bun through Node-API.

Not a directory walker. It produces a manifest: normalized paths, byte sizes,
optional content hashes, an aggregate revision, ignore-rule provenance, typed
evidence for everything it skipped, and hard bounds it will not exceed. It
executes no repository code.

```console
npm install weavatrix-scan
# or
bun add weavatrix-scan
```

```js
const { scanRepository } = require('weavatrix-scan')

const report = await scanRepository(process.cwd(), {
  extensions: ['js', 'ts', 'rs'],
  selectedFilesOnly: true,
  maxFileBytes: 2_000_000,
})

report.revision        // one hash for the whole selection
report.files           // [{ relative, bytes, content_hash?, binary_checked }]
report.skipped         // why each excluded entry was excluded
report.complete        // false when a bound stopped the scan
```

---

## Why a manifest instead of a list of paths

Two runs over the same tree must produce the same bytes, or nothing built on
top can be cached, diffed, or trusted. So the report carries:

- **`revision`** — an aggregate hash of the selection. Equal revisions mean
  equal selections, without comparing file lists.
- **`skipped`** — typed evidence for every exclusion, so "the file is missing
  from your output" always has an answer.
- **`ignore_sources`** — which ignore files were consulted and their content
  hashes, so a selection can be explained and reproduced.
- **`complete` / `termination`** — an explicit statement that a bound was hit,
  instead of a silently short list.

---

## API

### `scanRepository(root, options?) → Promise<ScanReport>`

Runs on the native worker pool; the JavaScript event loop stays free.

### `scanRepositorySync(root, options?) → ScanReport`

The blocking form, for CLIs and controlled startup paths.

| Parameter | Type | Notes |
| --- | --- | --- |
| `root` | `string` | Repository root. Must be an existing directory. |
| `options` | `ScanOptions` | See below. |

### `ScanOptions`

| Option | Type | Default | Effect |
| --- | --- | --- | --- |
| `extensions` | `string[]` | all | Restricts selection to these extensions, without a leading dot. |
| `overrideRules` | `string[]` | — | Gitignore-syntax rules applied above discovered ignore files. A leading `!` re-includes. |
| `metadataOnly` | `boolean` | `false` | Skips content reads: no hashing, no binary detection. The fastest useful mode. |
| `selectedFilesOnly` | `boolean` | `false` | Returns only selected files and drops per-entry skip records, which keeps memory flat on very large trees. |
| `skipHidden` | `boolean` | `true` | Whether dotfiles and dot-directories are skipped. |
| `maxFileBytes` | `number` | scanner default | Files above this are skipped with typed evidence rather than read. |
| `maxEntries` | `number` | unbounded | Hard entry bound. Hitting it sets `complete: false` and `termination`. |
| `maxTotalBytes` | `number` | unbounded | Hard byte bound, same reporting. |
| `maxDepth` | `number` | unbounded | Traversal depth bound. |
| `parallelism` | `number` | available | Worker count. |

---

## `ScanReport`

The report is the crate's **portable** report, so its field names are the
serialized Rust names.

| Field | Type | Meaning |
| --- | --- | --- |
| `files` | `ScannedFile[]` | The selection, in deterministic order. |
| `skipped` | `{ relative, kind, detail_hash? }[]` | One record per exclusion. `kind` names the reason. |
| `warnings` | `{ relative?, message_hash }[]` | Non-fatal problems, hashed so a report never leaks message text. |
| `ignore_sources` | `{ kind, repository_relative?, content_hash }[]` | Which ignore inputs shaped this selection. |
| `revision` | `string` | Aggregate hash of roots, paths, and content. |
| `complete` | `boolean` | `false` when a bound stopped the scan. |
| `termination` | `string \| undefined` | Which bound: entries, total bytes, timeout, or cancellation. |
| `selection_portable` | `boolean` | Whether the selection is reproducible on another platform. |

### `ScannedFile`

| Field | Type | Meaning |
| --- | --- | --- |
| `relative` | `string` | Forward-slash path relative to `root`, on every platform. |
| `bytes` | `number` | File size. |
| `content_hash` | `string \| undefined` | Present unless `metadataOnly` was set. |
| `binary_checked` | `boolean` | Whether binary detection actually ran on this file. |

---

## Errors

| `code` | Cause |
| --- | --- |
| `InvalidArg` | Unknown option key, or malformed option JSON. |
| `GenericFailure` | Root missing, unreadable, or not a directory. |

---

## What ships

| | |
| --- | --- |
| Runtimes | Node.js 18+ (Node-API 8), Bun 1.4+ |
| Platforms | Windows x64/arm64, macOS x64/arm64, glibc Linux x64/arm64 |
| Install script | none |
| Network at install | none |
| Runtime dependencies | none |
| Platform packages | none — all six bindings are in this one tarball |
| Writes to disk | none |

---

## Measured

[`benchmark/RESULTS.md`](benchmark/RESULTS.md) is generated from the
[weavatrix-benchmarks](https://github.com/Weavatrix/weavatrix-benchmarks)
harness, which forces both sides to return the identical array before either is
timed. The competitor is `fdir`, the fastest widely used Node crawler.

Medians of three independent runs over 20,000 files:

| Contract | Node 24 | Bun 1.3 |
| --- | ---: | ---: |
| Sorted relative paths | **0.94x** (0.83–1.01) | **0.94x** (0.93–1.14) |
| Sorted paths **plus byte sizes** | **80.1x** (77.5–82.7) | **86.7x** (84.0–92.4) |

The first row is deliberately unfavourable and stays published: `fdir` returns
raw paths while Weavatrix still performs its scanner metadata work, so the two
land close together and the ordering flips between runs. The second row is the
equal consumer-facing contract, where `fdir` needs one `statSync` per path.

---

Scan owns its repository, package, release evidence, and MIT license, and can
be used entirely on its own.

Repository: [Weavatrix/weavatrix-scan](https://github.com/Weavatrix/weavatrix-scan) ·
Rust crate: [crates.io/crates/weavatrix-scan](https://crates.io/crates/weavatrix-scan) ·
License: [MIT](https://github.com/Weavatrix/weavatrix-scan/blob/main/LICENSE)

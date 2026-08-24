# weavatrix-scan

An independent repository-scanning product, not just a directory walker.

`weavatrix-scan` produces deterministic, path-safe manifests with normalized paths, byte sizes, optional hashes, aggregate revisions, ignore policy, bounded traversal, and typed evidence for skipped files. The npm package runs the same Rust scanner through Node-API; it does not execute repository code and does not depend on an MCP server.

## Install

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

console.log(report.revision)
console.log(report.files)
console.log(report.skipped)
```

`scanRepository` runs on the native worker pool so the JavaScript API does not perform the scan on the event loop. `scanRepositorySync` is available for CLIs and controlled startup paths.

## Measured Node and Bun performance

Windows x64, 20,000 one-byte files, medians after two warmups, alternating execution order:

| Equal result | Runtime | Weavatrix | fdir 6.5.0 | Winner |
| --- | --- | ---: | ---: | ---: |
| Sorted paths only | Node 24.15.0 | 73.443 ms | 43.707 ms | fdir 1.68x |
| Sorted paths only | Bun 1.4.0 | 54.977 ms | 30.280 ms | fdir 1.82x |
| Sorted paths + byte sizes | Node 24.15.0 | 61.886 ms | 716.302 ms | Weavatrix 11.57x |
| Sorted paths + byte sizes | Bun 1.4.0 | 56.170 ms | 455.607 ms | Weavatrix 8.11x |

The paths-only row keeps the narrower walker's win visible: Weavatrix still gathers scanner metadata while only paths are compared. For the path-plus-size contract, `fdir` adds `statSync` per file. Hashing and revision work are excluded from both. See the [full report and reproduction commands](https://github.com/Weavatrix/weavatrix-scan/blob/main/node/benchmark/RESULTS.md).

## Runtime and ownership boundary

One self-contained npm package supports Node.js 18+ and Bun 1.4+ and includes the Windows, macOS, and glibc Linux binaries for x64 and arm64. No public platform-package names are created.

Scan owns its repository, package, release evidence, and MIT license. It can be used independently of every other Weavatrix product.

Repository: [Weavatrix/weavatrix-scan](https://github.com/Weavatrix/weavatrix-scan) · Rust crate: [crates.io/crates/weavatrix-scan](https://crates.io/crates/weavatrix-scan) · License: [MIT](https://github.com/Weavatrix/weavatrix-scan/blob/main/LICENSE)

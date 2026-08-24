# weavatrix-scan for Node.js and Bun

Native Node-API bindings to the current Rust `weavatrix-scan` crate. The npm
package performs the same boundary-safe, ignore-aware, deterministic scan; it
is not a JavaScript port and does not execute repository code.

```js
const { scanRepository } = require('weavatrix-scan')

const report = await scanRepository(process.cwd(), {
  extensions: ['js', 'ts', 'rs'],
  selectedFilesOnly: true,
})
```

`scanRepository` runs on the native worker pool. `scanRepositorySync` is
available for CLIs and controlled startup paths. The same npm package targets
Node.js 18+ and Bun 1.4+.

The benchmark compares exact sorted relative paths and file byte sizes against
`fdir` plus `statSync` on a generated tree. Content hashing is disabled for both
sides so the measured contract is metadata traversal and result materialization,
not extra Weavatrix evidence.

# Node.js and Bun benchmark snapshot

Measured on 2026-08-24 on Windows x64 with 20,000 one-byte files. Values are medians after two warm-up rounds; execution order alternates per round and exact outputs are asserted before timing.

| Contract | Runtime | Weavatrix | fdir 6.5.0 | Result |
| --- | --- | ---: | ---: | ---: |
| Sorted relative paths | Node 24.15.0 | 73.443 ms | 43.707 ms | fdir 1.68x faster |
| Sorted relative paths | Bun 1.4.0 | 54.977 ms | 30.280 ms | fdir 1.82x faster |
| Sorted paths + byte sizes | Node 24.15.0 | 61.886 ms | 716.302 ms | Weavatrix 11.57x faster |
| Sorted paths + byte sizes | Bun 1.4.0 | 56.170 ms | 455.607 ms | Weavatrix 8.11x faster |

The first contract intentionally shows the narrow walker advantage: Weavatrix still performs scanner metadata work while only paths are compared. The second is the equal consumer-facing metadata contract; `fdir` adds `statSync` for every path. Neither row includes content hashing, ignore-heavy corpora, skip evidence, or aggregate revision comparison.

Reproduce from `node/`:

```console
npm ci
npm run build
npm run bench
bun run benchmark/fdir.mjs
```

Filesystem cache, antivirus, storage, and corpus shape can materially change traversal timings. Treat these as a reproducible snapshot, not a universal result.

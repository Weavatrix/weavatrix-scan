# Benchmark matrix

Historical numbers in the README and `node/benchmark/RESULTS.md` stay as
reproducible artifacts for the versions and machines that produced them. Do
not copy those times or RSS figures onto newer competitor versions.

## Current harness versions

The crate's dev-dependencies now pin `ignore` 0.4.33 and `jwalk` 0.9.0.
Re-run the benches before publishing a ranking against those crates.

## Required rows

Publish each row with OS, filesystem, hardware, warm/cold regime, crate
versions, output equality, p50/p95, spread, and memory:

| Workload | What is compared |
| --- | --- |
| Raw paths | Directory entries only |
| Paths + metadata | Paths and sizes |
| Ignore-aware selection | Git-style selection |
| Content + hash | Selected files with hashes |
| Incremental lifecycle | `N` files, `k` changes; CPU, allocations, peak RSS, filesystem reads, revision time |
| Scan → parse → index | Downstream cost after Scan |

Incremental updates are at least linear in the retained manifest size. A
file-only plan that inspects `k` files still clones or retains `N` records
and recomputes revision over `N` files.

Suggested `N`: 10k, 100k, 1M. Suggested `k`: 0, 1, 10, 100, and a bulk
change. Measure a series of updates, not a single shot. Do not hide work in
a deferred export.

## Node

`node/benchmark/fdir.mjs` remains a paths and path-plus-size control. Add a
bounded parallel async `stat` row when comparing `fdir`; `statSync` is not
the only competitor shape. A 20k one-byte-file fixture is not a content
workload. A million-file Windows result does not rank Linux.

Published npm targets are glibc Linux, Windows MSVC, and Apple Darwin.
musl is unsupported; `scanDiagnostics()` reports that explicitly.

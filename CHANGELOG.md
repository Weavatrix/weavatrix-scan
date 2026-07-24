# Changelog

All notable changes to this project are documented here.

## Unreleased

- Replace FNV content/revision hashing with streaming SHA-256 and persist
  file-version evidence for safe incremental hash reuse.
- Detect files changed between discovery and content completion, with typed
  `ConcurrentModification` evidence or abort-policy errors.
- Add `Scanner::scan_incremental`, cache statistics, same-size change
  detection, and backward-compatible Serde defaults.
- Parse ignore sources as raw bytes, cover non-UTF8/percent paths with
  differential tests, and honor matching Git conditional includes.
- Integrate adaptive parallel traversal into `Scanner`, add dynamic work below
  narrow roots, and retain low-overhead lanes on broad trees.
- Add first-touch, skewed, deep, bounded-handle, large-content, incremental,
  and real-Weavatrix benchmark profiles.
- Add multi-root `WalkBuilder`, native custom sorting, directory/entry
  callbacks, contents-first ordering, and named scanner file types.

## 0.2.0 - 2026-07-24

- Add a reusable repository matcher with explicit Git-global, Git-exclude,
  parent-rule, custom-rule, and evidence-producing ignore policies.
- Add streaming parallel visitors with subtree pruning, cooperative
  cancellation, and `Continue`/`Skip`/`Quit` controls.
- Add deterministic whole-scan entry, byte, timeout, and cancellation budgets
  with typed partial-report termination evidence.
- Reject repository-local ignore symlinks and keep matcher queries scoped to
  the configured scan root.
- Add independent local-ignore source switches, Git-repository gating,
  high-precedence override globs, and typed hidden-file filtering.
- Add minimum-depth parity across serial walking, parallel collection,
  streaming visitors, and ignore-aware scanning.
- Add deterministic manifest deltas for added, removed, modified, unchanged,
  and uniquely content-matched renamed files with typed evidence quality.
- Add atomic `RepositoryMatcher` refresh for long-lived watch and LSP
  consumers when ignore inputs change.
- Preserve parallel discovery-task order independently of worker completion
  while retaining round-robin load balancing.
- Replace the serialized idle-worker receiver with a condition-variable job
  queue so all bounded workers can wait and wake independently.
- Add regression coverage for completion-order independence and reduce the
  normalized parallel discovery benchmark below `jwalk` on the Windows sample.

## 0.1.1 - 2026-07-23

- Add iterative, lossless `Walker` and bounded `ParallelWalker` APIs.
- Continue after local filesystem errors with typed partial evidence.
- Add depth, open-handle, same-filesystem, and configurable symlink policies.
- Detect followed symlink cycles with native Unix and Windows file identities.
- Expand repository-local Git-ignore parity, diagnostics, and differential
  tests, including nested precedence and non-UTF8 paths.
- Remove per-entry shared-root contention and reuse a bounded worker pool.
- Add a selected-files-only evidence mode for fair, low-allocation manifest
  discovery and bucket common ignore matchers by literal prefix/suffix.
- Bring parallel traversal within the same performance tier as `jwalk` and
  make ignore-aware scanning faster than `ignore` on the published benchmark.
- Add a checked layered architecture contract and keep every Rust file at or
  below 300 lines.
- Validate Rust 1.88 on Linux, Windows, and macOS.

## 0.1.0 - 2026-07-23

- Initial MIT release.
- Add deterministic repository manifests and revisions.
- Add hierarchical `.gitignore`, `.weavatrixignore`, and custom ignore files.
- Add typed skip evidence for binary, oversized, ignored, generated, escaped,
  and symlink entries.
- Add metadata-only, safe-discovery, and parallel rich-manifest modes.
- Add differential correctness tests against `ignore`.
- Add synthetic and real-repository competitor benchmarks.
- Compare complete normalized path/size manifests instead of counts alone.
- Reuse inherited ignore rules, index exact literals, specialize common globs,
  and prefilter complex patterns.
- Stream no-ignore traversal and sort only the final deterministic report.
- Verify exact selected-path parity on Rust, Go, JavaScript, TypeScript,
  Analytics, and Python repositories.

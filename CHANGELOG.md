# Changelog

All notable changes to this project are documented here.

## Unreleased

## 0.4.0 - 2026-07-26

- Add `SelectionMatcher`, a reusable typed API that applies the scanner's
  complete depth, symlink, standard-directory, file-type, extension, size,
  ignore, and override selection policy to standalone paths or existing
  `WalkEntry` values.
- Add lossless root-relative `RepositoryMatcher::normalize`.
- Add the optional `rayon` feature with ready-to-use existing/new Rayon pool
  constructors and cancellable busy-timeout admission while preserving the
  dependency-light default build.

## 0.3.0 - 2026-07-24

- Enable `notify`'s native macOS backend, identify redirected Unix stdout from
  a safely duplicated descriptor, and keep raw non-UTF8 ignore coverage
  separate from UTF-8-only reference-matcher differential checks.
- Add `Scanner::visit_content`, a bounded parallel selected-content pipeline
  with worker-local state, borrowed byte chunks, single-pass hash/binary
  evidence, stable root/path identity, `SkipFile`/global `Quit`, selected
  runtimes, and reentrant fallback.
- Add `MultiScanner::{visit_content, visit_content_streaming}` with root-index
  tagging, shared cancellation, globally bounded execution, and insertion-order
  reports.
- Add traversal-free `Scanner::{visit_changed_content,
  visit_changed_content_streaming}` for safe file-only watcher plans, including
  typed full-rescan fallback and removed-path output.
- Add `ContentVisitMode::Streaming`, which keeps byte/hash/binary evidence while
  omitting selected-file retention and revision computation.
- Reuse one 64 KiB content buffer per worker instead of allocating one for
  every selected file.
- Add `ContentValidationPolicy::{Fast, Strict}` and consolidate Windows
  by-handle size/version checks into one safe `winapi-util` query per
  checkpoint.
- Add direct parallel multi-root streaming callbacks with root-index tagging,
  shared `Skip`/`Quit`/cancellation semantics, selected-runtime support, and
  deterministic per-root reports.
- Add `ParallelRuntime` with process-global, fallibly created dedicated, and
  application-owned executors, external busy-timeout propagation, typed
  worker-submission failures, and reentrant callback safety across walker,
  pull, scanner, and multi-root APIs.
- Add parallel ordered `StatefulWalkBuilder` execution: complete directory
  batches run on workers, callback-mutated state propagates to children, entry
  state and pruning are preserved, and bounded output remains strict DFS.
- Add `CompactScanReport` and direct `Scanner::scan_compact` discovery with one
  retained root path and optional boxed rich evidence, avoiding absolute-path
  and unused version/hash slots per metadata-only entry.
- Correct and expand the competitor matrix with workload-specific winners,
  million-file time/memory evidence, and explicit remaining API gaps.
- Add redirected-stdout protection to every `ParallelWalker` output mode and
  parallel multi-root traversal.
- Add fallible unordered and ordered parallel pull startup APIs.
- Add a reproducible large-scale profile covering raw streaming,
  collected traversal, ignore-aware metadata manifests, rich SHA-256 scans,
  ripgrep, exact `ignore` manifest verification, and peak working-set
  measurements up to 1,000,000 files.
- Add a plain-entry traversal fast path and consuming `WalkEntry::into_path`
  API, closing the measured deep-tree raw-walk gap without weakening typed
  errors or traversal limits.
- Expand built-in file types to 265 named definitions and 678 patterns, a
  tested strict superset of `ignore` 0.4.31, with deterministic catalog
  introspection through `len()` and `names()`.
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
  small-tree, and privacy-safe real-repository benchmark profiles.
- Raise the Windows collected-walker ceiling while bounding shallow frontier
  expansion, improving broad and skewed traversal without a small-tree penalty.
- Add `PortableScanReport` with host-path, file-identity, and free-form
  diagnostic redaction plus a root-independent portable revision.
- Add `SnapshotContentProvider` with path-scope validation, bounded reads,
  before/after file-version checks, and optional SHA-256 verification.
- Add ordered parallel `MultiScanner` reports and deterministic,
  backpressured `Scanner::scan_into` content emission.
- Add repository-relative glob definitions for named file types and separate
  traversal/content worker budgets.
- Add versioned compact `ScanCache` persistence instead of requiring a full
  previous `ScanReport`.
- Expand randomized differential coverage against both `ignore` and the real
  `git check-ignore` implementation.
- Add explicit root-symlink follow/reject policy to walkers and scanners.
- Add a bounded parallel pull iterator with cooperative cancellation on drop.
- Add a stateful `FnMut` directory filter shared across `WalkBuilder` roots.
- Add watcher-event coalescing into path-safe deterministic cache invalidation
  or full-rescan plans.
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
- Verify exact selected-path parity on mixed-language repositories without
  publishing repository identities or paths.

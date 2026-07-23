# Changelog

All notable changes to this project are documented here.

## 0.1.1 - 2026-07-23

- Add iterative, lossless `Walker` and bounded `ParallelWalker` APIs.
- Continue after local filesystem errors with typed partial evidence.
- Add depth, open-handle, same-filesystem, and configurable symlink policies.
- Detect followed symlink cycles with native Unix and Windows file identities.
- Expand repository-local Git-ignore parity, diagnostics, and differential
  tests, including nested precedence and non-UTF8 paths.
- Remove per-entry shared-root contention and reuse a bounded worker pool.
- Bring parallel traversal within the same performance tier as `jwalk` and
  make ignore-aware scanning faster than `ignore` on the published benchmark.
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

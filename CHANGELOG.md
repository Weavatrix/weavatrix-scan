# Changelog

All notable changes to this project are documented here.

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

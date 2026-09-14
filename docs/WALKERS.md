# Walker and scanner method matrix

The capability table and historical measured rows that used to live at the
top of the README are kept here so the README can start with the three Scan
jobs. The numbers below are historical: `ignore` 0.4.31 and `jwalk` 0.8.1 on
the machines documented in the main README. Re-run before comparing
`ignore` 0.4.33 or `jwalk` 0.9.0.

Scan layers:

- `Walker`: iterative, streaming, lossless low-level traversal
- `WalkBuilder`: multi-root traversal, native sorting, directory filters
- `Scanner`: ignore-aware deterministic manifest
- `CompactScanReport`: the same selection with one retained root path
- `ScanSession`: retained snapshot plus watch-plan updates
- `RepositoryMatcher` / `SelectionMatcher`: reusable selection
- `ParallelWalker` / `ParallelRuntime`: bounded parallel traversal

See the README historical tables for the full capability grid and the
measured 1M-file Windows fixture. Those rows are not a cross-OS ranking.

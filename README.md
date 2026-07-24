# Weavatrix Scan

[![CI](https://github.com/sergii-ziborov/weavatrix-scan/actions/workflows/ci.yml/badge.svg)](https://github.com/sergii-ziborov/weavatrix-scan/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/weavatrix-scan.svg)](https://crates.io/crates/weavatrix-scan)
[![docs.rs](https://docs.rs/weavatrix-scan/badge.svg)](https://docs.rs/weavatrix-scan)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/sergii-ziborov/weavatrix-scan/blob/main/LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://github.com/sergii-ziborov/weavatrix-scan/blob/main/Cargo.toml)

`weavatrix-scan` is a deterministic, read-only repository scanner for static
analysis, code intelligence, indexing, and AI tooling.

It does more than walk a directory. A scan produces a stable manifest with
normalized paths, file sizes, optional content hashes, an aggregate revision,
and explicit evidence explaining why files were skipped. Linux and macOS builds
have zero mandatory runtime dependencies; Windows uses only `winapi-util` for
native volume and file identities.

## Why another repository walker?

`walkdir` and `jwalk` are excellent traversal libraries. `ignore` adds mature
Git-style filtering. Weavatrix Scan exposes five deliberately separate
layers:

- `Walker`: iterative, streaming, lossless low-level traversal;
- `WalkBuilder`: multi-root traversal, native sorting, directory filters, and
  contents-first ordering;
- `Scanner`: ignore-aware deterministic manifest, hashes, revision, and typed
  evidence;
- `RepositoryMatcher`: cached path selection for incremental consumers;
- `ParallelWalker`: bounded adaptive traversal for broad or skewed trees.

| Capability | weavatrix-scan | ignore | walkdir | jwalk |
| --- | :---: | :---: | :---: | :---: |
| Iterative traversal | Yes | Yes | Yes | Yes |
| Lossless native paths | Yes | Yes | Yes | Yes |
| Continue after local errors | Configurable | Yes | Yes | Yes |
| `max_depth` / bounded handles | Yes | Yes | Yes | Depth limit |
| Same-filesystem boundary | Yes | Yes | Yes | No |
| `.gitignore` hierarchy | Yes | Yes | No | No |
| Custom ignore files | Yes | Yes | No | No |
| Repository / Git-compatible ignore modes | Yes | Yes | No | No |
| Override globs / source switches | Yes | Yes | No | Directory callback |
| Reusable cached matcher | Yes | Yes | No | No |
| Multi-root / custom native sort | Yes / Yes | Yes / Yes | No / Yes | No / Yes |
| Directory callback / contents-first | Yes / Yes | Yes / Yes | Yes / Yes | Yes / No |
| Named file types | Yes | Yes | No | No |
| Stable normalized paths | Yes | No | No | Sorted traversal |
| Path-safe portable report | Yes | No | No | No |
| Snapshot-verified content provider | Yes | No | No | No |
| File sizes and SHA-256 hashes | Yes | No | No | No |
| Persistent incremental hash reuse | Yes | No | No | No |
| Concurrent-mutation evidence | Yes | No | No | No |
| Aggregate deterministic revision | Yes | No | No | No |
| Typed manifest delta / rename evidence | Yes | No | No | No |
| Binary and oversized-file policy | Yes | No | No | No |
| Typed skip reasons and warnings | Yes | No | No | No |
| Symlinks skipped by default / loop detection | Yes | Yes | Yes | Configurable |
| Parallel collected / streaming traversal | Yes / Yes | Yes / Yes | No | Yes / Yes |
| Parallel pull iterator | No (callback API) | No (callback API) | No | Yes |
| Parallel multi-root traversal | No | Yes | No | No |
| Stateful per-directory callback | No | No | No | Yes |
| Separate root-symlink policy | No | No | Yes | No |
| Cancellation and whole-scan budgets | Yes | Quit only | No | No |
| Minimum depth / hidden policy | Yes / Yes | Yes / Yes | Yes / No | Yes / Yes |
| Default runtime dependencies | 0 Unix / 1 Windows | Multiple | 2 platform helpers | Rayon stack |

Use `Walker` when you only need paths. Use `Scanner` when downstream results
must be reproducible and explainable.

The remaining `No` cells are API-shape differences rather than scanner
correctness gaps. `jwalk` uniquely offers a parallel pull iterator and mutable
per-directory state, `ignore` can share one parallel traversal across multiple
roots, and `walkdir` exposes a separate root-symlink switch. Weavatrix currently
uses a callback for parallel streaming, parallelizes one repository root at a
time, and canonicalizes the configured root. These are candidates for later API
work, but do not weaken deterministic manifests, ignore selection, or safety
evidence.

## Install

```toml
[dependencies]
weavatrix-scan = "0.2"
```

Enable serialization only when needed:

```toml
[dependencies]
weavatrix-scan = { version = "0.2", features = ["serde"] }
```

## Quick start

```rust
use weavatrix_scan::{ScanOptions, Scanner};

let options = ScanOptions::default()
    .with_extensions(["rs", "go", "ts", "py"])
    .with_parallelism(0);

let report = Scanner::new(".").options(options).scan()?;

println!("revision: {}", report.revision);
for file in &report.files {
    println!(
        "{}: {} bytes, hash={}",
        file.relative,
        file.bytes,
        file.content_hash.as_deref().unwrap_or("disabled")
    );
}
for skipped in &report.skipped {
    println!("skipped {}: {:?}", skipped.relative, skipped.kind);
}
# Ok::<(), weavatrix_scan::Error>(())
```

For the fastest path-only discovery, disable content reads:

```rust
use weavatrix_scan::{ScanOptions, Scanner};

let report = Scanner::new(".")
    .options(
        ScanOptions::default()
            .with_extensions(["rs", "go", "ts"])
            .metadata_only()
            .selected_files_only(),
    )
    .scan()?;

assert!(report.files.iter().all(|file| file.content_hash.is_none()));
# Ok::<(), weavatrix_scan::Error>(())
```

## Low-level walkers

`Walker` is a streaming iterator. It keeps paths as native `PathBuf`/`OsStr`
values, uses iterative DFS, bounds open directory handles, and yields local
errors according to policy:

```rust
use weavatrix_scan::{ErrorPolicy, WalkOptions, Walker};

let options = WalkOptions::default()
    .with_max_depth(Some(64))
    .with_max_open(8)
    .with_same_file_system(true)
    .with_error_policy(ErrorPolicy::Continue);

let mut walker = Walker::with_options(".", options)?;
while let Some(item) = walker.next() {
    match item {
        Ok(entry) => println!("{}", entry.path().display()),
        Err(error) => eprintln!("partial walk: {error}"),
    }
}
# Ok::<(), weavatrix_scan::WalkError>(())
```

The root is depth zero. After receiving a directory, callers can invoke
`skip_current_dir()` before requesting the next item. Symbolic links are not
followed by default; enabling `.with_follow_links(true)` keeps traversal inside
the root and reports loops as typed skip reasons.

`WalkBuilder` adds flexible low-level policies without changing the minimal
streaming `Walker`:

```rust
use weavatrix_scan::WalkBuilder;

let entries = WalkBuilder::new("repo-a")
    .add_root("repo-b")
    .sort_by_file_name()
    .filter_directories(|entry| entry.file_name() != "target")
    .contents_first(true)
    .build()
    .collect::<Result<Vec<_>, _>>()?;
# Ok::<(), weavatrix_scan::WalkError>(())
```

Custom sort callbacks receive native `OsStr` names, so sorting never requires
lossy UTF-8 conversion. Directory filters run before descent.

`ParallelWalker` adapts between low-overhead frontier lanes and dynamic
scheduling below narrow top-level trees:

```rust
use weavatrix_scan::ParallelWalker;

let report = ParallelWalker::new(".")
    .with_parallelism(0)
    .walk()?;
println!("entries={}, local_errors={}", report.entries.len(), report.errors.len());
# Ok::<(), weavatrix_scan::WalkError>(())
```

For pipelines that should parse entries immediately instead of collecting
them, `visit` invokes a thread-safe callback directly on traversal workers:

```rust
use weavatrix_scan::{ParallelWalker, WalkControl, WalkEvent};

let summary = ParallelWalker::new(".").visit(|event| match event {
    WalkEvent::Entry(entry) if entry.file_name() == "target" => WalkControl::Skip,
    WalkEvent::Entry(entry) => {
        println!("{}", entry.path().display());
        WalkControl::Continue
    }
    WalkEvent::Error(error) => {
        eprintln!("{error}");
        WalkControl::Continue
    }
})?;
# Ok::<(), weavatrix_scan::WalkError>(())
```

## Scan modes

The same scanner supports three useful cost levels:

| Mode | Configuration | Reads content | Skip evidence | Hashes content |
| --- | --- | :---: | :---: | :---: |
| Rich manifest | `ScanOptions::default()` | Yes | Complete | Yes |
| Safe discovery | `hash_file_contents = false` | First 8 KiB | Complete | No |
| Metadata only | `.metadata_only()` | No | Complete | No |
| Selected manifest | `.metadata_only().selected_files_only()` | No | Omitted | No |

Traversal and content inspection use bounded available parallelism by default.
Set
`.with_parallelism(1)` for a serial run or pass a fixed worker count when a
host application owns the wider scheduling policy.

## Output contract

`ScanReport` contains:

- `root`: canonical absolute repository root;
- `files`: stable, lexicographically sorted `ScannedFile` values;
- `skipped`: stable, sorted evidence for excluded entries;
- `warnings`: non-fatal ignore-file and local I/O diagnostics;
- `ignore_sources`: typed location and hash of every loaded selection input;
- `revision`: SHA-256 digest over ignore inputs, selected paths, optional content
  hashes, portability, and partial-termination state;
- `complete`: false when local errors made the evidence partial.
- `termination`: typed reason for a bounded or cancelled partial scan;
- `portable`: false when host-level Git configuration affected selection.
- `cache`: content reads and strong hashes reused by an incremental scan.

Each `ScannedFile` contains an absolute path, slash-normalized repository path,
byte size, optional `sha256:` content hash, and file-version evidence used to
validate persistent cache reuse. The scanner compares size, timestamps, native
file identity where available, and metadata before/after content reads. Native
paths remain lossless in the walker and absolute `PathBuf`; invalid Unicode
units in normalized manifest names are escaped (`%XX` on Unix, `%uXXXX` on
Windows) instead of being replaced with the lossy Unicode replacement marker.
With the `serde` feature, invalid native path units use a tagged byte/wide-unit
representation and round-trip without loss; ordinary Unicode paths remain
plain JSON strings.

## Portable evidence and verified content

Use `ScanReport::to_portable` before sending scan evidence to another process,
writing public logs, or attaching it to an AI request. `PortableScanReport`
omits the absolute root, absolute file paths, file identities, timestamps,
cache statistics, and free-form diagnostic text. Repository-relative paths and
typed skip kinds remain; diagnostic details are represented only by stable
SHA-256 values. External ignore-source locations are removed. Its
`selection_portable` field separately records whether host-level Git
configuration affected file selection.

Future content consumers such as Search or Clone can bind bytes back to the
full local report:

```rust
use weavatrix_scan::{Scanner, SnapshotEvidence};

let report = Scanner::new(".").scan()?;
let portable = report.to_portable();
let content = report
    .content_provider()?
    .read_bounded("src/lib.rs", 2 * 1024 * 1024)?;

assert!(!portable.revision.is_empty());
assert_eq!(content.evidence, SnapshotEvidence::Sha256);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`SnapshotContentProvider` accepts only sorted entries belonging to its report,
rejects path escapes and symlinks, enforces an optional byte limit, and compares
size plus native file-version evidence before and after reading. When a content
hash exists, returned bytes must also match the recorded SHA-256. A missing or
changed file produces typed `SnapshotReadError::Stale` evidence.

## Incremental consumers

Two completed reports produce a stable changed-file set without filesystem
access:

```rust
use weavatrix_scan::{DeltaQuality, Scanner};

let previous = Scanner::new(".").scan()?;
// Apply repository changes, then scan again.
let current = Scanner::new(".").scan_incremental(&previous)?;
let delta = current.delta_from(&previous);

assert!(matches!(
    delta.quality,
    DeltaQuality::ContentHash | DeltaQuality::Metadata | DeltaQuality::Partial
));
println!(
    "added={} modified={} removed={} renamed={}",
    delta.added.len(),
    delta.modified.len(),
    delta.removed.len(),
    delta.renamed.len()
);
# Ok::<(), weavatrix_scan::Error>(())
```

Unchanged files reuse prior SHA-256 values without reopening their content.
Reports from another root, legacy reports without version evidence, and files
whose size/version changed are read normally. Rename evidence is emitted only
when the same content hash is unique in both
manifests; duplicate-content moves remain explicit add/remove pairs instead of
being guessed. Metadata-only deltas compare size plus available file-version
evidence; callers that need content certainty should keep hashes enabled.
Partial scans always produce `DeltaQuality::Partial`.

Long-lived file watchers can keep a `RepositoryMatcher` and call `refresh()`
after an ignore input changes. Refresh builds a replacement matcher first, so a
failure leaves the existing matcher usable.

`SkipKind` distinguishes:

- `Binary`
- `ConcurrentModification`
- `Extension`
- `FileSystemBoundary`
- `Hidden`
- `Ignored`
- `IoError`
- `MaxDepth`
- `Override`
- `Oversized`
- `PathEscape`
- `ScanLimit`
- `StandardDirectory`
- `Symlink`
- `SymlinkLoop`

This distinction matters to analyzers: "not selected by policy" is different
from "unreadable" or "outside the repository."

## Configuration

`ScanOptions` exposes:

| Option | Default | Purpose |
| --- | --- | --- |
| `max_file_bytes` | 1,500,000 | Reject oversized source candidates |
| `extensions` | Empty | Empty accepts every extension |
| `file_types` | Empty | Named reusable extension groups, combined with `extensions` |
| `ignore_files` | `.gitignore`, `.ignore`, `.weavatrixignore` | Hierarchical local ignore files |
| `ignore_policy` | Repository-only | Optional parents, `.git/info/exclude`, global Git and explicit files |
| `override_rules` | Empty | Request-level include/exclude globs above ignore sources |
| `ignore_case_insensitive` | `false` | Optional ASCII case-insensitive ignore matching |
| `skip_hidden` | `false` | Skip dot-prefixed and Windows-hidden entries unless included |
| `standard_skips` | Enabled | Skip generated/vendor directories |
| `hash_file_contents` | `true` | Attach per-file hashes and content-sensitive revision |
| `detect_binary_files` | `true` | Reject files containing a NUL byte |
| `evidence` | `Complete` | Keep all typed exclusions, or only selected files |
| `parallelism` | `0` | Traversal/content workers; zero uses bounded available parallelism |
| `limits.max_entries` | None | Bound examined filesystem entries |
| `limits.max_total_bytes` | None | Deterministically bound selected content bytes |
| `limits.timeout` | None | Stop traversal/content inspection after a duration |
| `cancellation` | None | Cooperative cross-thread cancellation token |
| `walk.max_depth` | None | Limit entry depth; root is zero |
| `walk.min_depth` | `0` | Suppress shallower results while still traversing them |
| `walk.max_open` | `64` | Bound live directory handles/workers |
| `walk.same_file_system` | `false` | Stop at filesystem boundaries when enabled |
| `walk.follow_links` | `false` | Follow only in-root links and detect cycles |
| `walk.error_policy` | `Continue` | Continue with partial typed evidence or abort |
| `walk.collect_metadata` | `true` in `ScanOptions` | Reuse directory-entry metadata without reopening selected paths |

The standard directory policy skips:

```text
.git .hg .svn .venv __pycache__ build coverage dist
node_modules target vendor
```

Disable it when another layer owns generated-directory policy:

```rust
use weavatrix_scan::{ScanOptions, StandardSkips};

let mut options = ScanOptions::default();
options.standard_skips = StandardSkips::Disabled;
```

## Ignore semantics

Ignore files are loaded hierarchically with source precedence
`.weavatrixignore`/custom > `.ignore` > `.gitignore` >
`.git/info/exclude` > global Git. Deeper files win within the same source
class. Supported Git-style constructs include:

- comments and escaped leading `#` / `!`;
- negation with `!`;
- root-anchored patterns;
- directory-only patterns;
- `*`, `**`, and `?`;
- character classes, negated classes, and ranges;
- brace alternatives such as `{foo,bar}`;
- escaped literals and escaped trailing spaces.

The default scanner intentionally does not read global Git configuration,
parent rules outside the scan root, or `.git/info/exclude`; repository-local
selection therefore stays portable. `IgnorePolicy::git_compatible()` enables
all three explicitly inside Git repositories, records their content hashes,
honors unconditional and matching `includeIf` Git config includes, and marks
host-dependent reports non-portable. Local `.gitignore`, `.ignore`,
and custom sources can be toggled independently. Request-level override globs
use `ignore::Override` semantics: ordinary patterns include and leading `!`
patterns exclude. Explicit includes can opt paths back into standard-directory
and extension filtering, but never bypass size or binary safety checks.
`RepositoryMatcher::matched` exposes the winning typed
decision without requiring a full walk. Differential tests compare
exact selected path sets against the
`ignore` crate for anchored, nested, negated, wildcard, and character-class
fixtures plus 64-seed deterministic randomized rule sets. Stress cases cover
deep trees, permission errors, raw non-UTF8 ignore rules/names, percent escapes,
and followed symlink loops. The
differential suite and competitor crates are dev-only.

## Safety model

- never executes repository code;
- never starts subprocesses or accesses the network;
- canonicalizes and validates the root before traversal;
- does not follow symlink entries by default;
- rejects followed links outside the canonical root and detects cycles;
- can enforce a same-filesystem boundary;
- continues after independent local errors by default and marks the report
  partial;
- caps selected file size before content reads;
- rejects repository-local ignore-file symlinks and path traversal;
- supports entry, total-byte, timeout, and cooperative cancellation bounds;
- exports path-safe portable evidence without host paths or diagnostic text;
- revalidates snapshot content before and after bounded consumer reads;
- forbids unsafe Rust.

The scanner is read-only. Concurrent filesystem changes between discovery and
the final metadata check are surfaced as `ConcurrentModification` warnings and
skips under `Continue`, or as the first error under `Abort`.

## Benchmarks

Run all included benchmarks:

```sh
cargo bench --locked
```

Run the competitor comparison:

```sh
cargo bench --locked --bench compare_competitors
```

The `Competitor benchmarks` workflow runs the same output-equivalent comparison
on Ubuntu, Windows, and macOS for scanner or benchmark changes.

Run exact selected-path parity on a real repository:

```powershell
$env:WEAVATRIX_BENCH_ROOT = "C:\path\to\repository"
cargo bench --locked --bench real_repository
```

Run skewed, deep, first-touch, bounded-handle, large-content, and incremental
profiles:

```sh
cargo bench --locked --bench stress_profiles
```

The synthetic comparison uses 6,000 source files across Rust, Go, and
TypeScript in 80 sibling directories. It runs two warmups and 11 interleaved
measured samples, then reports the median. Raw walkers must produce the same
fully sorted native relative-path set; the ignore-aware comparison additionally
checks the same normalized path-and-size manifest.

Sample result on Windows 11, Rust 1.97.1, warm filesystem cache, measured
2026-07-24 against `ignore` 0.4.31, `walkdir` 2.5.0, and `jwalk` 0.8.1:

| Mode | Library | Files | Median |
| --- | --- | ---: | ---: |
| Raw paths | weavatrix `Walker` | 6,004 | 8.3 ms |
| Raw paths | weavatrix `ParallelWalker` | 6,004 | 5.8 ms |
| Raw paths | ignore | 6,004 | 9.6 ms |
| Raw paths | walkdir | 6,004 | 9.2 ms |
| Raw paths | jwalk | 6,004 | 7.5 ms |
| Ignore-aware manifest | weavatrix `Scanner` serial | 6,001 | 31.5 ms |
| Ignore-aware manifest | weavatrix `Scanner` parallel | 6,001 | 18.8 ms |
| Ignore-aware manifest | ignore | 6,001 | 37.4 ms |
| Rich SHA-256 manifest | weavatrix `Scanner` | 6,000 | 123.5 ms |

Each row is the median of five independent process medians. Every process runs
11 interleaved output-equivalent samples after two warmups. On this measurement
`ParallelWalker` was 23.2% faster than `jwalk`; the parallel
selected-manifest `Scanner` was 49.7% faster than `ignore`. The rich row
additionally reads content, detects binaries, computes SHA-256 hashes, captures
snapshot evidence, and records typed exclusions. Absolute timings vary by
filesystem, cache, antivirus, CPU, and operating system; the benchmark workflow
reruns the same checks on Ubuntu, Windows, and macOS.

Source review explains the remaining differences:

- `walkdir` streams unsorted directory entries and bounds open descriptors;
- `jwalk` schedules `read_dir` work through Rayon and restores ordered output;
- `ignore` compiles patterns into `GlobSet` matchers and shares inherited
  matchers;
- Weavatrix `Walker` streams iterative DFS, bounds live handles and buffers the
  oldest remaining frame only when `max_open` is reached;
- Weavatrix `ParallelWalker` expands a small shallow frontier for narrow roots,
  then uses up to 16 Windows or 8 Unix workers without serially over-expanding
  small trees; bounded lanes keep report order independent of worker completion;
- Weavatrix `Scanner` reuses inherited rules, indexes exact literals,
  specializes prefix/suffix globs, prefilters complex patterns, and sorts only
  the final report.

The optional real-repository benchmark keeps repository identities and paths
local; published documentation records only synthetic corpus results. It first
asserts the exact same sorted `(normalized path, bytes)` manifest. The stress
profile also measured a
skewed raw tree at 3.2 ms (`ParallelWalker`), 3.4 ms (`jwalk`), and 4.7 ms
(`walkdir`), while an unchanged 12 MiB SHA-256 manifest fell from 48.6 ms full
scan to 0.7 ms with incremental hash reuse. Treat these as reproducible
samples, not universal constants.

## Correctness checks

The test suite covers:

- deterministic results and revisions;
- ignore-rule precedence and nested ignore files;
- repository-only, Git-exclude, parent, explicit and reusable-matcher policies;
- representative and randomized parity with `ignore`;
- raw entry parity with `walkdir` and `jwalk`;
- iterative deep trees, bounded handles, local error continuation, non-UTF8
  paths, and symlink loops;
- concurrent mutation detection and same-size incremental changes;
- multi-root walking, named file types, custom native sorting, directory
  filtering, and contents-first ordering;
- binary, oversized, extension, generated-directory, and symlink policies;
- serial/parallel content-inspection equivalence;
- streaming parallel pruning and cancellation;
- manifest delta evidence and live matcher refresh;
- optional Serde support.

The real-repository benchmark compares the complete normalized selected-path
set against `ignore`. Its comparison policy disables Weavatrix's file-size cap
so an oversized file cannot masquerade as an ignore-rule mismatch.

## Development

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --no-deps --all-features
cargo bench --locked
cargo publish --locked --dry-run
```

The MSRV is Rust 1.88. CI checks Rust 1.88 on Linux, Windows, and macOS, with
stable test coverage on all three platforms.

## Relationship to Weavatrix

`weavatrix-scan` owns repository discovery. It does not parse languages or
build graphs. [`weavatrix-graph`](https://github.com/sergii-ziborov/weavatrix-graph)
owns typed graph primitives. Higher-level Weavatrix crates can compose both
without coupling either library to MCP, a CLI, or language-specific parsers.

## License

MIT © 2026 Sergii Ziborov.

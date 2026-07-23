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
Git-style filtering. Weavatrix Scan now exposes three deliberately separate
layers:

- `Walker`: iterative, streaming, lossless low-level traversal;
- `Scanner`: ignore-aware deterministic manifest, hashes, revision, and typed
  evidence;
- `ParallelWalker`: bounded parallel traversal for wide trees.

| Capability | weavatrix-scan | ignore | walkdir | jwalk |
| --- | :---: | :---: | :---: | :---: |
| Iterative traversal | Yes | Yes | Yes | Yes |
| Lossless native paths | Yes | Yes | Yes | Yes |
| Continue after local errors | Configurable | Yes | Yes | Yes |
| `max_depth` / bounded handles | Yes | Yes | Yes | Depth limit |
| Same-filesystem boundary | Yes | No | Yes | No |
| `.gitignore` hierarchy | Yes | Yes | No | No |
| Custom ignore files | Yes | Yes | No | No |
| Stable normalized paths | Yes | No | No | Sorted traversal |
| File sizes and content hashes | Yes | No | No | No |
| Aggregate deterministic revision | Yes | No | No | No |
| Binary and oversized-file policy | Yes | No | No | No |
| Typed skip reasons and warnings | Yes | No | No | No |
| Symlinks skipped by default / loop detection | Yes | Yes | Yes | Configurable |
| Parallel traversal / content inspection | Yes / Yes | Yes / N/A | No | Yes |
| Default runtime dependencies | 0 Unix / 1 Windows | Multiple | 2 platform helpers | Rayon stack |

Use `Walker` when you only need paths. Use `Scanner` when downstream results
must be reproducible and explainable.

## Install

```toml
[dependencies]
weavatrix-scan = "0.1"
```

Enable serialization only when needed:

```toml
[dependencies]
weavatrix-scan = { version = "0.1", features = ["serde"] }
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
            .metadata_only(),
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

`ParallelWalker` is a separate collected mode for wide directory frontiers:

```rust
use weavatrix_scan::ParallelWalker;

let report = ParallelWalker::new(".")
    .with_parallelism(0)
    .walk()?;
println!("entries={}, local_errors={}", report.entries.len(), report.errors.len());
# Ok::<(), weavatrix_scan::WalkError>(())
```

## Scan modes

The same scanner supports three useful cost levels:

| Mode | Configuration | Reads content | Detects binary | Hashes content |
| --- | --- | :---: | :---: | :---: |
| Rich manifest | `ScanOptions::default()` | Yes | Yes | Yes |
| Safe discovery | `hash_file_contents = false` | First 8 KiB | Yes | No |
| Metadata only | `.metadata_only()` | No | No | No |

Content inspection uses available CPU parallelism by default. Set
`.with_parallelism(1)` for a serial run or pass a fixed worker count when a
host application owns the wider scheduling policy.

## Output contract

`ScanReport` contains:

- `root`: canonical absolute repository root;
- `files`: stable, lexicographically sorted `ScannedFile` values;
- `skipped`: stable, sorted evidence for excluded entries;
- `warnings`: non-fatal ignore-file and local I/O diagnostics;
- `revision`: FNV-1a digest over selected relative paths and optional content
  hashes;
- `complete`: false when local errors made the evidence partial.

Each `ScannedFile` contains an absolute path, slash-normalized repository path,
byte size, and optional content hash. Default hashes are deterministic FNV-1a
digests intended for change detection, not cryptographic verification. Native
paths remain lossless in the walker and absolute `PathBuf`; invalid Unicode
units in normalized manifest names are escaped (`%XX` on Unix, `%uXXXX` on
Windows) instead of being replaced with the lossy Unicode replacement marker.
With the `serde` feature, invalid native path units use a tagged byte/wide-unit
representation and round-trip without loss; ordinary Unicode paths remain
plain JSON strings.

`SkipKind` distinguishes:

- `Binary`
- `Extension`
- `FileSystemBoundary`
- `Ignored`
- `IoError`
- `MaxDepth`
- `Oversized`
- `PathEscape`
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
| `ignore_files` | `.gitignore`, `.weavatrixignore` | Hierarchical local ignore files |
| `ignore_case_insensitive` | `false` | Optional ASCII case-insensitive ignore matching |
| `standard_skips` | Enabled | Skip generated/vendor directories |
| `hash_file_contents` | `true` | Attach per-file hashes and content-sensitive revision |
| `detect_binary_files` | `true` | Reject files containing a NUL byte |
| `parallelism` | `0` | Content workers; zero uses available parallelism |
| `walk.max_depth` | None | Limit entry depth; root is zero |
| `walk.max_open` | `64` | Bound live directory handles/workers |
| `walk.same_file_system` | `false` | Stop at filesystem boundaries when enabled |
| `walk.follow_links` | `false` | Follow only in-root links and detect cycles |
| `walk.error_policy` | `Continue` | Continue with partial typed evidence or abort |
| `walk.collect_metadata` | `true` in `ScanOptions` | Capture size during traversal and avoid a second metadata pass |

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

Ignore files are loaded hierarchically. Later matching rules win. Supported
Git-style constructs include:

- comments and escaped leading `#` / `!`;
- negation with `!`;
- root-anchored patterns;
- directory-only patterns;
- `*`, `**`, and `?`;
- character classes, negated classes, and ranges;
- brace alternatives such as `{foo,bar}`;
- escaped literals and escaped trailing spaces.

The scanner intentionally does not read global Git configuration or
`.git/info/exclude`. Repository-local selection therefore stays portable across
machines. Differential tests compare exact selected path sets against the
`ignore` crate for anchored, nested, negated, wildcard, and character-class
fixtures plus deterministic randomized rule sets. Stress cases cover deep
trees, permission errors, non-UTF8 names, and followed symlink loops. The
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
- forbids unsafe Rust.

The scanner is read-only. Concurrent filesystem changes are surfaced as local
warnings/skips under `Continue` or as the first error under `Abort`.

## Benchmarks

Run all included benchmarks:

```sh
cargo bench --locked
```

Run the competitor comparison:

```sh
cargo bench --locked --bench compare_competitors
```

Run exact selected-path parity on a real repository:

```powershell
$env:WEAVATRIX_BENCH_ROOT = "C:\path\to\repository"
cargo bench --locked --bench real_repository
```

The synthetic comparison uses 6,000 source files across Rust, Go, and
TypeScript in 80 sibling directories. It runs two warmups and 11 interleaved
measured samples, then reports the median. Raw walkers must produce the same
fully sorted native relative-path set; the ignore-aware comparison additionally
checks the same normalized path-and-size manifest.

Sample result on Windows 11, Rust 1.97.1, warm filesystem cache:

| Mode | Library | Files | Median |
| --- | --- | ---: | ---: |
| Raw paths | weavatrix `Walker` | 6,004 | 18.9 ms |
| Raw paths | weavatrix `ParallelWalker` | 6,004 | 15.6 ms |
| Raw paths | ignore | 6,004 | 21.1 ms |
| Raw paths | walkdir | 6,004 | 20.2 ms |
| Raw paths | jwalk | 6,004 | 15.4 ms |
| Ignore-aware manifest | weavatrix `Scanner` | 6,001 | 43.5 ms |
| Ignore-aware manifest | ignore | 6,001 | 51.0 ms |
| Rich manifest | weavatrix `Scanner` | 6,000 | 109.2 ms |

This is the median of three independent output-equivalent Windows benchmark
processes. `Walker` is in the same performance tier as `walkdir`;
`ParallelWalker` is within 2% of `jwalk` on this wide corpus. The ignore-aware
`Scanner` is about 15% faster than `ignore` here while also recording typed
skip/warning evidence. The rich row additionally reads content, detects
binaries, hashes sources, and computes a deterministic revision.

Source review explains the remaining differences:

- `walkdir` streams unsorted directory entries and bounds open descriptors;
- `jwalk` schedules `read_dir` work through Rayon and restores ordered output;
- `ignore` compiles patterns into `GlobSet` matchers and shares inherited
  matchers;
- Weavatrix `Walker` streams iterative DFS, bounds live handles and buffers the
  oldest remaining frame only when `max_open` is reached;
- Weavatrix `Scanner` reuses inherited rules, indexes exact literals,
  specializes prefix/suffix globs, prefilters complex patterns, and sorts only
  the final report.

Exact-path real-repository sample:

| Repository | Files | weavatrix-scan | ignore |
| --- | ---: | ---: | ---: |
| weavatrix-scan | 36 | 3.4 ms | 4.2 ms |

Timing varies by filesystem, cache, antivirus, and CPU. Treat the table as a
reproducible sample, not a universal constant.

## Correctness checks

The test suite covers:

- deterministic results and revisions;
- ignore-rule precedence and nested ignore files;
- representative and randomized parity with `ignore`;
- raw entry parity with `walkdir` and `jwalk`;
- iterative deep trees, bounded handles, local error continuation, non-UTF8
  paths, and symlink loops;
- binary, oversized, extension, generated-directory, and symlink policies;
- serial/parallel content-inspection equivalence;
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

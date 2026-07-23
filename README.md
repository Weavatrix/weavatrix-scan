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
and explicit evidence explaining why files were skipped. The default build has
zero runtime dependencies.

## Why another repository walker?

`walkdir` and `jwalk` are excellent traversal libraries. `ignore` adds mature
Git-style filtering. `weavatrix-scan` targets the next layer: the repeatable,
auditable source manifest an analyzer needs before parsing begins.

| Capability | weavatrix-scan | ignore | walkdir | jwalk |
| --- | :---: | :---: | :---: | :---: |
| Recursive traversal | Yes | Yes | Yes | Yes |
| `.gitignore` hierarchy | Yes | Yes | No | No |
| Custom ignore files | Yes | Yes | No | No |
| Stable normalized paths | Yes | No | No | Sorted traversal |
| File sizes and content hashes | Yes | No | No | No |
| Aggregate deterministic revision | Yes | No | No | No |
| Binary and oversized-file policy | Yes | No | No | No |
| Typed skip reasons and warnings | Yes | No | No | No |
| Symlinks skipped by default | Yes | Configurable | Configurable | Configurable |
| Parallel content inspection | Yes | Parallel walk available | No | Yes |
| Default runtime dependencies | 0 | Multiple | 2 platform helpers | Rayon stack |

Choose a traversal crate when you only need paths. Choose `weavatrix-scan` when
downstream results must be reproducible and explainable.

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
- `warnings`: non-fatal ignore-file diagnostics;
- `revision`: FNV-1a digest over selected relative paths and optional content
  hashes.

Each `ScannedFile` contains an absolute path, slash-normalized repository path,
byte size, and optional content hash. Default hashes are deterministic FNV-1a
digests intended for change detection, not cryptographic verification.

`SkipKind` distinguishes:

- `Binary`
- `Extension`
- `Ignored`
- `Oversized`
- `PathEscape`
- `StandardDirectory`
- `Symlink`

This distinction matters to analyzers: "not selected by policy" is different
from "unreadable" or "outside the repository."

## Configuration

`ScanOptions` exposes:

| Option | Default | Purpose |
| --- | --- | --- |
| `max_file_bytes` | 1,500,000 | Reject oversized source candidates |
| `extensions` | Empty | Empty accepts every extension |
| `ignore_files` | `.gitignore`, `.weavatrixignore` | Hierarchical local ignore files |
| `standard_skips` | Enabled | Skip generated/vendor directories |
| `hash_file_contents` | `true` | Attach per-file hashes and content-sensitive revision |
| `detect_binary_files` | `true` | Reject files containing a NUL byte |
| `parallelism` | `0` | Zero uses available parallelism |

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
- escaped literals and escaped trailing spaces.

The scanner intentionally does not read global Git configuration or
`.git/info/exclude`. Repository-local selection therefore stays portable across
machines. Differential tests compare exact selected path sets against the
`ignore` crate for anchored, nested, negated, wildcard, and character-class
fixtures.

## Safety model

- never executes repository code;
- never starts subprocesses or accesses the network;
- canonicalizes and validates the root before traversal;
- does not follow symlink entries;
- rejects paths outside the canonical root;
- caps selected file size before content reads;
- forbids unsafe Rust.

The scanner is read-only. As with ordinary filesystem walkers, callers should
avoid concurrently replacing directories while a scan is running.

## Benchmarks

Run all included benchmarks:

```sh
cargo bench --locked
```

Run the competitor comparison:

```sh
cargo bench --locked --bench compare_competitors
```

Run exact-count parity on a real repository:

```powershell
$env:WEAVATRIX_BENCH_ROOT = "C:\path\to\repository"
cargo bench --locked --bench real_repository
```

The synthetic comparison uses 6,000 source files across Rust, Go, and
TypeScript. It runs two warmups and 11 interleaved measured samples, then
reports the median. Every competitor must return the same file count in each
comparable mode.

Sample result on Windows 11, Rust 1.97.1, warm filesystem cache:

| Mode | Library | Files | Median |
| --- | --- | ---: | ---: |
| Raw discovery | weavatrix-scan | 6,004 | 19.1 ms |
| Raw discovery | ignore | 6,004 | 7.8 ms |
| Raw discovery | walkdir | 6,004 | 8.6 ms |
| Raw discovery | jwalk | 6,004 | 5.5 ms |
| Ignore-aware discovery | weavatrix-scan | 6,001 | 32.2 ms |
| Ignore-aware discovery | ignore | 6,001 | 32.5 ms |
| Rich manifest | weavatrix-scan | 6,000 | 150.0 ms |

The raw row is deliberately honest: `jwalk` is the fastest choice when sorted
paths are the whole requirement. The rich-manifest row has no direct equivalent
in those walkers; it additionally reads, validates, hashes, sorts, and records
evidence for all selected files.

Timing varies by filesystem, cache, antivirus, and CPU. Treat the table as a
reproducible sample, not a universal constant.

## Correctness checks

The test suite covers:

- deterministic results and revisions;
- ignore-rule precedence and nested ignore files;
- parity with `ignore` on representative Git-style patterns;
- binary, oversized, extension, generated-directory, and symlink policies;
- serial/parallel content-inspection equivalence;
- optional Serde support.

The real-repository benchmark currently confirms equal ignore-aware selected
counts against `ignore` on `radiochron`, `weavatrix-graph`, Weavatrix, and
Analytics fixtures used during development.

## Development

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --no-deps --all-features
cargo bench --locked
cargo publish --locked --dry-run
```

The MSRV is Rust 1.88. CI checks stable Rust plus Rust 1.88 on Linux and Windows.

## Relationship to Weavatrix

`weavatrix-scan` owns repository discovery. It does not parse languages or
build graphs. [`weavatrix-graph`](https://github.com/sergii-ziborov/weavatrix-graph)
owns typed graph primitives. Higher-level Weavatrix crates can compose both
without coupling either library to MCP, a CLI, or language-specific parsers.

## License

MIT © 2026 Sergii Ziborov.

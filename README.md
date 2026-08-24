# Weavatrix Scan

[![CI](https://github.com/Weavatrix/weavatrix-scan/actions/workflows/ci.yml/badge.svg)](https://github.com/Weavatrix/weavatrix-scan/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/weavatrix-scan.svg)](https://crates.io/crates/weavatrix-scan)
[![docs.rs](https://docs.rs/weavatrix-scan/badge.svg)](https://docs.rs/weavatrix-scan)
[![npm](https://img.shields.io/npm/v/weavatrix-scan.svg)](https://www.npmjs.com/package/weavatrix-scan)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Weavatrix/weavatrix-scan/blob/main/LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.88-blue.svg)](https://github.com/Weavatrix/weavatrix-scan/blob/main/Cargo.toml)

The filesystem-evidence layer of the [Weavatrix ecosystem](https://weavatrix.com/ecosystem).

`weavatrix-scan` is a deterministic, read-only repository scanner for static
analysis, code intelligence, indexing, and AI tooling.

It does more than walk a directory. A scan produces a stable manifest with
normalized paths, file sizes, optional content hashes, an aggregate revision,
and explicit evidence explaining why files were skipped. Linux and macOS builds
have zero mandatory runtime dependencies; Windows uses only `winapi-util` for
native volume and file identities.

## Why another repository walker?

[`walkdir`](https://docs.rs/walkdir/latest/walkdir/struct.WalkDir.html) and
[`jwalk`](https://docs.rs/jwalk/latest/jwalk/struct.WalkDirGeneric.html) are
excellent traversal libraries.
[`ignore`](https://docs.rs/ignore/latest/ignore/struct.WalkBuilder.html) adds
mature Git-style filtering. Weavatrix Scan exposes deliberately separate
layers:

- `Walker`: iterative, streaming, lossless low-level traversal;
- `WalkBuilder`: multi-root traversal, native sorting, directory filters, and
  contents-first ordering;
- `Scanner`: ignore-aware deterministic manifest, hashes, revision, typed
  evidence, and bounded one-pass selected-content callbacks;
- `CompactScanReport`: the same deterministic selection and revision with one
  retained root path and optional boxed rich evidence for million-file
  manifests;
- `MultiScanner`: ordered concurrent manifests and a globally bounded
  multi-root content pipeline;
- `ParallelMultiWalker`: collected or direct-streaming traversal across
  concurrent raw roots with ordered per-root reports;
- `RepositoryMatcher`: cached path selection for incremental consumers;
- `SelectionMatcher`: the complete scanner selection policy as a reusable,
  typed standalone matcher;
- `ParallelWalker`: bounded adaptive traversal for broad or skewed trees.
- `ParallelRuntime`: process-global, dedicated, or application-owned execution
  shared by walker and scanner APIs.

Within the wider Weavatrix stack, this crate owns repository discovery and
selection, [`weavatrix-parse`](https://github.com/Weavatrix/weavatrix-parse)
owns dependency-free source tokenization and structural facts, and
[`weavatrix-rust`](https://github.com/Weavatrix/weavatrix-rust) composes
the scan and analysis layers. Go and Node walkers below are performance and
capability controls, not proposed alternate implementations of that product
pipeline.

### Competitive position

Against the versions tested in this repository (`ignore` 0.4.31, `walkdir`
2.5.0, `jwalk` 0.8.1, and `dirwalk` 1.1.1), Weavatrix Scan is the strongest
overall fit when the output must be a deterministic, explainable code-scanner
manifest rather than only a stream of directory entries. It is not the
universal winner for every walker workload:

| Workload | Strongest fit in this comparison | Why |
| --- | --- | --- |
| Deterministic code-scanner manifest | **Weavatrix Scan** | Only entry with normalized paths, hashes, aggregate revision, typed skips, portable evidence, incremental cache, and changed-path updates |
| Raw parallel streaming | **Weavatrix `ParallelWalker`** | 264.7 ms and 7.6 MiB peak on the measured 1,000,000-file Windows fixture |
| Fixed real-corpus raw traversal | **dirwalk / jwalk** | On the pinned Rust checkout with a warm Windows cache, `dirwalk` measured 123.3 ms and `jwalk` 161.2 ms; Weavatrix collected `ParallelWalker` measured 285.9 ms |
| Minimal serial traversal | **walkdir / Weavatrix `Walker`** | walkdir remains the small established primitive; Walker measured 546.3 ms versus 584.5 ms with comparable 5 MiB-class memory |
| Memory-efficient deterministic manifest | **Weavatrix `CompactScanReport`** | Exact output parity at 1,019.4 ms / 63.2 MiB; `ignore` used 56.4 MiB but took 2,106.7 ms and does not produce revision/evidence |
| Reusable selection matcher | **Weavatrix / ignore** | Weavatrix combines ignore, overrides, types, depth, size, symlink and filesystem policy with typed outcomes; `ignore` exposes modular matcher builders |
| Host-owned parallel scheduling | **Weavatrix / jwalk** | Weavatrix accepts any fallible executor, optionally wraps existing/new Rayon pools directly, and preserves busy-timeout policy |

| Capability | weavatrix-scan | ignore | walkdir | jwalk |
| --- | :---: | :---: | :---: | :---: |
| Iterative traversal | Yes | Yes | Yes | Yes |
| Single-file root | Yes | Yes | Yes | Yes |
| Lossless native paths | Yes | Yes | Yes | Yes |
| Continue after local errors | Configurable | Yes | Yes | Yes |
| Depth / open-handle control | Depth + configurable `max_open` | Depth + internally bounded | Depth + configurable `max_open` | Depth + directory scheduler |
| Same-filesystem boundary | Yes | Yes | Yes | No |
| `.gitignore` hierarchy | Yes | Yes | No | No |
| Custom ignore files | Yes | Yes | No | No |
| Repository / Git-compatible ignore modes | Yes | Yes | No | No |
| Override globs / source switches | Yes | Yes | No | No |
| Reusable cached full selection matcher | Yes | Yes | No | No |
| Multi-root / custom sort | Serial + parallel / full `DirEntry` | Yes / name or path, serial only | No / full `DirEntry` | No / mutable directory batch |
| Directory callback / contents-first | Parallel typed batch / Yes | Filter only / No | Filter / Yes | Parallel typed batch / No |
| Built-in types / composition / negation | 265 / Yes / Yes | 224 / Yes / Yes | No | No |
| Stable normalized paths | Yes | No | No | No |
| Path-safe portable report | Yes | No | No | No |
| Snapshot-verified content provider | Yes | No | No | No |
| File sizes and SHA-256 hashes | Yes | No | No | No |
| Versioned compact incremental cache | Yes | No | No | No |
| Watcher events to changed-path manifest update | Yes | No | No | No |
| Optional direct `notify` adapter | Yes | No | No | No |
| Concurrent-mutation evidence | Yes | No | No | No |
| Aggregate deterministic revision | Yes | No | No | No |
| Typed manifest delta / rename evidence | Yes | No | No | No |
| Binary and oversized-file policy | Yes | No | No | No |
| Typed skip reasons and warnings | Yes | No | No | No |
| Symlinks skipped by default / loop detection | Yes | Yes | Yes | Configurable |
| Parallel collected / streaming traversal | Yes / visitor + bounded pull | No / callback | No / serial iterator | No / ordered iterator |
| Deterministic backpressured scan sink | Yes | No | No | No |
| Parallel one-pass verified content callback | Yes | No | No | No |
| Multi-root verified content callback | Yes | No | No | No |
| Changed-path-only content callback | Yes | No | No | No |
| Streaming content without retained manifest | Yes | No | No | No |
| Parallel pull iterator | Bounded unordered / ordered DFS | No (callback API) | No | Ordered DFS |
| Parallel multi-root raw traversal | Collected + streaming | Streaming callback | No | No |
| Parallel multi-root manifest scanner | Yes | No | No | No |
| Stateful per-directory batch | Parallel ordered, typed | No | No | Parallel ordered, typed |
| Redirected-stdout protection | Yes | Yes | No | No |
| Separate root-symlink policy | Yes | No | Yes | No |
| Cancellation and whole-scan budgets | Yes | Quit only | No | No |
| Minimum depth / hidden policy | Yes / Yes | Yes / Yes | Yes / No | Yes / Yes |
| Existing/dedicated worker pool | Generic external / owned | Internal threads | Not applicable | Rayon existing / new |
| Busy timeout / fallible submission | External contract / Yes | No / No | Not applicable | Yes / Yes |
| Measured 1M raw time / peak | **264.7 ms / 7.6 MiB** | Not raw-equivalent | 584.5 ms / **4.6 MiB** | 313.1 ms / 159.7 MiB |
| Measured 833k manifest time / peak | **Compact: 1,019.4 ms / 63.2 MiB** | 2,106.7 ms / **56.4 MiB** | Not a scanner | Not a scanner |
| Default runtime dependencies | 0 Unix / 1 Windows | Multiple | 2 platform helpers | Rayon stack |

Use `Walker` when you only need paths. Use `Scanner` when downstream results
must be reproducible and explainable.

### Remaining competitive boundaries

The functional gaps in retained-manifest memory, embeddable scheduling,
parallel stateful batches, multi-root streaming, and traversal-free changed
content are now closed. The remaining differences are evidence and ecosystem
boundaries:

1. **Matcher production history.** `ignore` remains the established
   Git-ignore implementation. Weavatrix checks representative, randomized,
   arbitrary-byte, and million-file exact-manifest parity, but does not claim
   equal ecosystem age.
2. **Cross-platform million-file evidence.** CI and normal benchmarks cover
   Linux, Windows, and macOS, while the opt-in million-file RSS result below
   has so far been measured only on Windows.
3. **Raw traversal ceiling.**
   [`dirwalk`](https://docs.rs/dirwalk/latest/dirwalk/) and `jwalk` lead the
   fixed real-corpus raw measurement below. Go
   [`fastwalk`](https://pkg.go.dev/github.com/charlievieth/fastwalk) remains an
   additional native competitor. Go
   [`gocodewalker`](https://pkg.go.dev/github.com/boyter/gocodewalker) is a
   closer ignore-aware comparison, while
   [`parawalk`](https://docs.rs/parawalk/latest/parawalk/) and Node
   [`fdir`](https://www.npmjs.com/package/fdir) are raw traversal controls.
   None produces Weavatrix's revision, portable report, typed skip evidence,
   snapshot validation, or incremental cache.

The million-file result establishes top-tier performance on the measured
Windows fixture, not a universal cross-platform ranking. The regular benchmark
workflow still validates smaller output-equivalent corpora on Linux, Windows,
and macOS; the million-file profile remains opt-in.

## Install

```toml
[dependencies]
weavatrix-scan = "0.4"
```

Enable serialization only when needed:

```toml
[dependencies]
weavatrix-scan = { version = "0.4", features = ["serde"] }
```

Enable direct conversion from `notify::Event` without making a watcher runtime
mandatory for other users:

```toml
weavatrix-scan = { version = "0.4", features = ["notify"] }
```

Enable direct existing/new Rayon pool integration without changing the default
scheduler:

```toml
weavatrix-scan = { version = "0.4", features = ["rayon"] }
```

The default build has no third-party runtime dependency on Unix. Windows uses
the small `winapi-util` safe wrapper for file, volume, and redirected-stdout
identity because the equivalent `std` by-handle identity APIs remain unstable
on the Rust 1.88 MSRV. Reimplementing that layer locally would require unsafe
WinAPI FFI and still depend on Windows bindings; this crate keeps
`unsafe_code = "forbid"`.

## Node.js and Bun

The `weavatrix-scan` npm package exposes the same Rust scanner through
Node-API. It is not a JavaScript port and does not execute repository code.
The async entrypoint runs outside the JavaScript event loop:

```console
npm install weavatrix-scan
# or: bun add weavatrix-scan
```

```js
const { scanRepository } = require('weavatrix-scan')

const report = await scanRepository(process.cwd(), {
  extensions: ['js', 'ts', 'rs'],
  selectedFilesOnly: true,
})
```

One package supports Node.js 18+ and Bun 1.4+ with native targets for Windows,
macOS, and glibc Linux on x64 and arm64. The
[Node/Bun benchmark report](node/benchmark/RESULTS.md) publishes both the raw
walker loss and the equal path-plus-size scanner result: `fdir` won the
paths-only rows by 1.68x-1.82x, while Weavatrix won the path-plus-size rows by
8.11x-11.57x on the disclosed 20k-file fixture. This npm library is a
separately released Scan product; it does not belong to Online or MCP.

## Rust quick start

```rust
use weavatrix_scan::{ScanOptions, Scanner};

let options = ScanOptions::default()
    .with_extensions(["rs", "go", "ts", "py"])
    .with_parallelism(0);

let report = Scanner::new(".").options(options).scan()?;

println!("{}", report.summary());
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
    .scan_compact()?;

assert!(report.files.iter().all(|file| file.content.is_none()));
# Ok::<(), weavatrix_scan::Error>(())
```

`scan_compact` discovers directly into root-shared records; it does not first
build and convert a full report. Use `report.absolute_path(file)` only for the
entries that need an owned absolute path. Rich compact scans retain hashes in
optional boxed content evidence, accessible with `file.content_hash()`.

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
the root and reports loops as typed skip reasons. The root itself follows a
separate `RootSymlinkPolicy`: `Follow` is the compatibility default and
`Reject` prevents an explicitly supplied symlink root from being opened.

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

`sort_by` receives complete `std::fs::DirEntry` values, including path,
file type, and metadata access. `sort_by_name` remains the allocation-free
native `OsStr` comparator, so sorting never requires lossy UTF-8 conversion.
Low-level walkers accept either a directory or one file as the root.
`skip_stdout(true)` prevents a redirected output file inside the tree from
feeding back into a command that is scanning it. `ParallelWalker` applies the
same option to collected, visitor, unordered-pull, and ordered-pull traversal;
`ParallelMultiWalker` applies it to every root. Directory filters run before
descent.
`filter_directories_stateful` accepts `FnMut`, serializes callback access, and
keeps one captured state across every root in the builder. For batch mutation
and typed state propagation, `StatefulWalkBuilder<R, E>::process_read_dir`
receives all immediate children before they are yielded. It can reorder or
retain the batch, mutate `R` inherited by child directories, attach `E` to
entries, and disable descent per entry.

```rust
use weavatrix_scan::StatefulWalkBuilder;

let entries = StatefulWalkBuilder::<usize, usize>::new(".", 0)
    .with_parallelism(0)
    .process_read_dir(|_, _, depth_state, entries| {
        *depth_state += 1;
        for entry in entries.iter_mut().filter_map(|item| item.as_mut().ok()) {
            entry.state = *depth_state;
        }
    })
    .build_parallel_ordered(1024)?
    .collect::<Result<Vec<_>, _>>()?;
# Ok::<(), weavatrix_scan::WalkError>(())
```

The parallel form runs each complete directory batch on the configured
runtime, propagates callback-mutated state to child tasks, and yields strict
DFS order under bounded backpressure. `build()` remains the zero-coordinator
serial iterator.

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

Applications can isolate scans or reuse their own scheduler:

```rust
use weavatrix_scan::{ParallelRuntime, ParallelWalker, Scanner};

let runtime = ParallelRuntime::dedicated(8)?;
let raw = ParallelWalker::new(".")
    .runtime(runtime.clone())
    .walk()?;
let manifest = Scanner::new(".")
    .runtime(runtime)
    .scan_compact()?;

assert!(!raw.entries.is_empty());
println!("selected={}", manifest.files.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

`ParallelRuntime::external` accepts an `Arc<dyn ParallelExecutor>`. The
executor receives each boxed job and the optional busy timeout, and can reject
submission with `io::Error`; traversal reports that as
`WalkOperation::ScheduleWorker` without waiting for an unsubmitted worker.
With the optional `rayon` feature,
`ParallelRuntime::{rayon_existing, rayon_new}` provides the same contract
without requiring an application wrapper. Busy timeout cancels a queued job if
the Rayon pool does not start it before the deadline.

Consumers that prefer pull semantics can use a bounded iterator. A full buffer
applies backpressure to traversal workers, and dropping the iterator cancels and
joins its coordinator:

```rust
use weavatrix_scan::ParallelWalker;

for entry in ParallelWalker::new(".").into_iter_bounded(1024) {
    println!("{}", entry?.path().display());
}
# Ok::<(), weavatrix_scan::WalkError>(())
```

Use `into_iter_ordered_bounded` when consumers require strict deterministic DFS
ordering. It prefetches directory reads in parallel while preserving the
configured output capacity and `max_open` bound. Both pull modes cancel and
join their coordinator when dropped. `try_into_iter_bounded` and
`try_into_iter_ordered_bounded` report coordinator thread creation failures
instead of panicking before traversal starts.

Larger bounded buffers improve throughput without changing the memory bound.
Very small capacities are useful when minimum buffered state matters more than
raw traversal speed.

`ParallelMultiWalker::visit` applies the same direct callback contract across
multiple roots. Callback order is intentionally concurrent, every event is
tagged with its root insertion index, and the returned reports stay in root
insertion order:

```rust
use weavatrix_scan::{
    ParallelMultiWalker, WalkControl, WalkEvent,
};

let summary = ParallelMultiWalker::new("repo-a")
    .add_root("repo-b")
    .with_root_parallelism(2)
    .visit(|event| match event.event {
        WalkEvent::Entry(entry) if entry.file_name() == "target" => {
            WalkControl::Skip
        }
        WalkEvent::Entry(_) | WalkEvent::Error(_) => WalkControl::Continue,
    })?;

println!("roots={}, visited={}", summary.len(), summary.visited());
# Ok::<(), weavatrix_scan::WalkError>(())
```

`WalkControl::Quit` cooperatively cancels every active root. The cancellable
form shares one `CancellationToken`; `skip_stdout`, traversal limits, error
policy, and the selected `ParallelRuntime` apply to every root.

Parallel callbacks may start another walk using the same runtime. Such
reentrant walks fall back to the iterative serial engine instead of waiting on
workers that they already occupy. A callback panic stops and wakes the dynamic
scheduler, is resumed on the caller, and leaves global, dedicated, or external
execution reusable.

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
host application owns the wider scheduling policy. Use
`.with_traversal_parallelism(...)` and `.with_content_parallelism(...)` when
directory latency and content hashing need separate budgets.

For many independent roots, `MultiScanner` scans roots concurrently while
returning reports in insertion order:

```rust
use weavatrix_scan::{MultiScanner, ScanOptions};

let reports = MultiScanner::new("workspace-a")
    .add_root("workspace-b")
    .options(ScanOptions::default().with_extensions(["rs", "go", "ts"]))
    .with_root_parallelism(2)
    .scan()?;

assert_eq!(reports.len(), 2);
# Ok::<(), weavatrix_scan::Error>(())
```

`Scanner::scan_into` keeps deterministic discovery metadata, then inspects and
hands off one selected file at a time. The synchronous sink provides
backpressure without an unbounded channel, and selected records are not retained
by `ScanStreamReport`:

```rust
use weavatrix_scan::{ScanSinkControl, Scanner};

let summary = Scanner::new(".").scan_into(|file: &weavatrix_scan::ScannedFile| {
    println!("{}", file.relative);
    ScanSinkControl::Continue
})?;

assert_eq!(summary.selected, summary.emitted);
# Ok::<(), weavatrix_scan::Error>(())
```

For Search, Clone, and language adapters that need bytes, `visit_content`
connects ignore-aware traversal to bounded parallel content workers. A
worker-local callback receives borrowed chunks from the same read used for
binary detection and optional SHA-256 evidence:

```rust
use weavatrix_scan::{
    ContentDiscoveryMode, ContentFileStatus, ContentValidationPolicy,
    ContentVisitControl, ContentVisitEvent, ScanOptions, Scanner,
};

let options = ScanOptions::default()
    .with_extensions(["rs", "go", "ts", "py"])
    .with_content_discovery(ContentDiscoveryMode::BufferedParallel)
    .with_content_validation(ContentValidationPolicy::Strict);

let summary = Scanner::new(".")
    .options(options)
    .visit_content(|_worker_index| {
        let mut file_bytes = 0_u64;
        move |event| {
            match event {
                ContentVisitEvent::FileStart { .. } => file_bytes = 0,
                ContentVisitEvent::Chunk { bytes, .. } => {
                    file_bytes += u64::try_from(bytes.len()).unwrap();
                }
                ContentVisitEvent::FileEnd {
                    status: ContentFileStatus::Selected,
                    ..
                } => {
                    std::hint::black_box(file_bytes);
                }
                ContentVisitEvent::FileEnd { .. } => {}
            }
            ContentVisitControl::Continue
        }
    })?;

println!("files={}, bytes={}", summary.completed, summary.bytes_read);
# Ok::<(), weavatrix_scan::Error>(())
```

`ContentVisitControl::SkipFile` suppresses remaining chunks for one file while
allowing required hash/binary evidence to finish. `Quit` cancels every worker.
Events include `root_index`, root path, normalized relative path, and a
monotonic sequence. Sort durable results by `(root_index, relative)`.
`ContentValidationPolicy::Strict` verifies native file evidence before and
after the read; `Fast` keeps the safe opened-handle check but omits the
post-read check for latency-sensitive local search. A deterministic total-byte
budget automatically uses the compact two-phase path so budget selection
remains path-order stable.

`ContentDiscoveryMode::Streaming` is the constant-memory default: one serial
producer overlaps discovery with bounded content readers.
`BufferedParallel` uses the parallel ignore-aware walker, retains only compact
candidate evidence, then dispatches the same verified readers. Choose it for
minimum latency on wide or warm repositories; Search and index builders can
sort their durable results after the callback. Both modes use the same
selection, validation, binary, error, and cancellation contracts.

`visit_content_streaming` keeps the same byte, validation, cancellation, hash,
and binary contracts but does not retain compact selected-file evidence or
compute a revision. With `selected_files_only()` it has memory bounded by the
queue, worker state, and one 64 KiB buffer per worker rather than selected-file
count. `ContentVisitReport::mode` makes the empty streaming revision explicit.
A deterministic `max_total_bytes` budget still requires the two-phase
selection path.

`MultiScanner::visit_content` and `visit_content_streaming` use the selected
runtime across all roots; the factory receives `(root_index, worker_index)` and
reports remain in root insertion order. `Scanner::visit_changed_content` and
`visit_changed_content_streaming` accept a safe file-only `WatchPlan`, read
only existing changed paths, return removed paths separately, and yield
`FullRescanRequired` before callbacks for structural or selection-changing
plans.

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
- `cache`: content reads, strict-validation fingerprint reads, and strong hashes
  reused by an incremental scan.

`report.summary()` returns a deterministic, path-free `ScanSummary` for logs,
telemetry, and higher-level tools such as `weavatrix-rust`. It aggregates file
and byte counts, hash/binary work, retained skips by typed kind, warnings,
ignore-source count, completion/termination, portability, and cache work.
Skip totals intentionally describe retained evidence and are zero after
`selected_files_only()`.

Each `ScannedFile` contains an absolute path, slash-normalized repository path,
byte size, optional `sha256:` content hash, whole-content cache fingerprint, and
file-version evidence used to validate persistent cache reuse. The scanner
compares size, timestamps, native file identity where available, and metadata
before/after content reads. Native
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

Consumers reopening an existing snapshot can bind bytes back to the full local
report:

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

### Weavatrix package boundary

Scan owns repository selection, safe bounded content delivery, hashes,
revision, cancellation, and incremental deltas. Search owns literal/regex
matching, line context, encodings, compressed inputs, indexes, and result
formatting. Clone owns token normalization, Moss/winnowing, MinHash/LSH,
Aho-Corasick, and clone grouping. Graph consumes normalized facts and keeps
graph algorithms; content-search and clone algorithms do not belong there.

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

Persistent consumers should store `ScanReport::to_cache()` instead of the full
report and pass it to `Scanner::scan_cached`. `ScanCache` has an explicit format
version and contains only the canonical root plus relative path, size, version,
hash, whole-content fingerprint, and binary-check evidence for reusable files.
`CacheValidationPolicy::Fast` trusts matching file-version evidence.
`CacheValidationPolicy::Strict` additionally reads a compact whole-content
fingerprint before reusing the cached SHA-256, protecting coarse-timestamp and
network filesystems from same-size, same-timestamp changes.

```rust
use weavatrix_scan::{CacheValidationPolicy, ScanOptions, Scanner};

let options = ScanOptions::default()
    .with_cache_validation(CacheValidationPolicy::Strict);
let first = Scanner::new(".").options(options.clone()).scan()?;
let second = Scanner::new(".").options(options).scan_cached(&first.to_cache())?;
assert_eq!(first.revision, second.revision);
# Ok::<(), weavatrix_scan::Error>(())
```

Long-lived file watchers that only need ignore decisions can keep a
`RepositoryMatcher`. Consumers that need the exact Scanner selection contract
can use `SelectionMatcher`; it additionally applies depth, symlink, standard
directory, named type, extension, maximum-size, and filesystem-boundary policy:

```rust
use weavatrix_scan::{ScanOptions, SelectionMatcher};

let options = ScanOptions::default().with_extensions(["rs", "go", "ts", "py"]);
let mut matcher = SelectionMatcher::with_options(".", &options)?;
let decision = matcher.matched("src/lib.rs")?;
assert!(decision.is_selected());
# Ok::<(), weavatrix_scan::Error>(())
```

`SelectionMatcher::matched_entry` reuses metadata already present in a
Weavatrix `WalkEntry`, while `matched` safely classifies an isolated existing
path and its ancestors. Matchers are cloneable for worker-local incremental
queries. Both matcher types expose `refresh()`. Refresh builds a replacement
ignore matcher first, so a failure leaves the existing matcher usable.

`WatcherEventAdapter` converts create/modify/remove/rename notifications from
any watcher library into sorted relative `WatchPlan` invalidations. Events
outside the root are rejected, while directory, ignore-source, and explicit
rescan events request a full scan. `ScanCache::apply_watch_plan` removes only
affected entries or clears the cache when selection may have changed.
`Scanner::scan_watch_plan` goes further: for a safe file-only plan it re-matches
and inspects only changed paths, removes deleted paths from the previous
manifest, keeps unchanged evidence, and recomputes the deterministic revision
without traversing the tree. Structural, unsafe, partial, or selection-changing
plans automatically use a complete scan.
For indexes that consume bytes directly, `visit_changed_content` performs the
same safe path matching and one-pass verified content delivery without walking
unchanged directories. Its revision covers only the changed subset;
`visit_changed_content_streaming` omits that subset manifest and revision.
With the optional `notify` feature, `plan_notify` maps `notify::Event` batches
directly. Access-only events are ignored; imprecise, rescan, and possibly
structural events conservatively request a complete scan.

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
| `file_types` | Empty | Named file-name/repository-relative glob groups |
| `ignore_files` | `.gitignore`, `.ignore`, `.weavatrixignore` | Hierarchical local ignore files |
| `ignore_policy` | Repository-only | Optional parents, `.git/info/exclude`, global Git and explicit files |
| `override_rules` | Empty | Request-level include/exclude globs above ignore sources |
| `ignore_case_insensitive` | `false` | Optional ASCII case-insensitive ignore matching |
| `skip_hidden` | `false` | Skip dot-prefixed and Windows-hidden entries unless included |
| `standard_skips` | Enabled | Skip generated/vendor directories |
| `hash_file_contents` | `true` | Attach per-file hashes and content-sensitive revision |
| `cache_validation` | `Fast` | Trust file-version evidence, or verify a whole-content fingerprint in `Strict` mode |
| `content_validation` | `Strict` | Verify newly opened content before and after reading, or omit the post-read check in `Fast` mode |
| `content_discovery` | `Streaming` | Constant-memory overlapped discovery, or compact `BufferedParallel` discovery for minimum latency |
| `detect_binary_files` | `true` | Reject files containing a NUL byte |
| `evidence` | `Complete` | Keep all typed exclusions, or only selected files |
| `parallelism` | `0` | Traversal/content workers; zero uses bounded available parallelism |
| `traversal_parallelism` | None | Optional traversal-only worker override |
| `content_parallelism` | None | Optional content-inspection worker override |
| `limits.max_entries` | None | Bound examined filesystem entries |
| `limits.max_total_bytes` | None | Deterministically bound selected content bytes |
| `limits.timeout` | None | Stop traversal/content inspection after a duration |
| `cancellation` | None | Cooperative cross-thread cancellation token |
| `walk.max_depth` | None | Limit entry depth; root is zero |
| `walk.min_depth` | `0` | Suppress shallower results while still traversing them |
| `walk.max_open` | `64` | Bound live directory handles/workers |
| `walk.same_file_system` | `false` | Stop at filesystem boundaries when enabled |
| `walk.follow_links` | `false` | Follow only in-root links and detect cycles |
| `walk.root_symlink_policy` | `Follow` | Follow or reject the explicitly supplied root symlink |
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

`NamedFileTypes::defaults()` provides 265 deterministic language, markup, data,
build, configuration, and infrastructure definitions backed by 678 patterns.
That is a strict name-and-pattern superset of the 224 definitions and 594
patterns in `ignore` 0.4.31. `len()` and `names()` expose the catalog without
activating it. Types can be composed, selected, and negated; later matching
selections win:

```rust
use weavatrix_scan::NamedFileTypes;

let types = NamedFileTypes::defaults()
    .with_composed_type("product", ["rust", "go", "typescript", "javascript"])
    .select(["product"])
    .negate(["javascript"]);
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
exact selected path sets against the `ignore` crate for anchored, nested,
negated, wildcard, and character-class fixtures plus 96-seed deterministic
randomized rule sets and direct comparison with `git check-ignore`. A scheduled
workflow also runs 100,000 arbitrary-byte grammar cases and deterministic
directory-read fault injection. Stress cases cover
deep trees, permission errors, raw non-UTF8 ignore rules/names, percent escapes,
and followed symlink loops. The
differential suite and competitor crates are dev-only.

## Enforced modular architecture

Repository traversal is separated from selection policy and manifest
orchestration:

```text
contracts and evidence
    |
selection policy
  ignore hierarchy · globs · typed decisions
    |
bounded traversal
  serial · parallel · streaming · runtime ownership
    |
scan engine
  content delivery · hashes · revision · reports · watch updates
    |
public facade
```

The implementation uses one unambiguous `foo/mod.rs` layout and domain names
such as `discovery`, `inspection`, `scheduler`, and `configuration`; it is not
split into arbitrary numbered chunks.

`.weavatrix/architecture.json` is verified against the crate's own graph.
Release gates require zero runtime cycles, files no larger than 300 physical
lines, functions no larger than 100 physical lines, no exceptions, an empty
baseline, strict Clippy, all platform tests, and benchmark compilation.

## Safety model

- never executes repository code;
- never starts subprocesses or accesses the network;
- canonicalizes and validates the root before traversal;
- does not follow symlink entries by default;
- rejects followed links outside the canonical root and detects cycles;
- follows in-root links in parallel using per-task ancestry, without a serial
  mode switch;
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

Run root-policy, stateful-callback, bounded-pull, and watcher-adapter profiles:

```sh
cargo bench --locked --bench p2_apis
```

Create, verify, and measure an opt-in synthetic scale fixture outside the
repository:

```sh
cargo bench --locked --bench scale_large -- prepare /tmp/weavatrix-scale 1000000
cargo bench --locked --bench scale_large -- verify /tmp/weavatrix-scale
cargo bench --locked --bench scale_large -- parallel-stream /tmp/weavatrix-scale 5
cargo bench --locked --bench scale_large -- scanner-compact /tmp/weavatrix-scale 5
cargo bench --locked --bench scale_large -- scanner /tmp/weavatrix-scale 5
cargo bench --locked --bench scale_large -- content-revision /tmp/weavatrix-scale 5
cargo bench --locked --bench scale_large -- content-stream /tmp/weavatrix-scale 5
```

The command refuses to populate an existing unmarked directory. The fixture
uses 500 empty `.rs` files per directory; one sixth of its directories are
excluded by a root `.ignore`. `verify` asserts the exact sorted path/size
manifest from both full and compact scanners against `ignore`, not only the
selected count. The profile is
intentionally opt-in because creating and removing hundreds of thousands or
millions of filesystem entries is itself expensive.

The synthetic comparison uses 6,000 source files across Rust, Go, and
TypeScript in 80 sibling directories. It runs two warmups and 11 interleaved
measured samples, then reports the median. Raw walkers must produce the same
fully sorted native relative-path set; the ignore-aware comparison additionally
checks the same normalized path-and-size manifest.

### Fixed public repository benchmark

`benches/real_repository.rs` also accepts a fixed checkout through
`WEAVATRIX_BENCH_ROOT`. The primary Windows measurement below used the public
[Rust repository](https://github.com/rust-lang/rust/tree/e19d321c06479c6fd77533582b0d5a86651f1be3)
at commit `e19d321c06479c6fd77533582b0d5a86651f1be3`, which provides an MIT
license option: 61,362 tracked paths, 40,660 raw source paths, and 40,649
paths after repository ignore rules. It is a pinned checkout, not generated
by the benchmark.

```powershell
$env:WEAVATRIX_BENCH_ROOT = "C:\corpora\rust-e19d321c"
$env:WEAVATRIX_BENCH_MODE = "raw"       # or manifest / no-ignore
cargo bench --locked --bench real_repository
```

The defaults are two warmups and 11 interleaved samples. Short exploratory
runs can override them without changing the harness:

```powershell
$env:WEAVATRIX_BENCH_WARMUPS = "1"
$env:WEAVATRIX_BENCH_RUNS = "3"
```

Every published row below is the median of three independent process medians.
Each process uses the defaults and asserts exact sorted output parity before
timing. Manifest rows compare identical `(normalized path, bytes)` values.
Measured 2026-07-28 on Windows 11, Rust 1.97.1, 14 logical processors, and a
warm filesystem cache:

| Contract | Implementation | Files | Median |
| --- | --- | ---: | ---: |
| Raw paths | `dirwalk` 1.1.1 | 40,660 | 123.3 ms |
| Raw paths | `jwalk` 0.8.1 | 40,660 | 161.2 ms |
| Raw paths | Weavatrix collected `ParallelWalker` | 40,660 | 285.9 ms |
| Raw paths | `ignore` 0.4.31, filters off | 40,660 | 498.6 ms |
| Raw paths | Weavatrix `Walker` | 40,660 | 504.8 ms |
| Raw paths | dependency-free `std::fs::read_dir` baseline | 40,660 | 511.6 ms |
| Raw paths | `walkdir` 2.5.0 | 40,660 | 532.8 ms |
| Ignore-aware compact manifest | Weavatrix `scan_compact`, parallel | 40,649 | **208.3 ms** |
| Ignore-aware full manifest | Weavatrix `scan`, parallel | 40,649 | **261.2 ms** |
| Ignore-aware compact manifest | Weavatrix `scan_compact`, serial | 40,649 | 978.2 ms |
| Ignore-aware path/size manifest | `ignore` 0.4.31 | 40,649 | 994.2 ms |
| Ignore-aware full manifest | Weavatrix `scan`, serial | 40,649 | 1,055.2 ms |
| No-ignore path/size manifest | Weavatrix `scan_compact` | 40,660 | **211.5 ms** |
| No-ignore path/size manifest | `walkdir` | 40,660 | 675.1 ms |

The compact and full parallel scanners were respectively 4.77x and 3.81x
faster than the output-equivalent `ignore` manifest. Serial compact remained
1.02x faster, while serial full was 6.1% slower. The no-ignore compact manifest
was 3.19x faster than `walkdir`.

The second fixed corpus is
[Gitea](https://github.com/go-gitea/gitea/tree/0ab3d569b4944d2b4603bb0228d6cfa4ae6ea15e),
an MIT Go/TypeScript/Vue repository pinned at
`0ab3d569b4944d2b4603bb0228d6cfa4ae6ea15e`. It has 6,166 tracked paths and
3,557 selected source paths. The compact/full parallel Weavatrix manifests
measured 37.4/39.2 ms versus 210.8 ms for `ignore`, 5.63x/5.38x advantages with
exact path/size parity.

`dirwalk` is now an explicit raw competitor and is the raw winner on the
primary corpus. Its ignore-aware result is not reported: it differed from the
oracle by ten Rust paths and three Gitea paths, so the harness excluded that
non-equivalent row instead of comparing counts. `scanner-walker` is built on
`ignore`; `parawalk` is a raw-only control; and `wax` is primarily a glob-tree
matcher. These remain useful capability references but do not add another
output-equivalent manifest implementation.

The Windows result is not caused by a private filesystem primitive.
`std::fs::read_dir` already uses
[`FindFirstFileExW(FindExInfoBasic)`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-findfirstfileexw)
and `FindNextFileW`, and its `DirEntry` reuses the returned
[`WIN32_FIND_DATAW`](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-win32_find_dataw)
metadata. The dependency-free standard-library baseline above isolates that
fact. `dirwalk` instead gains from a narrower relative UTF-8-string result,
early filtering, and recursive Rayon scheduling. Weavatrix keeps lossless
native paths, typed local errors, bounded handles, and `unsafe_code = "forbid"`.
Balancing skewed top-level lanes and using eight default traversal workers
reduced the collected Weavatrix row from 421.1 ms to 285.9 ms; closing the
remaining 2.32x raw-only gap would require a separate compact relative-path
contract, not a hidden WinAPI switch.

`dirwalk` is not Windows-only. Its Linux backend reads packed directory entries
with a 32 KiB
[`getdents64`](https://man7.org/linux/man-pages/man2/getdents.2.html) buffer,
then requests size and modification time with
[`statx`](https://man7.org/linux/man-pages/man2/statx.2.html) for every entry,
falling back to `std::fs::read_dir` on backend failure. That per-entry metadata
call may disadvantage a path-only workload, but no native fixed-corpus Linux
ranking is claimed here until it is measured under the same parity harness.

The earlier temporary Go harness established exact raw path parity for
`fastwalk` and `filepath.WalkDir`; `gocodewalker` matched the selected count
but was not a byte-manifest oracle. Go implementations remain competitor
controls rather than another Weavatrix product layer: repository discovery
stays in this crate, parsing in `weavatrix-parse`, and orchestration in
`weavatrix-rust`. Absolute timings still vary with antivirus, cache state,
filesystem, and corpus shape.

Sample result on Windows 11, Rust 1.97.1, warm filesystem cache, measured
2026-07-26 against `ignore` 0.4.31, `walkdir` 2.5.0, and `jwalk` 0.8.1:

| Mode | Library | Files | Median |
| --- | --- | ---: | ---: |
| Raw paths | weavatrix `Walker` | 6,004 | 16.5 ms |
| Raw paths | weavatrix `ParallelWalker` | 6,004 | 10.2 ms |
| Raw paths | ignore | 6,004 | 17.3 ms |
| Raw paths | walkdir | 6,004 | 15.4 ms |
| Raw paths | jwalk | 6,004 | 10.8 ms |
| Ignore-aware manifest | weavatrix `Scanner` serial | 6,001 | 35.2 ms |
| Ignore-aware manifest | weavatrix `Scanner` parallel | 6,001 | 20.7 ms |
| Ignore-aware manifest | ignore | 6,001 | 37.4 ms |
| Rich SHA-256 manifest | weavatrix `Scanner` | 6,000 | 146.2 ms |

On Windows, 0.4.1 reuses the file metadata already collected by the walker
when applying the hidden-attribute policy. This removes a redundant metadata
query per selected entry; hidden, ignore, override, and manifest results are
unchanged.

Each row is the median of five independent process medians. Every process runs
11 interleaved output-equivalent samples after two warmups. On this measurement
`ParallelWalker` was 5.9% faster than `jwalk`; the parallel
selected-manifest `Scanner` was 44.7% faster than `ignore`. The rich row
additionally reads content, detects binaries, computes SHA-256 hashes, captures
snapshot evidence, and records typed exclusions. Absolute timings vary by
filesystem, cache, antivirus, CPU, and operating system; the benchmark workflow
reruns the same checks on Ubuntu, Windows, and macOS.

The separate scale profile was measured on the same Windows host with a warm
filesystem cache. Raw streaming rows are the median of seven independent
process medians with seven measured runs after warmup. Metadata rows use five
process medians with five runs, and the rich SHA-256 row uses three process
medians with three runs. Peak working set is sampled in a fresh process
containing one warmup and one measured run.

| Work | Implementation | Files | Median | Peak |
| --- | --- | ---: | ---: | ---: |
| Raw streaming count | `ParallelWalker::visit` | 300,000 | 90.7 ms | 6.9 MiB |
| Raw streaming count | jwalk | 300,000 | 96.6 ms | 56.5 MiB |
| Raw collected paths | `ParallelWalker::walk` | 300,000 | 208.8 ms | 162.5 MiB |
| Ignore-aware path/size manifest | `Scanner` metadata-only | 250,000 | 346.9 ms | 97.3 MiB |
| Ignore-aware path/size manifest | ignore | 250,000 | 432.4 ms | 20.5 MiB |
| No-ignore metadata manifest | `Scanner` metadata-only | 300,000 | 368.9 ms | 157.5 MiB |
| Ignore-aware content manifest | `Scanner` SHA-256 | 250,000 | 5,693.0 ms | 235.9 MiB |
| Ignore-aware path emission | `rg --files` to null | 250,000 | 1,117.7 ms | 35.3 MiB |

The scanner row is a stronger contract than the comparison manifest: it also
captures native version evidence, hashes ignore inputs, normalizes paths,
sorts deterministically, and computes a revision. The ripgrep row is a
whole-command throughput guardrail, not a library-equivalent benchmark:
ripgrep formats and writes every path, while the library rows count or retain
typed entries in-process. Content hashing is necessarily compared separately
because `rg --files` does not open and hash every selected file. On this sample
the bounded streaming walker stayed below jwalk in both median and peak memory,
and metadata scanning stayed below both the output-equivalent `ignore` manifest
and the ripgrep guardrail.

The same profile was then expanded to 1,000,000 files in 2,000 directories;
833,000 files passed `.ignore`. Raw rows below are the median of five
independent process medians with five measured runs after warmup. The new
compact/full/ignore manifest rows are the median of three independent process
medians, each with one warmup and five measured runs; peak is the median of the
three sampled process peaks. Serial, collected, no-ignore, rich, and ripgrep
rows retain their earlier methodology described in the preceding revision of
this benchmark.

| Work | Implementation | Files | Median | Peak |
| --- | --- | ---: | ---: | ---: |
| Raw serial count | `Walker` | 1,000,000 | 546.3 ms | 5.0 MiB |
| Raw streaming count | `ParallelWalker::visit` | 1,000,000 | 264.7 ms | 7.6 MiB |
| Raw streaming count | jwalk | 1,000,000 | 313.1 ms | 159.7 MiB |
| Raw serial count | walkdir | 1,000,000 | 584.5 ms | 4.6 MiB |
| Raw collected paths | `ParallelWalker::walk` | 1,000,000 | 650.2 ms | 529.0 MiB |
| Ignore-aware compact path/size manifest | `Scanner::scan_compact` metadata-only | 833,000 | **1,019.4 ms** | 63.2 MiB |
| Ignore-aware full path/size manifest | `Scanner::scan` metadata-only | 833,000 | 1,665.9 ms | 309.6 MiB |
| Ignore-aware path/size manifest | ignore | 833,000 | 2,106.7 ms | **56.4 MiB** |
| No-ignore metadata manifest | `Scanner` metadata-only | 1,000,000 | 1,125.2 ms | 369.6 MiB |
| Ignore-aware content manifest | `Scanner` SHA-256 | 833,000 | 24,716.9 ms | 776.2 MiB |
| Ignore-aware path emission | `rg --files` to null | 833,000 | 2,156.3 ms | 42.3 MiB |
| No-ignore path emission | `rg --no-ignore --files` to null | 1,000,000 | 2,535.6 ms | 95.1 MiB |

On this million-file sample, streaming traversal was 15.5% faster than jwalk
and used 95.2% less peak working set. The compact metadata Scanner was 51.6%
faster than the output-equivalent `ignore` manifest while using 6.8 MiB more
peak working set. Compared with the compatibility-oriented full report, it
reduced peak memory by 79.6% and median time by 38.8%. Use the full report when
every entry needs an owned absolute path and file-version evidence; use the
compact report for large retained manifests, streaming traversal for raw
consumers, and rich hashing only when content evidence is required.

The API benchmark uses the same corpus and methodology. Five process medians
measured the ordered bounded DFS iterator at 6.4 ms versus 8.2 ms for `jwalk`.
The new parallel typed stateful batch iterator measured 3.4 ms versus 4.3 ms
for `jwalk::process_read_dir`, 22.2% faster on this sample while preserving the
same batch mutation, child-state propagation, pruning, and ordered output
contract. Output-equivalent streaming over two roots and 12,008 files measured
3.375 ms for Weavatrix versus 7.408 ms for `ignore::build_parallel`, 54.4%
faster on this sample. Each process result is itself the median of 11
interleaved measured runs after two warmups, not a single best run. Watcher
planning and changed-path scan profiles remain in the same reproducible
benchmark.

The one-pass content profile selects and reads the same 6,001 tiny files in
every case. `Fast` and the corresponding `ignore` baseline both validate the
opened handle once; `Strict` and its baseline validate before and after the
read. The raw `ignore` row intentionally omits snapshot validation and is the
lower-contract throughput floor:

| Content pipeline | Retention / validation | Median |
| --- | --- | ---: |
| Weavatrix `visit_content_streaming` | None / opened handle | 62.793 ms |
| Weavatrix `visit_content_streaming` | None / before and after | 73.872 ms |
| Weavatrix `visit_content` | Revision / opened handle | 83.307 ms |
| Weavatrix `visit_content` | Revision / before and after | 78.475 ms |
| `ignore` + `File::read` | None / opened handle | 76.139 ms |
| `ignore` + `File::read` | None / before and after | 80.529 ms |
| `ignore` + `File::read` | None / unchecked | 70.748 ms |

These are five independent process medians from 11 interleaved samples after
two warmups, measured on the Windows host above. Streaming Fast was 11.2%
faster than unchecked `ignore` despite validating the opened handle; Streaming
Strict was 8.3% faster than the equivalent before/after baseline. The
two-root streaming profile processed 12,002 files in 152.013 ms versus
163.588 ms for verified `ignore`, 7.1% faster. A safe 1,024-file changed plan
completed in 47.879 ms versus 109.694 ms for a full 6,000-file scan.

The scale memory check used a temporary 100,000-file fixture with 83,000
selected empty files. A fresh-process sample measured 7.3 MiB peak for
streaming versus 39.4 MiB for revision retention, an 81.5% reduction; three-run
medians were 2,109.956 and 2,182.466 ms respectively. This validates the
scanner-to-consumer handoff, not future literal or regex matching: final
comparison with ripgrep belongs to the Search package and must include matched
output, line handling, and encoding policy.

Source review explains the remaining differences:

- `walkdir` streams unsorted directory entries and bounds open descriptors;
- `jwalk` schedules `read_dir` work through Rayon and restores ordered output;
- `ignore` compiles patterns into `GlobSet` matchers and shares inherited
  matchers;
- Weavatrix `Walker` streams iterative DFS, bounds live handles and buffers the
  oldest remaining frame only when `max_open` is reached; its plain-entry fast
  path and consuming `WalkEntry::into_path` avoid universal-policy checks and
  long-path clones in raw traversal;
- Weavatrix `ParallelWalker` expands a small shallow frontier for narrow roots,
  then uses up to eight default workers without serially over-expanding small
  trees; bounded lanes keep report order independent of worker completion;
- Weavatrix `Scanner` reuses inherited rules, indexes exact literals,
  specializes prefix/suffix globs, prefilters complex patterns, and sorts only
  the final report.

The optional real-repository benchmark never publishes local paths. Fixed
public-corpus rows identify only the pinned upstream checkout and aggregate
timings; other local repositories remain private. Every comparison first
asserts the exact same sorted `(normalized path, bytes)` manifest.

The synthetic stress profile measured a skewed raw tree at 6.7 ms
(`ParallelWalker`), 8.2 ms (`jwalk`), and 14.1 ms (`walkdir`). The expanded
synthetic deep-tree profile contains 60 levels and 7,680 files; five independent
process medians measured 18.9 ms for `Walker` and 20.0 ms for `walkdir`, making
`Walker` 5.6% faster on that sample. An unchanged synthetic 12 MiB SHA-256
manifest fell from 164.4 ms full scan to 1.9 ms with incremental hash reuse.
Treat these as reproducible samples, not universal constants.

## Correctness checks

The test suite covers:

- deterministic results and revisions;
- ignore-rule precedence and nested ignore files;
- repository-only, Git-exclude, parent, explicit and reusable-matcher policies;
- representative and randomized parity with `ignore`;
- raw entry parity with `walkdir` and `jwalk`;
- opt-in exact path/size manifest parity with `ignore` at arbitrary scale,
  including the measured 1,000,000-file fixture;
- iterative deep trees, bounded handles, local error continuation, non-UTF8
  paths, and symlink loops;
- concurrent mutation detection and same-size incremental changes;
- strict cache validation under simulated size/timestamp collisions;
- multi-root walking, named file types, custom native sorting, directory
  filtering, and contents-first ordering;
- single-file roots, full-entry sorting, built-in type composition/negation,
  strict catalog superset parity with `ignore`, redirected-stdout protection,
  parallel raw roots, and `notify` conversion;
- binary, oversized, extension, generated-directory, and symlink policies;
- serial/parallel content-inspection equivalence;
- streaming parallel pruning and cancellation;
- ordered bounded parallel DFS and parallel followed-link cycle handling;
- global, dedicated, and rejecting external runtimes, typed submission
  failures, reentrant callbacks, panic propagation, and pool reuse;
- parallel multi-root callback tagging, subtree pruning, global quit,
  cancellation, and deterministic per-root reports;
- serial/parallel stateful batch order, pruning, entry state, inherited child
  state, and multi-worker execution;
- full/compact exact manifest and revision equivalence;
- changed-path watcher manifests, arbitrary-byte ignore grammar, and injected
  directory-read failures;
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
build graphs. [`weavatrix-graph`](https://github.com/Weavatrix/weavatrix-graph)
owns typed graph primitives. Higher-level Weavatrix crates can compose both
without coupling either library to MCP, a CLI, or language-specific parsers.

## License

MIT © 2026 Sergii Ziborov.

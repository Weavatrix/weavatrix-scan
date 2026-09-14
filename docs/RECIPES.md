# Scan recipes

## Path list without a manifest

`scan_repository_paths` / `Scanner::scan_paths` (Node: `scanPaths` /
`scanPathsSync`) return sorted relative paths only. Use this when the
consumer does not need sizes, hashes, revision, or skip evidence.

```rust
use weavatrix_scan::{ScanOptions, Scanner};

let paths = Scanner::new(".")
    .options(ScanOptions::default().with_extensions(["rs", "ts"]))
    .scan_paths()?;
# Ok::<(), weavatrix_scan::Error>(())
```

```js
const { scanPathsSync } = require('weavatrix-scan')

const paths = scanPathsSync(process.cwd(), { extensions: ['rs', 'ts'] })
```

The selected set matches `scan` / `scanRepository` on the same options,
except size limits are not applied. Cancel and timeout are errors, not an
incomplete report. Join relatives with the host path API only when you need
a native filesystem path.

## Three jobs

1. **One-shot manifest.** `Scanner::scan` or `scan_compact` produces a
   selected file list, hashes, revision, descriptor, and typed skips.
2. **Streaming read for an indexer.** `visit_content` emits borrowed chunks.
   Commit a file only after `FileEnd` with `ContentFileStatus::Selected`.
3. **Correct manifest update.** `WatcherEventAdapter` plus
   `Scanner::scan_watch_plan` or `ScanSession::apply_watch_plan`. Compare
   files, skips, completeness, descriptor, and revision—not only counts.

## Provisional content chunks

```rust
use weavatrix_scan::{ContentFileStatus, ContentVisitControl, ContentVisitEvent, Scanner};

Scanner::new(".").visit_content_streaming(|_| {
    let mut pending = Vec::new();
    move |event| match event {
        ContentVisitEvent::FileStart { .. } => {
            pending.clear();
            ContentVisitControl::Continue
        }
        ContentVisitEvent::Chunk { bytes, .. } => {
            pending.extend_from_slice(bytes);
            ContentVisitControl::Continue
        }
        ContentVisitEvent::FileEnd { status, .. } => {
            if status == ContentFileStatus::Selected {
                // commit pending
            }
            pending.clear();
            ContentVisitControl::Continue
        }
    }
})?;
```

`Changed`, `Binary`, and cancel must not leave a partially committed index
entry. The scanner does not roll back your store.

## Incremental session

```rust
use weavatrix_scan::{ScanOptions, ScanSession, WatchPlan};

let mut session = ScanSession::open(".", ScanOptions::default().with_extensions(["rs"]))?;
session.apply_watch_plan(&WatchPlan {
    changed: vec!["src/lib.rs".into()],
    removed: vec!["src/old.rs".into()],
    full_rescan: false,
    rejected_events: 0,
})?;
```

Lost or overflowed watcher events require `full_rescan: true`. Scan will not
invent the missing changes.

## Composite alternatives

A walker plus a separate ignore crate, a watcher, a hasher, and a
hand-written cache is a different product. Compare that whole pipeline,
including recovery, to Scan—not a raw directory iterator alone.

[Watchman](https://facebook.github.io/watchman/) is the reference for clock
and freshness barriers. [`@parcel/watcher`](https://github.com/parcel-bundler/watcher)
is the reference for a Node subscribe plus historical-change query. Neither
is a Scan-shaped library.

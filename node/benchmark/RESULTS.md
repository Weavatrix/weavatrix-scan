# Node.js and Bun benchmark snapshot

This file is generated. Every number below was produced by the
[weavatrix-benchmarks](https://github.com/Weavatrix/weavatrix-benchmarks)
harness and copied out of its recorded run; none of it is typed by hand.
That repository states the rules every suite obeys, including what each
row had to prove equal before it was allowed to be timed.

**Question.** How fast is an ignore-aware walk for sorted paths, and for paths plus file metadata?

**Competitor.** `fdir`

| Property | Value |
| --- | --- |
| Measured | 2026-09-14 |
| Platform | win32 x64, 10.0.26200 |
| CPU | Intel(R) Core(TM) Ultra 7 255U (14 logical cores) |
| Memory | 47.5 GiB |
| Rounds | 7 measured, after 2 warm-ups, alternating order, median reported |
| Independent runs | 3 per suite, each in a fresh process; the table shows the median and the spread |
| Package | weavatrix-scan 0.5.2 |

## node 24.15.0

Corpus: `[{"files":20000,"bytes":20000}]`

| Contract | Parity | Weavatrix | Competitor | Result |
| --- | --- | ---: | ---: | ---: |
| sorted relative paths | identical path array | 21.447 ms | 28.868 ms | Weavatrix 1.27x faster (1.01x–1.35x) |
| sorted relative paths plus byte sizes | identical {relative, bytes} array | 32.158 ms | 242.987 ms | Weavatrix 7.79x faster (7.18x–7.90x) |

## bun 1.3.14

Corpus: `[{"files":20000,"bytes":20000}]`

| Contract | Parity | Weavatrix | Competitor | Result |
| --- | --- | ---: | ---: | ---: |
| sorted relative paths | identical path array | 14.016 ms | 15.208 ms | Weavatrix 1.07x faster (1.07x–1.09x) |
| sorted relative paths plus byte sizes | identical {relative, bytes} array | 27.700 ms | 282.242 ms | Weavatrix 10.48x faster (9.48x–10.93x) |

## Reading these rows

- **sorted relative paths** — scanPaths returns sorted relative paths from native code without encoding a portable report
- **sorted relative paths plus byte sizes** — the equal consumer-facing contract; fdir needs one statSync per path

## Reproduce

```console
git clone https://github.com/Weavatrix/weavatrix-benchmarks
cd weavatrix-benchmarks && npm ci
node run.mjs --suite=scan
bun run.mjs --suite=scan
node export.mjs
```

CPU, memory bandwidth, filesystem, antivirus, and JavaScript engine
version all move these timings. Treat them as a reproducible snapshot of
the environment above, not as a universal result.

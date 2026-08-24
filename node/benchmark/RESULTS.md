# Node.js and Bun benchmark snapshot

This file is generated. Every number below was produced by the
[weavatrix-benchmarks](https://github.com/Weavatrix/weavatrix-benchmarks)
harness and copied out of its recorded run; none of it is typed by hand.
That repository states the rules every suite obeys, including what each
row had to prove equal before it was allowed to be timed.

**Question.** How fast is an ignore-aware repository walk that also returns file metadata?

**Competitor.** `fdir`

| Property | Value |
| --- | --- |
| Measured | 2026-08-24 |
| Platform | win32 x64, 10.0.26200 |
| CPU | Intel(R) Core(TM) Ultra 7 255U (14 logical cores) |
| Memory | 47.5 GiB |
| Rounds | 7 measured, after 2 warm-ups, alternating order, median reported |
| Independent runs | 3 per suite, each in a fresh process; the table shows the median and the spread |
| Package | weavatrix-scan 0.4.7 |

## node 24.15.0

Corpus: `[{"files":20000,"bytes":20000}]`

| Contract | Parity | Weavatrix | Competitor | Result |
| --- | --- | ---: | ---: | ---: |
| sorted relative paths | identical path array | 47.406 ms | 43.943 ms | competitor 1.06x faster (0.99x–1.20x) |
| sorted relative paths plus byte sizes | identical {relative, bytes} array | 38.645 ms | 3096.983 ms | Weavatrix 80.14x faster (77.50x–82.71x) |

## bun 1.3.14

Corpus: `[{"files":20000,"bytes":20000}]`

| Contract | Parity | Weavatrix | Competitor | Result |
| --- | --- | ---: | ---: | ---: |
| sorted relative paths | identical path array | 42.145 ms | 44.102 ms | competitor 1.06x faster (0.88x–1.08x) |
| sorted relative paths plus byte sizes | identical {relative, bytes} array | 38.427 ms | 3333.106 ms | Weavatrix 86.74x faster (84.03x–92.35x) |

## Reading these rows

- **sorted relative paths** — the narrow-walker row: fdir returns raw paths while Weavatrix still performs its scanner metadata work, so the two land close together and the ordering flips between runs
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

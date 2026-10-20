# perf-data

Written only by `.github/workflows/perf-regression.yml` (the nightly performance check). `perf/history.jsonl` has one JSON line a night:
the date, the two builds compared, the medians of every measure, what flagged and the open `perf-regression` issue. Fields are added, never
renamed. The format is in SPEC_NIGHTLY_PERF_REGRESSION section 4.9; the nightly GitHub email reads the last line.

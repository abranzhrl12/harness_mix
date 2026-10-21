# Sync pipeline probe

Isolated protocol probe for moving extraction before SQLite write transactions.
It uses Python's standard `sqlite3` against temporary databases configured as
WAL + `synchronous=FULL`; it does not import or modify the application or its
real CodeGraph index. The extractor is a deterministic CPU workload standing
in for parsing, so timings compare lock placement only and are not parser or
end-to-end project benchmarks.

Run from the repository root with `python experiments/sync-pipeline-probe/probe.py`.
The scenarios cover source/config revalidation, cancellation and SQL-error
rollback, extraction errors before transaction entry, bounded queue pressure,
and a small lock-duration comparison between extraction inside and outside
`BEGIN IMMEDIATE`.

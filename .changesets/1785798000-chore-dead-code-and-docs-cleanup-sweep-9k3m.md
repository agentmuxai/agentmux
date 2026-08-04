---
type: patch
---

chore: remove dead code (Rust + frontend) and archive/dedupe stale docs

Multi-agent verified sweep (cargo check + knip, each candidate independently
grep-checked against known false-positive patterns before removal):

- Rust: 20 confirmed-dead symbols removed across 16 files (unused functions,
  fields, enum variants, one dead trait method with its 4 impl overrides).
  One item (`Command::Unregister`) deliberately left in place — removing it
  would delete tested regression-guard behavior for a real historical race
  condition (reagent #2275), not just dead code.
- Frontend: 29 whole files deleted (zero importers anywhere), ~300 unused
  exports/types removed or made module-private across ~120 otherwise-live
  files, 9 unused npm dependencies removed from package.json.
- Docs: 29 stale/superseded docs moved into their existing `archive/`
  convention, 1 exact duplicate deleted, the stray `docs/retros/` directory
  merged into the canonical `docs/retro/`. 937 of 967 total docs kept as-is
  (legitimate historical record).

Full findings report: docs/reports/REPORT_DEAD_CODE_AND_DOCS_SWEEP_2026_08_03.md

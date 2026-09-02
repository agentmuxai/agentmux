# docs/

Project documentation organized by type.

| Path | Purpose |
|------|---------|
| [`linux.md`](linux.md) | Linux operator guide — AppImage structure, display server, logging, diagnostics, known limitations |
| `analysis/` | Technical analysis, audits, benchmarks, root cause investigations |
| `api/` | User-facing API guides (start with `api/getting-started.md`) |
| `architecture/` | Architecture decision records and subsystem design docs |
| `archive/` | Retired handoff/report docs, kept for historical reference |
| `brand-icons/` | Logo/icon assets |
| `cef-build/` | Guides for building the patched `libcef.so` from source |
| `cef-patches/` | Patches applied to the vendored CEF source |
| `handoff/`, `sessions/` | Point-in-time session handoff notes |
| `incident/`, `recovery/` | Incident write-ups and recovery runbooks |
| `investigations/` | Active bug investigations with reproduction steps |
| `plans/` | Standalone implementation plans |
| `providers/` | Provider (Claude/Codex/etc.) integration notes |
| `reports/` | Session reports, handoff notes, bug fix summaries |
| `research/` | Research into technologies, approaches, and design options |
| `retro/` | Post-incident retrospectives |
| `specs/` | Specs, design explorations, and implementation plans, from draft through implemented |
| `status/` | Point-in-time subsystem status snapshots |

Spec `**Status:**` lines use a closed vocabulary (`draft | proposed | active | implemented | living | historical | superseded`) — the rule, and the reader guardrail that goes with it, live in [`docs/specs/README.md`](docs/specs/README.md#status-field).

All specs live under `docs/specs/`. The top-level `specs/` tree was merged into it
(2026-09-01) — a doc's lifecycle is its `**Status:**` line, not the directory it sits in,
and the two had come to contradict each other. See [`docs/specs/README.md`](specs/README.md)
for why. There is no `docs-internal/` directory in this repo.

**Note (2026-08-03):** this directory list and the specs-location claim above were themselves
found stale during a docs-lifecycle audit — see
[`docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`](docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md)
for the fuller audit and a plan to stop this file (and others like it) silently drifting out of
date again. If you're reading this after that plan's Phase 1-3 shipped, some of the manual
bookkeeping described here should have been superseded by an auto-generated index — check before
trusting this table blindly.

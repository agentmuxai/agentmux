# Report: large migrations and initiatives — what is done, what is pending

**Status:** proposed
**Date:** 2026-09-06
**Author:** Korp
**Baseline:** `main` @ `fe87e1732` (post-v0.55.37)
**Ask (repo owner):** *"we were doing a lot of migrations and initiatives, but some never completed. can you take a look through all the docs and code and ID what is done and what is pending as far as our large migrations."*

---

## 0. The short version

Fourteen large initiatives were identified. **Five are genuinely complete**, **six are partially
done and stalled**, and **three were designed and never started.**

The headline is not the count — it is *which* ones stalled. The two biggest migrations in the
repo's history (Tauri→CEF and React→SolidJS) both **ran to completion and are done**, quietly,
without either being tracked to closure in a doc. Meanwhile the initiatives that stalled are
almost all **hardening and consolidation** work — the unglamorous second half of a migration,
where the new thing works but the old thing was never removed.

Three findings worth acting on independently of any single initiative:

| # | Finding | Evidence |
|---|---|---|
| 1 | **`agent-view.tsx` more than doubled (1,282 → 2,730 lines) while the two items assigned to fix it stayed open for 3 months** | A6/A9 in issue #1549, open since 2026-06-18; `wc -l` today |
| 2 | **`CLAUDE.md`'s jekt section states `transcript_request` "does not exist anywhere in agentmux-srv today" — it shipped 2026-08-22 in PR #2764** | `agentmux-srv/src/server/reactive.rs:540-606`, `agentmux-common/src/transcript_request.rs` |
| 3 | **Migration failure is still silently non-fatal at startup**, exactly as `SPEC_MIGRATION_SYSTEM_HARDENING` says — every migration runs behind a `warn!` | `agentmux-srv/src/bootstrap.rs:565-569` |

---

## 1. Method, and how much to trust this

- **Doc sweep:** all 857 specs under `docs/specs/` (+61 archived), 70 reports, 137 analyses,
  15 status docs, 5 plans. `**Status:**` lines extracted in one pass and tallied.
- **Status lines are not trusted on their own.** `docs/specs/README.md` says so itself
  ("statuses rot... spot-verify against current code"), and the numbers back it: of 754 specs
  with a Status line, only 544 use the closed enum — 210 use non-canonical words and 150 have
  no Status line at all. Every claim in §2–§4 below is marked **[verified]** (checked against
  code at this baseline) or **[doc-claim]** (taken from a spec, not independently confirmed).
- **Code checks:** grep/`wc` against the live tree for the specific artifact each initiative
  was supposed to create or remove.
- **Issue checks:** open tracking issues via `gh`.

**What this report does not do:** it does not re-litigate whether each initiative was a good
idea, and it does not estimate effort. It answers "is it finished, and if not, what is left."

---

## 2. Complete — verified

### 2.1 Tauri → CEF  ✅ **[verified]**

The entire desktop shell moved off Tauri/WebView2 onto embedded CEF. No Tauri dependency
remains in any `Cargo.toml` or `package.json`. The 29 files matching "tauri" are all
**comments and historical references** (e.g. `agentmux-cef/src/commands/backend.rs:5`
"Ported from src-tauri/..."; `ipc.rs:36` "maps to Tauri command names"), not code.

**Residue, cosmetic only:** IPC command names are still described as "Tauri command names" in
`agentmux-cef/src/ipc.rs`. Harmless; rename if it ever confuses someone.

### 2.2 React → SolidJS  ✅ **[verified]** — and nothing says so

390 frontend files import `solid-js`. **Zero** import `react`.
`docs/reports/solidjs-migration-analysis.md` (2026-03-11) opens with "Current stack: React 19 +
Jotai + Vite + Tauri" — three of those four are now false. The analysis doc was the *proposal*;
no doc anywhere records that the migration happened, let alone finished.

**Residue:** `react: 18.3.1` is still a declared `devDependency` in `package.json:57` with no
importer. Worth removing — it is a real install cost and a misleading signal to any new reader
(and to the analysis doc's own future readers).

### 2.3 Reducer / saga architecture (Phases E–H)  ✅ **[doc-claim, partially verified]**

The launcher/host/srv reducer stack and saga coordinator shipped across phases E, E4, E4b, F,
and H. Saga durability was later deliberately **collapsed** rather than completed:
`SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md` ("Implemented") deletes the durable log,
recovery walker, retention vacuum and `--diag sagas` offline reader from
`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`, keeping only the live coordinator semantics.
That is a completed decision, not an abandoned phase.

**Caveat:** `MASTER_REDUCER_STACK_STATUS_2026-05-05.md` claims authority over this subsystem's
status and carries its own staleness note — it has not been updated since 2026-05-29, and a
later independent pass (`REPORT_REDUCER_STACK_AUDIT_2026_07_26.md`) covers the same ground
without updating it. Two "current status" docs disagree about which is current. See §5.2.

### 2.4 Architecture refactor A1–A15 — 12 of 15  ✅ **[verified]**

Issue #1549. Verified by locating the artifacts each item was supposed to produce:

- **A1** — `rpc_types.rs` (2,427 lines) → `agentmux-srv/src/backend/rpc_types/` (directory);
  `rpc-api.ts` (1,568 lines) → `frontend/app/store/rpc-api/` (directory).
- **A4** — `service.rs` (2,892 lines, one 2,272-line `match`) → `server/service/` split into
  `client.rs`, `object.rs`, `credential.rs`, `introspect.rs`, `reducer_helpers.rs`, and more.
- **A5** — `blockcontroller/shell.rs` (2,358 lines) → `shellexec.rs`, `shellintegration.rs`,
  `shell_node.rs`, `server/shell_handlers.rs`.
- **A8** — `websocket.rs` 2,371 → 1,837 lines, split by command family.
- A2, A3, A7, A11, A12, A13, A14, A15 all closed with cited PRs.

Three remain open — see §3.1.

### 2.5 muxspect cross-tier conversation visibility (Phases A, B, C)  ✅ **[verified]**

Phase A, B, and C all shipped (B/C in PR #2764, 2026-08-22). `transcript_request` parsing,
per-agent `conversation_visibility` policy, trust-grant storage, and server-side `ESCALATE`
resolution are live:

- `agentmux-common/src/transcript_request.rs` — the parser
- `agentmux-srv/src/server/reactive.rs:562` — `resolve_transcript_request_tier_fields()`
- `agentmux-srv/src/backend/storage/conversation_trust_grants.rs` — the allowlist store

Both phase specs say "Implemented (scoped)" with deferred items listed in their own §2/§3.
**This is the initiative `CLAUDE.md` still describes as unbuilt** — see §5.1.

---

## 3. Partially complete — the actual backlog

These are the ones the ask was about. Each has a working first phase and a stalled remainder.

### 3.1 Architecture refactor A1–A15 — the 3 that stalled  🟡 **[verified]**

Open since 2026-06-18 (issue #1549 last updated the same day — ~12 weeks untouched):

| Item | What's left | Verified state today |
|---|---|---|
| **A6** | Collapse agent-pane's 4 parallel state systems | All four still exist: `agent-document-store.ts`, `agent-pane-layout-store.ts`, `agent-pane-model.ts`, `agent-pane-state-store.ts` |
| **A9** | De-dup the agent-pane "is busy?" selector (4×); route raw dispatches via `paneModel` | Blocked at filing on PR #1573; that PR has long since resolved |
| **A10** | Consolidate data-dir resolution onto `DataPaths` | Marked "safe to skip or tackle incrementally" by the board itself |

**This is the item to act on.** The audit named `agent-view.tsx` at **1,282 lines** as the
A6/A9 hotspot. It is **2,730 lines today** — it has more than doubled while the fix sat open.
A6 is the only one of the three the board rates "high effort," and it gets more expensive every
month. A10 is explicitly optional; A9 is small and no longer blocked.

### 3.2 Migration system hardening  🟡 **[verified — the gap is real]**

`SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`: "Phase 0 shipped in PR #2394; Phases 1-6 not
started (migration failure is still non-fatal at bootstrap.rs)."

**Confirmed true at this baseline.** `agentmux-srv/src/bootstrap.rs:565-569`:

```rust
match migrations::run_pending_migrations(&wave_data_dir) {
    Ok(0) => {}
    Ok(n) => tracing::info!(applied = n, "startup: applied pending migrations"),
    Err(e) => tracing::warn!("startup: migration error (continuing): {}", e),
}
```

A failed schema migration logs a warning and the server starts anyway, against a database in an
unknown state. Note the asymmetry a few lines below: failing to *open* the store calls
`process::exit(1)`. Failing to *migrate* it does not.

The underlying framework (`SPEC_MIGRATION_FRAMEWORK_2026_06_24.md`) is fully built and in daily
use — `agentmux-srv/src/migrations/` with `m0000_bootstrap` through `m0006_definitions_global`
and beyond — so this is hardening a working system, not finishing a broken one. Phases 1–6 remain.

### 3.3 Docs lifecycle hardening  🟡 **[doc-claim, corroborated]**

`SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`: Phase 0 (PR #2394) and Phase 1 (closed Status
vocabulary, 2026-08-10) shipped; **Phases 2, 3, 5 not started**; Phase 4 partially covered by
the reader guardrail in `docs/specs/README.md`.

Phase 3's generated `INDEX.md` and `scripts/gen-docs-index.sh` **do** exist and run, which
suggests Phase 3 landed after that Status line was last edited. The enforcement gate
(`scripts/check-doc-status.sh`) exists and is deliberately scoped to changed files only.

**The measured backlog this leaves:** of 857 specs, **150 have no Status line** and **210 use a
non-canonical status word** (`ready` ×38, `spec` ×34, `design` ×13, `shipped` ×12, `approved`
×11, …). The gate stops the backlog growing but by design never reduces it, and nothing
schedules the cleanup of those ~360 files.

### 3.4 Jekt cross-channel trust  🟡 **[doc-claim]**

`SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md`: "Phase A (D1 key publication + D5 signing
primitives) shipped in #2959. Phases B (verification), C (enforcement) and D (escalation
chaining) remain."

Phase A is explicitly additive — it publishes keys, but nothing verifies against them yet, so
the security benefit arrives in Phase B/C. This is the newest stalled item (4 days old) and is
plausibly just in-flight rather than abandoned, but it is incomplete at this baseline.

**Update 2026-09-07:** Phase B shipped (D2 verification, `DELIVERY=channel`, `TRUST=channel-verified`
in the `ESCALATE=none` list) — spec §10.2. Phase C (forcing `sensitive` on a failed cross-channel
signature) is a deliberate one-line follow-up once published keys have propagated; Phase D is its
own spec.

### 3.5 Container / sandbox agents  🟡 **[doc-claim]**

Umbrella issue #2939 (open, "multi-generation"), with Phase 3 integration gaps in issue #1400
(open since 2026-06). The umbrella's own framing: *"The lifecycle and turn execution have
worked for months; everything around them did not, and the gaps were found one at a time
because nothing tracked them together."*

Known open gaps from #1400: the sidecar is not reachable from inside the container
(`AGENTMUX_LOCAL_URL` is never added to the per-turn `env_vars` map, and `localhost` would
resolve to the container anyway); MCP servers and PreToolUse/bashwrap hooks are written to the
agent's *host* working directory, which the container never mounts.

### 3.6 Armory / Stash foundation consolidation  🟡 **[doc-claim]**

`ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` is explicitly a **north-star
document**: "proposal — vision/north-star... Not implemented, not meant to land as one PR.
Intended to be worked incrementally via follow-up `SPEC_` docs, sequenced per §4."

Earlier Armory phases (4: storage rename; 5: consolidation + skill seeding) did ship. The
consolidation *vision* layered on top of them has not started. This one is correctly labelled
and is pending-by-design rather than stalled — but it is pending.

---

## 4. Designed, never started

### 4.1 Mandatory ABF rethink  🔴 **[doc-claim]**

`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md`: **"DECISIONS RESOLVED, NOT YET
IMPLEMENTED."** All open questions closed; extended the same day with the portability idea
(harness + model as read-only ABF fields, making an ABF the portable unit rather than "the
agent"). Nothing built. This is the largest fully-designed-but-unbuilt item found.

### 4.2 DRY / modularity slimming plan  🔴 **[doc-claim, 1 of 5 started]**

`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` (**written today**, Status:
proposed). Measured duplication is low (2.8% frontend / 3.2% Rust) but it found **334 "keep in
sync" comments across 210 source files** — one file in six. Five root causes, in its own payoff
order:

1. **No Rust↔TypeScript codegen** — 322 hand-maintained RPC stubs + a 2,819-line
   hand-maintained `gotypes.d.ts`. **[verified]** `frontend/types/gotypes.d.ts` is 2,819 lines
   and no codegen script exists in `scripts/`. Note this is the *unfinished half of A1*: A1
   split the god-files and added a contract-enforcement test, but never removed the
   hand-maintenance the test was guarding.
2. **`agentmux-common` isn't used as a common** — 8K shared lines vs 300K Rust.
3. **Whole-file platform forks** — `zoom.{win32,linux,darwin}.ts` differ by 9 comment lines of 186.
4. **Twin primitives by copy-rename** — `mcp` ↔ `skill` are 58–71% structurally identical.
5. **Parallel saga+reducer frameworks** in launcher and srv sharing only vocabulary.

**Root cause 2 is already in progress** — PR #3033 (2026-09-06) lifted `CREATE_NO_WINDOW`,
`now_ms`, `event_log`, process-kill and `WindowKind` into `agentmux-common`, and PR #3036
consolidated the ten `backend_*` helpers onto one raw-TCP transport. The other four are untouched.

### 4.3 Wave → Mux rename  🔴 **[verified]** — issue #851, open since 2026-05-14

The single largest unfinished cleanup by raw count. Still in the tree:

| Symbol | Occurrences |
|---|---|
| `WaveEvent` | 223 |
| `WaveObjUpdate` | 93 |
| `WaveObj` | 42 |
| `WaveKeyboardEvent` | 23 |
| `WaveFile` | 19 |
| `WaveWindow` | 16 |
| `WaveObject` | 11 |
| others (`WaveInfoData`, `WaveLock`, `WaveBlock`, `WaveNotificationOptions`, …) | ~30 |

**285 source files** contain a `wave` reference of some kind. It also reaches into runtime
identifiers, not just type names: `bootstrap.rs:427` calls `base::get_wave_data_dir()`, and the
variable is `wave_data_dir` throughout the migration code quoted in §3.2.

This is pure tech debt with no user-visible payoff, which is presumably why it has sat for ~4
months. Worth an explicit decision: schedule it, or close #851 and accept `Wave*` as permanent
internal vocabulary. Leaving it open indefinitely is the worst of the three options — it keeps
signalling "we are mid-rename" to every new reader.

---

## 5. Stale claims found while verifying

Reported separately because each will actively mislead the next reader, and because the repo's
own guardrail (`docs/specs/README.md`) asks for exactly this kind of spot-check.

### 5.1 `CLAUDE.md` says `transcript_request` doesn't exist. It does. **[verified]**

The jekt security section states, twice and emphatically:

> "**`transcript_request` does not exist anywhere in `agentmux-srv` today (Phase B/C is
> designed, not built)** — this rule is a repo-owner-confirmed policy commitment for that
> future code, not a description of anything srv currently enforces."
>
> "Whoever implements Phase B/C must wire this rule in as part of that work, not assume it's
> already there."

It shipped on 2026-08-22 in PR #2764 — the same day that spec was written.
`reactive.rs:562`'s `resolve_transcript_request_tier_fields()` computes
`transcript_request_escalate_forced` from the responding agent's own `conversation_visibility`,
which is precisely the rule CLAUDE.md describes as unbuilt.

**Why this one matters more than the others:** CLAUDE.md instructs agents to treat that section
as authoritative and to distrust inline corrections. An agent following it literally would
either re-implement a live feature or wrongly conclude the escalation rule isn't enforced.
Correct it in CLAUDE.md directly, citing the PR.

### 5.2 Two competing "current status" docs for the reducer stack **[verified]**

`MASTER_REDUCER_STACK_STATUS_2026-05-05.md` claims "**Authority.** When this file disagrees
with another spec... this file wins for *current status*", then carries a note admitting it is
~9 weeks stale and that `REPORT_REDUCER_STACK_AUDIT_2026_07_26.md` covers the same subsystem
without updating it. Withdraw the authority claim or restamp the file `historical`.

### 5.3 `SPEC_MIGRATION_FRAMEWORK_2026_06_24.md` is marked "Proposed" but shipped **[verified]**

Already caught by the 2026-08-07 audit and annotated inline — but the Status line still reads
`Proposed`, so the generated `INDEX.md` files a foundational, in-daily-use framework under
proposals. A one-word fix.

### 5.4 The SolidJS analysis doc describes a stack that no longer exists **[verified]**

`docs/reports/solidjs-migration-analysis.md` still opens "Current stack: React 19 + Jotai +
Vite + Tauri (WebView2/Chromium)". Every element except Vite is wrong, and the migration it
proposes is done. Restamp `historical`.

---

## 6. Scoreboard

| # | Initiative | State | What remains |
|---|---|---|---|
| 1 | Tauri → CEF | ✅ done | comment-only residue |
| 2 | React → SolidJS | ✅ done | drop the unused `react` dep |
| 3 | Reducer/saga (E–H) | ✅ done | reconcile two status docs (§5.2) |
| 4 | Arch refactor A1–A15 | 🟡 12/15 | **A6, A9**, A10 (optional) |
| 5 | muxspect A/B/C | ✅ done | fix CLAUDE.md (§5.1) |
| 6 | Migration system hardening | 🟡 Phases 0, 1, 3, 6 of 6 (1a/1b/3/6 shipped 2026-09-07, 1c 2026-09-07) | Phases 2, 4, rest of 5 |
| 7 | Docs lifecycle hardening | 🟡 ~3 of 6 | Phases 2, 5; ~360-file status backlog |
| 8 | Jekt cross-channel trust | 🟡 Phases A–B of D (B shipped 2026-09-07) | Phases C, D |
| 9 | Container agents | 🟡 multi-generation | #2939 workstreams; #1400 Phase 3 gaps |
| 10 | Armory foundation consolidation | 🟡 by design | follow-up SPECs per its §4 |
| 11 | Mandatory ABF rethink | 🔴 not started | everything |
| 12 | DRY / modularity slimming | 🔴 1 of 5 | causes 1, 3, 4, 5 |
| 13 | Wave → Mux rename (#851) | 🔴 not started | ~460 symbol uses across 285 files |
| 14 | Agent working-state unification | 🟡 Phase 1 | `SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04` — overlaps A6 |

---

## 7. Recommendation

If only three things get picked up:

1. **A6/A9** (§3.1) — the only stalled item that is measurably getting worse (`agent-view.tsx`
   1,282 → 2,730 lines in 12 weeks). A9 is small and no longer blocked; do it first to derisk A6.
2. **Migration hardening Phase 1** (§3.2) — a small change to make migration failure fatal,
   guarding a database that seven-plus migrations write to. Highest safety-per-line here.
3. **Correct CLAUDE.md's jekt section** (§5.1) — costs minutes, and it is the one stale claim
   that actively instructs every agent in the fleet to believe something false.

Then make a **decision** on #851 rather than leaving it open: schedule it or close it.

One structural note. Items 1, 6, 7, 8 and 12 share a shape — a Phase 0 or Phase A lands, is
genuinely useful, and the remaining phases never start. Phase 0 is usually the instrumentation
or the additive half; the phase that *removes the old path* or *enforces the new one* is the
one that gets skipped. Worth considering whether "Phase 0 shipped" should be allowed to close a
tracking issue at all, or whether these should stay open against the enforcing phase.

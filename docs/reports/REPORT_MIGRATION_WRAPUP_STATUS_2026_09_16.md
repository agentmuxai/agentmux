# Report: migrations wrap-up — re-verification of the 09-06 audit, ten days on

**Status:** implemented — every doc correction this report recommends for itself
was made in the same PR (§4). The items in §5 are handed off, not done.
**Date:** 2026-09-16
**Author:** Manoz (manoz-0803a)
**Baseline:** `main` @ `65958f84e` (post-v0.56.2)
**Supersedes for status purposes:** `REPORT_LARGE_MIGRATIONS_COMPLETION_AUDIT_2026_09_06.md`
(that report's method and framing stand; six of its findings have since gone stale — §2)
**Ask (repo owner):** *"Codex analyzed the agentmux codebase and said there were many
in-progress migrations, we want to wrap them up. it may be stale docs too, see
issues/discussions. so we want to clean it all up — finish up the migrations, or update
the docs if they are done."*

---

## 0. The short version

The 09-06 audit was right that this repo's problem is **unfinished second halves**. Ten days
later the more urgent problem is the opposite one: **the repo is further along than its own
docs say.** Six of that audit's fourteen scores are now wrong in the "actually done"
direction, including its single largest red item and its own headline safety finding.

| | 09-06 audit said | True on 2026-09-16 |
|---|---|---|
| Migration failure at startup | silently non-fatal — headline finding #3 | **fatal** (`bootstrap.rs` → `process::exit(1)`) |
| Mandatory ABF rethink | "largest fully-designed-but-unbuilt item" | **shipped 2026-08-15, PR #2587** — the day after it was written |
| Arch refactor A9 | stalled | **done**, PR #3044 |
| Arch refactor A6 | stalled | **half done**, mirror killed in PR #3054 |
| Docs lifecycle Phase 5 | not started | **shipped**, PR #3068 |
| Platform-forked `zoom.*.ts` | 3 near-identical files | **collapsed**, PR #3034 |

That is not a documentation nitpick. An audit is the artifact people plan from, and this one
was already steering work: it named ABF-mandatory as the biggest unbuilt thing in the repo
while `m0021`, `agent_def_provision_and_bind_bundle` and the readonly-once-set guard had been
live for three weeks. **The dominant failure mode here is not abandoned work — it is finished
work that never got restamped, and then gets re-planned by the next reader.**

The genuine backlog is smaller than fourteen items, and is §5.

---

## 1. Method

Four parallel verification passes against this baseline, each re-deriving the audit's claims
from code rather than from its prose: initiatives 1–5, 6–10, 11–14 plus the runtime schema
migrations, and a GitHub issues/discussions sweep. Claims below are marked **[verified]**
(checked against code or CLI output at this baseline) or **[doc-claim]**.

Two findings that came back from those passes were **rejected on inspection** and are recorded
in §6, because both would have caused damage if actioned.

---

## 2. Corrections to the 09-06 audit — work that is done

### 2.1 Migration system hardening — Phase 4 is all that's left ✅ **[verified]**

The audit's headline finding #3 ("migration failure is still silently non-fatal at startup")
is **fixed**. `agentmux-srv/src/bootstrap.rs:603-628`:

```rust
Err(e) => {
    tracing::error!("startup: migration failed — refusing to start: {}", e);
    eprintln!("{}", agentmux_common::srv_stderr::migration_failed_line(&e));
    std::process::exit(1);
}
```

Phases 0, 1a (#3043, #3047), 1b, 1c (#3065), 2 (#3070), 3 (#3062), 5 (#3066) and 6 all shipped
2026-09-07. Supporting artifacts verified present: `migrations/phase5_tests.rs`,
`docs/recovery/RUNBOOK_MIGRATION_RECOVERY_2026_09_07.md`, `srv_stderr::migration_failed_line`,
`runner::doctor_report_for_instance`. **Only Phase 4 remains, and the spec itself calls it a
product decision, not engineering.**

### 2.2 Mandatory ABF rethink — shipped the day after it was designed ✅ **[verified]**

Scored 🔴 "not started — everything remains" by the audit. In fact §7.5's build-order steps 1–3
all carry inline `**DONE (2026-08-15).**` markers and landed in PR #2587. Verified live:
`m0021_backfill_agent_bundles.rs`, `Store::agent_def_provision_and_bind_bundle`,
`Store::bundle_provision_for_new_agent`, `Store::resolve_effective_provider_id`, and the
readonly-once-set enforcement at `app_api/bundle.rs:125-147`. Step 4 (delete-cascade / orphan
handling) is open and was written as "(if wanted)".

The header read `DECISIONS RESOLVED, NOT YET IMPLEMENTED` for a month while the body said DONE
three times. **Restamped in this PR.** The genuinely open part is §7 (harness + model as
readonly, portable ABF fields), and §7.0 already flags that it reverses a prior deliberate
architectural decision — that needs a repo-owner call before anyone builds it.

### 2.3 Architecture refactor A1–A15 (#1549) — 13.5 of 15 ✅ **[verified]**

- **A9 — done.** PR #3044 routed the 11 raw dispatches and added a guard test.
- **A6 — half done.** PR #3054 killed the `AgentAtoms` mirror; `model.state`/`model.document`
  are now the single reactive source, pinned by `agent-pane-view.test.ts`. The remaining half
  (scroll/expansion unification) moved to `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md`.
- **A10** — unchanged, still self-annotated "safe to skip".

All four agent-pane state files still exist, and **`agent-view.tsx` is now 3,017 lines** (1,282
when A6 was filed, 2,730 at the 09-06 audit, 13 commits in the last 10 days). The file is still
growing faster than the refactor meant to shrink it — §5.8.

### 2.4 Docs lifecycle hardening — Phase 2 is all that's left ✅ **[verified]**

Phase 5 shipped 2026-09-07 (#3068). Remaining: Phase 2 (directory consolidation); Phase 4
partially covered by the reader guardrail in `docs/specs/README.md`.

### 2.5 Already fixed by the audit's own follow-up PR ✅ **[verified]**

`react` is gone from `package.json` (411 files import `solid-js`, zero import react);
`solidjs-migration-analysis.md` is restamped `historical`; `MASTER_REDUCER_STACK_STATUS`'s
authority claim is explicitly withdrawn. Nothing remains on any of these.

### 2.6 DRY — one of five causes closed ✅ **[verified]**

`zoom.{win32,linux,darwin}.ts` collapsed into one `zoom.ts` (PR #3034). `agentmux-common` is now
17 files / 9,858 lines with `time.rs`, `win32.rs`, `process.rs` and `event_log.rs`, and the
srv/launcher copies are ~29-line re-export shims.

**Correction (2026-09-16, later the same day).** This section originally said "the call-site
sweep did not follow the lift: 29 files still name `CREATE_NO_WINDOW` locally and 23 still
define their own `fn now_ms`", and called it the same "additive half landed, removing half
skipped" shape. **That was wrong, and it was my own error repeating the DRY audits metric
without checking what it counted.** Verified directly: exactly ONE `const CREATE_NO_WINDOW`
exists in the tree (`agentmux-common/src/win32.rs`), zero files re-declare it, and 24 import
it correctly. The 23 `fn now_ms` "duplicates" are three-line delegators —
`fn now_ms() -> i64 { agentmux_common::time::now_ms() }` — so the logic exists once and the
short local call site is deliberate. Counting usages and delegators as duplication overstates
the problem; this lift is essentially complete.

The one real residue was different and smaller: **~20 call sites across 4 crates passed the
raw magic number `0x08000000`** instead of the shared constant — worse than a duplicate
`const`, because a magic number is unsearchable. Fixed in the PR that added this correction.

---

## 3. Stale docs — the reverse error, found systematically

The audit checked specs claiming *done* against reality. The larger population is the other
direction: **specs claiming they are unbuilt while their artifact is cited in source.** A cheap,
high-signal query (spec filename referenced from `.rs`/`.ts`/`.tsx` while its Status says not
implemented) returned **32 candidates**. Four were verified individually; three were
definitively wrong and are corrected here (§4).

This should be institutionalised rather than repeated by hand — §5.6.

---

## 4. What this PR changed

Every item was verified against a named code artifact first. **No bulk restamping** —
`scripts/check-doc-status.sh`'s own comments name that as the failure to avoid ("replaces
unknown status with confidently wrong status"), which is why the 357-file backlog in §5.5 is
deliberately left alone.

| File | Was | Now | Evidence |
|---|---|---|---|
| `CLAUDE.md` | `transcript_request` is "a pre-committed POLICY for a jekt type that **does not exist in shipped code yet**" | live since PR #2764 | `transcript_request.rs`; `resolve_transcript_request_tier_fields()` at `reactive.rs:716`, called at 10+ sites |
| `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` | `Proposed — not yet implemented` | `active` — host tier shipped, WAN half gated | `jekt_sign::{sign_jekt,verify_jekt}`; `inject_jekt_signing_keys_into_mcp_json` (`agent_config.rs:1312`). First restamped `implemented`; corrected after review — see §5.9 |
| `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` | `DECISIONS RESOLVED, NOT YET IMPLEMENTED` | `active` — steps 1–3 shipped | PR #2587 artifacts, §2.2 |
| `SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md` | `Draft — spec only, no code written yet` | `implemented` | `frontend/app/element/primitive-list-detail.tsx` cites it by name |
| `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md` | `Proposed (…not implemented) (implemented — see note below)` | `implemented` | `tool-renderers/registry.ts` + `registry.test.ts` |
| `migrations/runner.rs:50` | "17 of 19 registered migrations" | count removed | registry holds **34** |

**The CLAUDE.md edit is a status correction, not a policy change.** The rule it describes — a
`transcript_request`'s `ESCALATE=required` is not relaxed by a verified sender — is
byte-for-byte unchanged. Only the claim about whether that rule is enforced in code changed, and
it was already contradicted twice elsewhere in the same file (both corrected 2026-09-06).
Flagged explicitly because that section instructs readers to distrust inline corrections.

---

## 5. What genuinely remains, in priority order

### 5.1 Jekt cross-channel trust Phase C — small, but deliberately held 🟡 **[verified]**

`backend/reactive/handler.rs:715-721` checks only `req.channel_verified == Some(true)`; its own
comment says `Some(false)` is "NOT yet among the forcing rules above — Phase C, deliberately
held back", and `tests.rs:1588` pins the current behaviour. Mechanically this is one condition
plus flipping that test. **Do not just do it:** the hold is waiting on published keys to
propagate (spec §6/§10), so shipping early turns legitimate traffic sensitive. Needs a
repo-owner call on timing, plus the §9 two-live-instance end-to-end check that has never been
run for Phase A or B.

### 5.2 Container agents — one real blocker, one stale belief 🟡 **[verified]**

- `AGENTMUX_LOCAL_URL` is never inserted into the per-turn `env_vars`. `input.rs:404` asserts it
  "is already in the inherited process env" — true for host children, **false for `docker exec`**,
  whose env is built solely from `config.env_vars`. No `host.docker.internal` / `extra_hosts`
  anywhere in srv. The sidecar is genuinely unreachable from inside a container.
- **The audit's other container claim is already stale:** the working dir *is* mounted
  (`ContainerMountSpec.workspace_host_dir` → `/workspace`, landed 2026-09-02), so `.mcp.json` is
  visible in-container. What actually breaks is narrower: `docker/Dockerfile.agent-agentmux`
  ships neither `agentmux-mcp` nor `agentmux-bashwrap`, and `PATH` is in
  `CONTAINER_ENV_DENYLIST`, so `"command": "agentmux-mcp"` and the bashwrap PreToolUse hook
  cannot resolve.

### 5.3 Decision-prompt permission UI (#551 / #1469) — the clearest half-migration 🟡 **[doc-claim]**

93 days stalled. Triage on #551 records that the scaffolding all exists — `pending_approval`
status, `AgentDecisionPanel`, the `tooldecision` RPC, the translator hook — **but is not wired
end to end**, and `persistent.rs` still auto-allows every tool call. This is exactly the shape
the 09-06 audit's closing note described, and it is a security-adjacent surface, which makes it
the highest-value *functional* item left.

### 5.4 Wave → Mux rename (#851) — decide, don't drift 🔴 **[verified]**

Open 125 days, zero comments, explicitly deferred the day it was filed. It is **getting worse**:
`WaveEvent` 223 → **232**, `WaveObjUpdate` 93 → **103**, source files containing any `wave`
reference 285 → **306**. The audit asked for a decision and none was made. Schedule it or close
it — an open rename issue signals "we are mid-migration" to every new reader, which is the most
expensive of the three states.

### 5.5 Docs status backlog — flat at 357, and already instrumented 🟡 **[verified]**

898 specs (excluding `archive/`): **148 with no Status line, 209 non-canonical** — 357 total
versus the audit's ~360, i.e. **flat while the tree grew by 41**. New specs are compliant; old
ones are untouched, exactly as `check-doc-status.sh`'s changed-files-only scoping intends. Top
offenders: `ready` ×35, `spec` ×33, `design` ×13, `approved`/`analysis` ×11. Nothing schedules
the burn-down. Issue **#3216** (`docs-stale-sweep`, auto-updated weekly, backed by
`scripts/docs-stale-sweep.mjs`) already reports a wider 1,330-doc scan with 453 flagged — the
instrumentation exists; the burn-down does not.

### 5.6 Automate §3's reverse check 🟢 **recommended**

The "claims unbuilt, but its artifact is cited in source" query found three real corrections in
a single pass and is a handful of lines. `docs-stale-sweep.mjs` already owns this niche; adding
the check there would catch the ABF-class error automatically instead of a month later. **This
is the highest-leverage item in this report** — it is the one that stops the next audit from
being necessary.

### 5.7 Issue hygiene 🟢 **cheap**

- **#1400** is now a duplicate of #2939 workstream 1 (its only two comments say so) — close into it.
- **#2024** — all three items answered; successor work parked by repo-owner call. Close or re-title.
- **#1960** closed 2026-07-12 with all six gaps verified; any doc still describing A4–A7 as pending is wrong.
- **#950** — `term.type` exists *only* in `docs/specs/app-api-extension.md`, not in server code: a doc promising an API that does not exist.
- Eight tracking discussions last moved 2026-06-14/15 (~93 days) and are effectively abandoned.

### 5.9 Jekt WAN sender-binding — built, inert, and nearly lost from the backlog 🟡 **[verified]**

`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13`’s title is "host-tier signing **+ WAN binding
enforcement**". The host half shipped; the WAN half did not. `checkAgentBinding` in
`agentmux-cloud` only warns, because `ENFORCE_AGENT_BINDING` is set in no CDK/Lambda
environment config, and the spec’s §4.1/§6 require live verification and burn-in before the
flag is flipped.

**Worth recording as a process finding, not just a backlog item.** This report’s first pass
restamped that spec `implemented` on the strength of the host tier alone, which would have
removed it from `INDEX.md`’s active backlog entirely and silently retired the sender-binding
work — committing, inside the very PR arguing that bad statuses cause bad planning, the exact
error it argues against. Caught in review (codex P2, PR #3284). The generalisable rule: **a
spec whose title names two halves cannot be closed by one of them**, and “verified against a
named code artifact” is necessary but not sufficient — the artifact has to cover the spec’s
*whole* scope.

### 5.8 `agent-view.tsx` 🟡

3,017 lines and growing through the refactor meant to shrink it. A6's remaining half is tracked
in `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md`; nothing caps the file's growth meanwhile.

---

## 6. Two findings rejected on inspection

Recorded because both looked actionable and both would have done damage.

### 6.1 "Six migrations are permanent no-ops; annotate them Retired" — **wrong, do not do this**

The claim: `m0026_registry_agent_id_rekey` (and `m0007`, `m0017`, `m0025`, `m0027`, `m0028`)
probe `db_agent_instances`, which `m0029` drops at schema v32, so they can never do work again
and should be marked `Retired` like `m0013`/`m0014`.

**They are not dead.** The registry is an ordered list and `m0026` runs *before* `m0029`
(`mod.rs:243` vs `:246`). On an install upgrading from pre-v32 the table is still present when
`m0026` runs, it does real work, and only then does `m0029` drop it. These are inert **on fresh
installs only** — which is correct and harmless, since a fresh install has nothing to re-key.
Marking them `Retired` would invite a future cleanup to delete migrations still load-bearing for
every upgrading install, and `m0026`'s own doc comment explains that its absence causes duplicate
picker rows and resurrected "forgotten" agents.

`m0026`'s doc comment ("the legacy table, which still exists until Phase 3c drops it") is
likewise **accurate, not stale** — it describes the ordering relationship, and the table does
still exist at the point that migration runs.

### 6.2 "334 keep-in-sync comments across 210 files" — **not reproducible**

The 09-06 DRY audit's headline duplication metric does not reproduce from its own stated grep:
the same pattern set returns **95 matches / 78 files** today, literal `"keep in sync"` returns
19/17, and broader variants overshoot to 663/349. Treat the figure as unverifiable rather than as
a target that was met — the underlying causes (§2.6) are real regardless.

---

## 7. Scoreboard

| # | Initiative | 09-06 | Today | What remains |
|---|---|---|---|---|
| 1 | Tauri → CEF | ✅ | ✅ | comment-only residue |
| 2 | React → SolidJS | ✅ | ✅ | — |
| 3 | Reducer/saga (E–H) | ✅ | ✅ | — (authority claim withdrawn) |
| 4 | Arch refactor A1–A15 | 🟡 12/15 | 🟡 **13.5/15** | A6 second half; A10 (optional) |
| 5 | muxspect A/B/C | ✅ | ✅ | — (CLAUDE.md corrected here) |
| 6 | Migration system hardening | 🟡 | ✅ **all but Phase 4** | Phase 4 = product decision |
| 7 | Docs lifecycle hardening | 🟡 ~3/6 | 🟡 **5/6** | Phase 2; 357-file backlog |
| 8 | Jekt cross-channel trust | 🟡 A–B | 🟡 A–B | Phase C (held), D |
| 9 | Container agents | 🟡 | 🟡 | `AGENTMUX_LOCAL_URL`; Dockerfile tooling |
| 10 | Armory foundation consolidation | 🟡 | 🟡 | Naming consolidation Phases 3–4; §3.2/3.3/3.5/3.6 have no follow-up |
| 11 | Mandatory ABF rethink | 🔴 not started | ✅ **shipped** | step 4 "(if wanted)"; §7 needs a decision |
| 12 | DRY / modularity | 🔴 1/5 | 🟡 **2/5** | codegen; twin primitives; saga/reducer duplication; call-site sweep |
| 13 | Wave → Mux (#851) | 🔴 | 🔴 **worse** | decide: schedule or close |
| 14 | Agent working-state unification | 🟡 P1 | 🟡 P1 | Phase 2 investigated-not-attempted; 3 and 4 not started |

---

## 8. Recommendation

1. **§5.6 — automate the reverse staleness check.** Everything in §2 and §4 was found by one
   query that nothing runs on a schedule. This is the fix that stops the next audit being needed.
2. **§5.3 — finish the decision-prompt wiring (#551 / #1469).** The most valuable functional
   half-migration left, and security-adjacent.
3. **§5.4 — make the #851 call.** Costs one comment, stops four months of drift.

Phase C (§5.1) and ABF §7 are both one-decision-away items owned by the repo owner, not
engineering backlog. Neither should be picked up by an agent unilaterally.

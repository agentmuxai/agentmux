# Report: migrations wrap-up — re-verification of the 09-06 audit, ten days on

**Status:** active — living status doc, updated as its own §5 items land. Every doc
correction it recommends for itself was made in the PR that created it (§4). Since then:
§5.4 (Wave → Mux, #851) is **done and the issue closed**. §5.4a tracks DRY/modularity: four
of its five named causes are closed or dismissed, but the one that carries real ongoing cost
(cause 1, RPC codegen) is a long grind now **ten domains in** — #3291, #3293, #3294, #3295,
#3296, #3297, #3299, #3305, #3307, #3308, #3310, #3311, #3312, #3313 — with six stub domains
still to go. Nearly every one surfaced a defect no existing CI gate could catch, including a
P0 that would have broken `bookmarks.list` on every call.

**The shape of the remaining work changed materially on 2026-09-17** (§5.4b): a quarter of
the stub surface turned out to be dead code, not un-migrated code. Remaining open items are
§5.1-5.3 and §5.5-5.9.
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
- **A10 — re-verified 2026-09-17, and the issue's own framing is slightly off.** It reads
  "consolidate data-dir resolution onto `DataPaths`", which implies duplicated resolution
  logic. There is not much: `agentmux-launcher`'s `DataPaths` is a documented compat shape
  that *wraps* `agentmux_common::DataPaths` rather than re-resolving, and
  `get_mux_data_dir()` (29 callers) is a different concept — the **global `~/.agentmux`
  root**, which the agent registry, transcripts and definitions deliberately use
  cross-channel (#1387–#1393), not the per-instance dirs `DataPaths` resolves.

  What IS real: **two independent env overrides for the same root, each honoured by only
  half the code.** `get_mux_data_dir()` reads `AGENTMUX_DATA_HOME`;
  `DataPaths::resolve_root()` reads `AGENTMUX_HOME_OVERRIDE` (documented test-only). With
  neither set — the normal case — both resolve to `~/.agentmux` and agree, which is why
  nothing has broken. Set either one and the two halves disagree about where the root is.

  Checked and NOT a live bug: `registry::write` takes `data_dir` as a parameter, so tests
  inject a tempdir and do not write through `get_mux_data_dir()` into a real home.

  Still fair to call low priority, but "safe to skip" undersells it — the finding above is
  the part worth keeping if A10 is closed unfinished.

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
exists in the tree (`agentmux-common/src/win32.rs`) — after this PR also migrated two
test-local copies in `agentmux-srv/tests/subprocess_io.rs` that an earlier `agentmux-*/src`
search had missed, because integration tests live outside `src/` (codex P3 on #3291) — and 24 import
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

### 5.4 Wave → Mux rename (#851) — ✅ **DONE 2026-09-16, issue closed**

*Was:* open 125 days, zero comments, deferred the day it was filed, and measurably getting
worse (`WaveEvent` 223 → 232, source files containing any `wave` reference 285 → 306).
This section asked for a schedule-or-close decision. Both halves were answered:

- **#3285 (phase 1)** — every Wave-derived identifier: types, camelCase, Rust snake_case,
  three file renames, plus 9 dead Wave Terminal AI types deleted.
- **#3287 (phase 3)** — the abbreviations: `WOS`/`wos` → `MOS`/`mos`, `WPS`/`wps` →
  `MPS`/`mps`, `wstore` → `mstore`, `gotypes.d.ts` → `srv-types.d.ts`, five module files.

**Result: zero Wave-derived identifiers remain. Source files containing any `wave`
reference: 306 → 91**, and every one of those 91 is a deliberate string contract.

*This claim was wrong twice before it was right, which is worth recording in full because it
is the report's own thesis happening to the report.*

- **First version** was produced by patterns matching `Wave[A-Z]`, `<char>Wave[A-Z]` and
  `wave[A-Z]`. None match SCREAMING_SNAKE, so every `WAVE_*` constant was invisible; and the
  globs were `agentmux-*/src`, so `agentmux-srv/tests/` was out of scope entirely.
  (codex P2 on #3291 and #3292.)
- **Second version** claimed verification "against a case-insensitive sweep" — but that
  sweep used `[A-Za-z_][A-Za-z0-9_]*[Ww][Aa][Vv][Ee]…`, whose **mandatory leading character**
  means it cannot match an identifier that *starts* with `WAVE`. So `WAVE_DATA_HOME_ENV`,
  `WAVE_APP_PATH_ENV`, `WAVE_DEV_ENV`, `WAVE_DEV_VITE_ENV`, `WAVE_JWT_TOKEN_ENV`,
  `WAVE_SWAP_TOKEN_ENV`, `WAVE_DB_DIR` and the `WAVE_VERSION` OnceLock survived a sweep
  advertised as exhaustive — in the very file the PR was editing. (reagent P1 on #3292.)

**Verification method, stated so it can be re-run rather than trusted:**

```bash
grep -rhoE "[A-Za-z0-9_]*[Ww][Aa][Vv][Ee][A-Za-z0-9_]*" \
  --include=*.rs --include=*.ts --include=*.tsx --include=*.d.ts \
  agentmux-*/src agentmux-srv/tests frontend | sort -u
```

The leading quantifier is `*`, not a mandatory character — that single difference is what the
second version got wrong. Everything it returns is now either a retained string contract or
the English word "waves" in an audio test.

All thirteen renamed constants keep their string VALUES untouched: `"wave.lock"` is a real
lockfile on disk, `"waveobj:update"` / `"waveobj:batchedupdates"` are wire events, and the
rest already read `"AGENTMUX_*"`. One stale comment referencing the renamed `initHostWave`
was fixed too.

Still deliberately retained, all string contracts rather than branding: `db_wave_file`,
`waveobj:*` and the `raw_waveobj_update` / `handle_wps_publish` handlers named after the wire
events they serve, the `__WAVE_SERVER_*_ENDPOINT__` window globals (a host↔renderer contract),
`getwaveairatelimit`, `--wavedata`, and the `WAVEMUX_AGENT_ID` / `~/.waveterm` /
`wavepwsh.ps1` legacy trio.

**Phase 2 (the strings) was deliberately NOT done, and the issue is closed saying so.** The
deciding evidence came from doing phase 3: it shipped a P1 that neither the compiler nor
2,960 tests caught — the frontend caller kept POSTing to `/agentmux/mps/publish` while srv
registers `/agentmux/wps/publish`, fire-and-forget, so it 404'd silently and broke
cross-window singleton coordination. A reviewer caught it. Phase 2 is *entirely* that class
of change: `"waveobj:update"` (WS discriminator), `db_wave_file` (needs an `ALTER TABLE`
against real user data), `/agentmux/wps/publish` (needs a compat alias — bashwrap is a
separate binary invoked from shell-integration scripts on disk), `waveblock` (SCSS lockstep).
Zero user-visible benefit, no automated safety net, one of them touching persisted data.

Three more must **never** change, being compatibility rather than branding — a naive
"finish the rename" sweep breaks all of them: `~/.waveterm` (the real legacy directory
`m0001` migrates users off), `WAVEMUX_AGENT_ID` (documented legacy env fallback, still read),
`wavepwsh.ps1` (a script filename on disk that user shell profiles reference).

### 5.4a DRY / modularity (row 12) — four of five causes closed or dismissed 🟡

Worked 2026-09-16. Each cause was re-verified against code before acting, and **three of
the five turned out not to be what the audit's metric implied** — the metric counted named
causes and raw occurrences rather than actionable duplication.

| Cause | Outcome |
|---|---|
| 1. No Rust↔TS codegen | **Template shipped, ~5% migrated** — #3291 merged. See below. |
| 2. `agentmux-common` underuse | **Done** — #3289. See §2.6's correction. |
| 3. Platform-forked `zoom.*.ts` | Already done before this report (#3034). |
| 4. `mcp` ↔ `skill` twin primitives | **Verified real, recommended against.** |
| 5. Parallel saga/reducer frameworks | **Not duplication.** |

**Cause 1 was mis-framed, and the real gap was sharper.** The audit said "no Rust↔TS
codegen". In fact `SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md` §3.4 step 1 had shipped —
`register_typed`, the `RpcSchema` registry, ts-rs export and `scripts/check-rpc-bindings.sh`
were all live with 12 generated types. What had not happened is step 2, and specifically:
**nothing in the frontend imported a single generated type.** All 12 were generated, gated
against drift, and unused, while the hand-written stubs kept inline duplicates of the same
shapes — a pipeline built and never connected at the far end. #3291 connects it end to end
for `voice.checkPath` and gives the remaining 16 stub files a template.

**This is a template, not a completed cause, and the scoreboard says so deliberately.**
Measured after #3291 merged: **286 hand-written stub functions across 16 files, exactly one of
which imports a generated type; 14 generated bindings against 195 `rpc_types` structs; 12
`register_typed` call sites against 232 `register_handler`; `srv-types.d.ts` still 2,755
hand-maintained lines.** Roughly 5%.

#### Progress since (updated 2026-09-17)

| Domain | PR | What it did |
|---|---|---|
| `voice.checkPath` | #3291 | First end-to-end connection; template for the rest. |
| `bookmarks.*` | #3293 | Found the **P0**: `Req = ()` rejects the `{}` the stub sends. |
| `reactive.registrations` | #3294 | Deleted 4 mirror interfaces; fixed a P2 where `skip_serializing_if` made `tab_id` `string \| null` when the key is *omitted*. |
| `agent:memory:*` | #3295 | 13 of 14 generated; first type ts-rs **cannot** express (`#[ts(optional)]` rejects non-`Option`). |
| `session:*` (11 cmds) | #3296 | 23 types; first case where generating was **worse** — `verdict` would have degraded from a union to `string`. |
| `fleet.*` | #3297 | Two commands had no Rust type at all. |
| `bundle` CRUD | #3299 | `Bundle` was both request and response and could not correctly be either; `Partial<Bundle>` wrongly allowed omitting `name`. |
| `bundle.validate` | #3305 | First handler **deliberately left untyped** — it normalizes before deserializing. |
| `bundle.import.*` | #3307 | Authoring job; surfaced a field (`project_instructions`) and two input modes the typed client could not reach. |
| `blockfile:*` | #3308 | 4 commands; two adjacent fields with opposite optionality semantics. |
| block (websocket) | #3310 | 8 commands; corrected the survey from #3308 (see below). |
| block dead stubs | #3311 | **Deleted** 10 commands no handler serves. |
| dead-stub sweep | #3312 | **Deleted** 55 more across misc/workspace/file. |
| `misc` (muxbus, providers) | #3313 | 6 private structs promoted; 2 more commands left untyped by design. |
| agent core (12 cmds) | #3327 | `getagentcontent` was recording `Value` in the registry despite an identical wire shape. |
| template / fork (3 cmds) | #3328 | Finished `template.rs`; `Option<Req>` for both encodings of "no argument". |
| Drone pane (6 cmds) | #3329 | Whole type surface; found ts-rs ignoring `serde(rename)` on a field. |
| editor mutations (7 cmds) | #3331 | 14 shapes out of closure-local `struct Cmd`s. |
| agent-instance (4 cmds) | #3332 | 9 fields typed `?:` that the server always sends. One deliberate behaviour change. |
| editor reads (5 cmds) | #3333 | 4 "back-compat optional" fields the server has never omitted. |
| agent-skills (4 cmds) | #3334 | Surfaced a create/update `skill_type` default asymmetry. Not fixed — product call. |
| LSP (3 cmds) | #3330 | Three requests that existed only inside closures. |
| editor watchers (4 cmds) | #3337 | Finished `editor_handlers.rs`. |
| agent-history (3 cmds) | #3339 | No drift; clean. |
| cli / toolchain / widget (7 cmds) | #3344 | Revised the §5.4c rule — see below. Third `ts(rename)` sighting. |
| ts-rs claim correction | #3345 | Retracted a wrong claim this report repeated. See below. |
| shell / agent-input (7 cmds) | #3342 | `exit_code?: number` where the key is always present-and-null. |
| websocket (10 cmds) | #3348 | `MuxInfoData` promised 5 fields; the handler sends 1. |

Measured 2026-09-17 (end of the effort), same greps as the original numbers.
`#3342` and `#3348` were still in review at the time of measurement, so the
final two columns understate by ~17 commands:

| metric | at report creation | mid-effort | now |
|---|---|---|---|
| `srv-types.d.ts` | 2,755 lines | 2,222 | **1,698** |
| declared commands | 277 | 221 | **221** |
| declared-but-unregistered | 69 | 14 | **14** |
| generated bindings | 14 | 123 | **280** |
| stub files importing generated types | 1 | 9 of 16 | **15 of 18** |
| `register_typed` vs `register_handler` | 12 / 232 | 63 / 181 | **170 / 66** |

That is **72% of registrations typed**, from 5%. `srv-types.d.ts` is down 38%.

#### 5.4b The finding that changed the plan: 25% of the stub surface was dead

Mapping `block`'s remaining commands turned up ten with **no backend handler**. That is not
new information — `test/contract/rpc-contract.test.ts` already tracked them in a
`KNOWN_DECLARED_UNREGISTERED` allowlist, and the first instinct to report it as a discovery
was wrong. What *was* new is the scale, measured across all 16 stub files: **69 of 277
commands (25%)**, concentrated in `file.ts` (23/42), `workspace.ts` (22/39), `misc.ts`
(14/22) and `block.ts` (10/26).

Those were the domains that looked largest by line count. More than half of `file` and
`workspace` was deletable rather than migratable. #3311 and #3312 removed 65 of the 69.

**Two qualifications learned while deleting them**, both of which would have caused a bad
deletion if missed:

- `captureblockscreenshot` looked identically dead but is a **reverse RPC** — the frontend
  implements it and the backend calls it. "No server handler" is not sufficient evidence a
  command is dead; the `tabrpcclient.ts` direction has to be checked too.
- **11 unregistered commands are still actively called** (`connconnect`, `connlist`,
  `workspacelist`, `resolveids`, `fileappend`, `filejoin`, …). Those fail at run time today.
  They are pre-existing and tracked, and were deliberately left alone: fixing them means
  wiring a handler or deleting a caller, which is product work. **This is the single most
  actionable thing the contract data shows and it is nobody's assigned task.**

#### 5.4c Not every handler should be typed

The spec's step 2 reads as "migrate every handler to `register_typed`". Three commands are
now deliberate exceptions, and the pattern is consistent enough to be worth stating as a rule:

- **`bundle.validate`** runs its payload through `normalize_bundle_upsert_input` *before*
  deserializing — that is what fills a missing `id` so an unsaved draft validates, and what
  coerces array-valued fields into the JSON strings `Bundle` expects. `register_typed`
  deserializes first, so there is no point at which normalize could run.
- **`widget.health` / `widget.api`** read fields straight off a `Value` with lenient
  defaults — a missing `port` becomes `0`, which the handler answers with `{healthy:false}`
  rather than an error. A typed `Req` would turn that into a deserialization failure.

**Rule (revised 2026-09-17, #3344):** a handler that transforms its payload before
deserializing, or that deliberately accepts loose input, should have its `Value` request
**registered as `Value`** — not left on `register_handler`.

The original rule said such handlers had to stay untyped entirely, which was stronger than
necessary. `serde_json::Value` deserializes from *anything*, so `register_typed::<Value,
Resp>` is exactly as tolerant as the untyped handler while still recording the response
type — and the response is where the drift risk actually lives. `widget.health` and
`widget.api` were converted this way in #3344.

`bundle.validate` and `listagents` are still on `register_handler` and could take the
same treatment; that was left as a follow-up rather than done inside an unrelated slice.

#### 5.4d Where generating produces a WORSE type

Four `block` structs are carved out (five before `createblock` was deleted as dead), and the
reasons generalise:

- **ts-rs cannot express an optional property whose Rust type is not `Option<T>`.** Any field
  that is `#[serde(default)]` or `skip_serializing_if` on a `bool`/`String`/`Vec` is
  omittable on the wire but generates as **required**.
- **A Rust `String` does not carry its domain.** `verdict`, `collision`, `outcome`, `scope`
  and `status` all carry closed sets the frontend branches on; ts-rs emits a bare `string`.
  Each needs an explicit `#[ts(type = ...)]` or real safety is silently lost.
- **The hand-written TypeScript is sometimes richer than the Rust.** `CommandCreateBlockData`
  typed `blockdef: BlockDef` where Rust had `Option<serde_json::Value>`. Generating would
  have emitted `unknown`. Fixing it properly means tightening the *Rust*, which is a
  behaviour change, not a refactor.

#### What the migration keeps finding: the gate does not check what you think

Every domain so far has surfaced a defect that **no existing check could catch**, which is the
strongest argument for continuing:

- `tsc` checks the frontend against the *generated* types, not against the backend.
- `check-rpc-bindings.sh` checks that a generated type *exists* per command, and that it is
  current — never that a real payload deserializes into the Rust `Req`.

So a `Req` that cannot parse what the stub sends **compiles, typechecks, and passes the
binding gate**, then fails on every call. That is precisely the `bookmarks.list` P0. Each
migrated domain therefore now carries `req_shape_tests` pinning each command to the literal
JSON its call site sends, including negative assertions (e.g. `agent:memory:diff` must
*reject* a payload with no `agent_id` — the ownership check behind an earlier P1).

#### A real expressiveness limit in ts-rs, found in #3295

`NativeMemoryWriteProvenance` **cannot be generated** and is the one type left hand-written.
Its `detail` field is a `serde_json::Value` the frontend has always treated as an optional
property, and ts-rs refuses `#[ts(optional)]` on anything that is not `Option<T>`:

```
error: `optional` can only be used on an Option<T> type
```

The tempting fix — change the field to `Option<serde_json::Value>` — would let the generator
dictate wire behaviour, since `default_detail()` exists so an omitted detail becomes `{}`
rather than `null`. **Expect this to recur:** any struct with a defaulted non-`Option` field
that the frontend treats as optional is inexpressible, and the right answer is to leave it
hand-written with a note on both sides, not to reshape the Rust to suit the tool.

#### 5.4e Three ts-rs traps, one of which this report got wrong

**Real, load-bearing: ts-rs ignores per-field and per-variant `#[serde(rename = "...")]`.**
The field generates under its *Rust* name — a key nothing sends and nothing reads — and
every gate stays green, because the generated file is internally consistent and simply
describes a wire format that does not exist. Three sightings: `oauth_config_dir` (#3320),
`FlowNode.type` → `node_type` (#3329), `pathSource` → `path_source` (#3344). All three
were caught by *reading generated output*, not by any check. There is one now —
`scripts/check-rpc-codegen-hygiene.mjs`, wired into `ci-pr.yml` beside the bindings gate.

**Wrong, and retracted in #3345: ts-rs does NOT ignore `#[serde(rename_all)]`.** #3329
added `#[ts(rename_all)]` to two enums with a comment asserting it did, and an earlier
revision of this report repeated that. The claim was never tested — it was generalised from
the per-field case above. Two counter-examples were already in the tree: `ToolStatus`
(`rename_all = "snake_case"`, no `ts` attribute, generates a correct snake_case union) and
`InstallStartReq` (`rename_all = "camelCase"`, generates `providerId`/`cliCommand`).
Removing the redundant attributes left the generated output **byte-identical**, which is
the proof they were inert.

Worth recording as a method note: the first version of the scan for this matched
`rename_all` too and reported 44 hits. Investigating them is what surfaced the error. A
noisy check that gets read beats a quiet one that gets trusted.

#### 5.4e-bis The four checks, and why none of them is `tsc`

`scripts/check-rpc-codegen-hygiene.mjs` carries four checks, each written after the same
defect got past review more than once. They share one property worth stating plainly:
**every failure they catch compiles, typechecks, AND passes `check-rpc-bindings.sh`.**

`cargo check` sees only the Rust half. `tsc` checks the frontend against the *generated*
types, never against the backend — so a binding that describes a wire format nobody speaks
typechecks cleanly on both sides. And the bindings gate verifies a generated file *exists*
per type and is *current* with the Rust, which a file can be while still being wrong about
the wire.

| check | what it catches | prior sightings |
|---|---|---|
| ambient duplicates | a global shadowing its own generated replacement; consumers that do not import silently resolve to the stale one | #3318, #3320 (×2), #3327 |
| `serde(rename)` without `ts(rename)` | a field generated under its Rust name — a key nothing sends and nothing reads | #3320, #3329, #3344 |
| typed handler hand-serializing | registry records `Value` instead of the real response type; wire bytes identical, so nothing else notices | #3327, #3331 |
| unused generated imports | leftovers from a migration | an earlier slice left nine |

Two of the four had a **false-negative or false-positive bug of their own** before they
were correct: the hand-serialize check originally matched `to_value` but not inline
`json!`, and passed `movescratchfile` clean while a test caught it (#3331); the
unused-import check ignored `X as XT` aliases and reported eleven healthy imports as dead.
Both are worth knowing about — a check is a piece of software with its own defects, and an
unexamined green is not evidence.

#### 5.4f "No argument" has two encodings, and the stubs disagree

An empty-struct `Req` rejects `null`; a unit `()` `Req` rejects `{}`. Which one breaks
depends on the stub, and **the stubs are not consistent**: most send `{}`, but every
no-argument stub on the websocket connection sends `null`
(`client.rpcCall("waveinfo", null, opts)` and four more). The earlier slices assumed `{}`
universally and would have broken those five.

`Option<Req>` accepts both and is now the standard for any command that takes no argument.

#### 5.4g Honest cost accounting

Three of the most-cited findings from this effort — `oauth_config_dir`, `FlowNode.type`,
`pathSource` — were bugs **the migration itself nearly introduced**. They were all caught
before merge, but that is the migration paying down risk it created, not value it
delivered. A fair ledger has to say so.

The value that is unambiguously banked: 65 dead stubs deleted, 11 called-but-unregistered
commands surfaced (§5.4b — still the most actionable item here), and a set of type lies
corrected where the hand-written declaration disagreed with what the server actually sends
(`AgentInstance`, `CommandReadEditorFileResult`, `ShellStatusResult.exit_code`,
`MuxInfoData`, `CommandBlockInputData.seq`). Most of those were wrong in the *safe*
direction — optional where the server always sends — which is why few produced live
crashes.

The forward value is the gate: `check-rpc-bindings.sh` now fails CI on Rust↔TS divergence
for 72% of registrations. `rpc_types/` sees ~90 commits in 6 months, and this repo does
sweeping identifier renames roughly every six weeks (Wave→Mux, Memory→Bundle,
Preset→Bundle, Trust Center→Armory). A mass rename is exactly the operation where a
hand-maintained mirror rots silently, and exactly what the gate now catches.

#### 5.4h Stopping point

The effort was stopped deliberately at 72%, not abandoned. What remains untyped:
`app_api/*` (bundle 12, agent_io 9, identity 4, memory 3 — MCP-facing, narrower blast
radius), `agent_handlers/identity.rs` (6), the seven `websocket.rs` commands that drag in
`ORef`/`MetaMapType`/`BlockDef`, and a handful of singles. Plus the two deliberate
carve-outs (`bundle.validate`, `listagents`) that §5.4c's revised rule now makes
convertible.

The judgement: the highest-value surfaces are done, and the remaining commands are the
lowest value per unit of churn — every slice touches the same three shared files, so the
rebase tax is constant regardless of slice size.

**Cause 4 — real duplication, but abstracting it would be premature.** `McpCatalogModel` and
`SkillCatalogModel` share 10 method names, and the bodies are near-identical (`saveDraft`
differs only in the RPC payload and one validation step). But there are exactly **two**
instances, they already diverge (mcp has catalog-picker methods skill has not), the view
layer is *already* shared via `PrimitiveListDetail` across 9 files, and a generic base would
buy ~150 lines while adding indirection to live reactive UI state. `bundle-mcp-model` is not
a third instance — it binds rather than managing a catalog. Recommend leaving these until a
genuine third primitive appears.

**Cause 5 — not duplication at all.** The launcher and srv saga/reducer systems share
exactly one type name (`Ctx`, a generic) and operate on different domains: window
pools/pipes/respawn versus blocks/tabs/workspaces/layout. The DRY audit's own wording was
"sharing only vocabulary", which is accurate — it was then scored as a defect anyway.

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
| 12 | DRY / modularity | 🔴 1/5 | 🟡 **cause 1 ~72% done, stopped deliberately; 2,3 done; 4,5 dismissed** | Do not read "4/5" as progress — it counts named causes, not work. Twenty-five domains migrated and 65 dead commands deleted (#3291–#3348). **15 of 18 stub files import generated types; 280 bindings; 170 `register_typed` vs 66 `register_handler`; `srv-types.d.ts` 2,755 → 1,698; declared-but-unregistered 69 → 14.** Stopped at 72% by decision, not exhaustion — remaining: `app_api/*` (28), `agent_handlers/identity.rs` (6), 7 `websocket.rs` commands needing `ORef`/`MetaMapType`/`BlockDef`. See §5.4b (a quarter of the surface was dead), §5.4c (revised: loose-input handlers take a `Value` request, not no typing), §5.4d (where generating produces a worse type), §5.4e (a claim this report got wrong), §5.4g (honest cost ledger) and §5.4h (why here). |
| 13 | Wave → Mux (#851) | 🔴 | ✅ **done, #851 closed** | 0 Wave identifiers; 306 → 91 files (#3285, #3287). Strings deliberately out of scope — §5.4 |
| 14 | Agent working-state unification | 🟡 P1 | 🟡 P1 | Phase 2 investigated-not-attempted; 3 and 4 not started |

---

## 8. Recommendation

**Updated 2026-09-16** as items landed. Original three, with outcomes:

1. ~~**§5.6 — automate the reverse staleness check.**~~ **Still the top item, still not done.**
   Everything in §2 and §4 was found by one query that nothing runs on a schedule, and the
   pattern kept repeating all day: `SPEC_RPC_BINDINGS_CODEGEN`'s Status still says "no
   generator exists yet" while its generator, gate and 14 bindings are live (§5.4a). This is
   the fix that stops the next audit being needed.
2. **§5.3 — finish the decision-prompt wiring (#551 / #1469).** Unchanged, and now the most
   valuable functional item left: scaffolding exists, `persistent.rs` still auto-allows every
   tool call, security-adjacent.
3. ~~**§5.4 — make the #851 call.**~~ ✅ **Done.** Decided by doing: the identifier half
   shipped (#3285, #3287), the string half was declined with reasons, and #851 is closed.

Phase C (§5.1) and ABF §7 remain one-decision-away items owned by the repo owner, not
engineering backlog. Neither should be picked up by an agent unilaterally.

### What the day actually taught

Five separate audit claims were checked against code and **four were wrong in the
"actually done" or "not really a problem" direction**: mandatory ABF (§2.2), migration
failure being silent (§2.1), the `agentmux-common` call-site sweep (§2.6's correction), and
DRY causes 4 and 5 (§5.4a). One was wrong in the other direction and nearly shipped a bug:
the jekt trust spec looked complete enough to restamp `implemented` when only its host half
had landed (§5.9).

The common thread is that **these reports' metrics are load-bearing and unverified**. Counts
of occurrences, files, or named causes get read as counts of problems. Before acting on any
line in this document, re-derive it — that is what §5.6 exists to automate, and why it stays
at the top of this list.

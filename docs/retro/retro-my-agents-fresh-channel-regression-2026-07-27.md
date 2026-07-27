# Retro: "My Agents" empty on a fresh `task package` portable (0.54.6)

**Date:** 2026-07-27
**Severity:** P1 if it recurs (the entire custom-agent roster — ~24 agents including "AgentA" — appears wiped), but **not reproduced live** and **not currently explained by any bad data on this machine**
**Status:** Partially root-caused. One architectural fragility is **confirmed** (by code reading) and **proven exploitable** (it already caused this exact symptom once, 2 days earlier, via PR #2296). A live server log from the user's *actual* reported session was found and analyzed, but it does not contain enough detail (no per-RPC-command tracing) to prove what the frontend rendered. Root cause for **this specific incident** is not pinned down — see §6 for the honest verdict.

---

## 1. What the user reported

> Built a fresh portable release (`task package`, version 0.54.6), opened it. The
> "My Agents" section of the agent picker — which normally lists ~24 custom
> agents, most importantly "AgentA" — was empty or missing agents.
> "This is a regression, loading DEV instances always came with the My Agents
> populated."

## 2. Ground truth going in (established by direct investigation before git archaeology; not re-derived here)

- "My Agents" = `frontend/app/view/agent/components/MyAgentsList.tsx` (renamed
  from `RecentSessionsList.tsx`, PR #977/#1008 cascade), calling
  `RpcApi.ListRecentSessionsCommand` → backend `COMMAND_LIST_RECENT_SESSIONS`
  in `agentmux-srv/src/server/agent_handlers/session.rs`.
- That handler sources from the **global, cross-channel instance registry**
  (`~/.agentmux/shared/agents/registry/`, `Registry::list_active()`,
  `agentmux-srv/src/registry/store.rs:150-176`), not per-channel SQLite — so a
  brand-new channel is *supposed* to still show every agent ever created,
  anywhere.
- **Direct validation, done live on this machine before this retro:** all 24
  files in `~/.agentmux/shared/agents/registry/*.json` pass
  `schema.rs`'s `validate()` (schema_version, instance_id/filename match,
  non-empty fields, safe relative `working_dir`). All 23 files in the sibling
  `~/.agentmux/shared/agents/definitions/*.json` (`DefinitionStore`, a
  different store backing a different picker section) also pass its
  `def_schema.rs` validate(). **Zero rows would be skipped by validation.**
- `session.rs`'s row-construction does **not** drop a row when its
  `definition_id` has no matching `AgentDefinition` — it falls back to
  `"(missing definition)"` display text. So there is no silent per-row drop in
  the merge logic either.
- A live test-launch of the fresh build from inside an agent shell was
  attempted twice, but the agent's own shell environment leaks an ambient
  `AGENTMUX_CHANNEL` that the launcher's nested-instance detection (correctly,
  per its own documented logic) treats as an explicit standalone override, not
  a leak — so neither attempt exercised a genuinely fresh channel end-to-end.
  **This investigation could not directly reproduce the symptom.**

This retro picks up from there: git/PR archaeology, and a search for whether
this exact complaint is already tracked.

---

## 3. Is this already a known pattern? Yes — repeatedly

`docs/retro/` already has **three** prior incidents in the same feature area,
all within the last six weeks:

| Retro | Date | Symptom | Root cause |
|---|---|---|---|
| `retro-local-build-isolation-regression-2026-06-09.md` | 06-09 | New local build joins an old running instance instead of starting fresh | Pipe-hash key didn't include the build stamp |
| `retro-per-build-launch-isolation-2026-06-13.md` | 06-13 | Same, plus nested-launch channel leak | cef-cache stayed per-branch; ambient `AGENTMUX_CHANNEL` leaked into nested launches |
| `docs/analysis/ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13.md` | 06-13 | Fresh channel shows **zero** custom agents (only 7 seeded templates) | The one-shot definitions-backfill migration skipped 10/12 DBs on a schema-drift error, never scanned `dev/`, and its one-shot marker made the failure permanent |
| `retro-legacy-agent-history-cross-channel-2026-06-16.md` + `retro-cross-channel-conversation-continuity-regression-2026-06-16.md` | 06-16 | Agent *appears in the list* but opens blank / loses history cross-channel | `sourceBlockId` in the global snapshot was channel-local, so the cross-channel read fallback couldn't anchor it |

None of these four is a live match for *this* incident (the registry/definition
data here is 100% valid, ruling out the 06-13 migration-skip bug; the symptom
is "list is empty," not "list shows agents that then open blank," ruling out
the 06-16 pair) — but they establish that "fresh channel, cross-channel
agent-visibility feature, breaks in a new way" is a **recurring failure class**
in this codebase, not a one-off.

More importantly, there is a **direct fourth instance, 2 days before this
report**, that reproduces this incident's exact wording:

---

## 4. The smoking gun: PR #2296 already hit this exact symptom, 2 days earlier

`git log -S"Was a silent" -- frontend/app/view/agent/components/MyAgentsList.tsx`
→ commit `cb8c9eeb` (`fix(identity): oauth_config_dir secret_ref serde rename
mismatch (#2296)`, merged **2026-07-25**, i.e. two days before this report).
Its commit message, verbatim:

> `SecretRef::OAuthConfigDir`'s enum-wide `rename_all = "snake_case"` derives
> `o_auth_config_dir` ... but every real account in `~/.agentmux/shared/store.db`
> was written with `oauth_config_dir` ... `identity_list()` aborts entirely on
> the first unparseable row, so this broke "My Agents"/recent-sessions for any
> instance with a real oauth-class account — **reported live as "what happened
> to My Agents? its an empty list."**

This is not analogous — it is **the identical user-facing symptom**, already
filed once. The fix (`agentmux-srv/src/backend/storage/identities.rs:80`,
explicit `#[serde(rename = "oauth_config_dir", alias = "o_auth_config_dir")]`)
also touched `MyAgentsList.tsx` to replace a silent `catch { return []; }`
with one that at least logs (`MyAgentsList.tsx:128-140`) — its own comment
says the silent version is "indistinguishable from 'genuinely no sessions'…
exactly what made a real regression here look like expected empty state."

**Is this specific bug present in the reported 0.54.6 build?** No —
`git merge-base --is-ancestor cb8c9eeb 178f7cee` (178f7cee = the `chore:
release v0.54.6` commit) confirms it **is** an ancestor; the fix shipped in
0.54.6. Direct inspection of this machine's actual
`~/.agentmux/shared/store.db` `db_accounts` table (2 rows) confirms both use
the canonical, already-fixed `"backend":"oauth_config_dir"` tag — this
specific bug cannot be firing here right now.

### But the *structural* fragility it exposed is still fully present

`session.rs`'s `COMMAND_LIST_RECENT_SESSIONS` handler makes **six** sequential
data-source calls, and aborts the **entire** list on the **first** error from
any of them:

| Call | Line | Source |
|---|---|---|
| `reg.list_active()` | `session.rs:92` | global instance registry |
| `wstore.instance_list_named(...)` (local fallback / append) | `session.rs:201` | per-channel SQLite |
| `wstore.agent_def_list()` | `session.rs:206` | global + local agent definitions |
| `id_store.agent_identity_list_all()` | `session.rs:215` | `db_agent_identity_links` |
| `id_store.identity_list(None)` | `session.rs:218` | `db_accounts` — **the exact call #2296 fixed** |
| `id_store.bundle_memory_list()` | `session.rs:231` | `db_bundles` |

Every one of these uses `.map_err(|e| format!("listrecentsessions: ..."))?` —
**any single malformed row in any of the four SQLite-backed sources, or any
I/O error against either global file-store, still takes down the whole list**,
identically to how #2296's bug did. The fix for #2296 patched the *one*
concrete instance (an enum-tag mismatch); it did not change this handler's
all-or-nothing structure. Nothing prevents the same class of failure from a
*different* malformed row in `db_accounts`, `db_agent_identity_links`, or
`db_bundles` tomorrow.

I directly checked this machine's live data against that risk (same technique
the original investigation used for the registry/definition JSON files, just
extended to the identity/memory-bundle side it hadn't covered):

- `db_accounts`: 2 rows, both parse cleanly against the current `SecretRef` enum.
- `db_agent_identity_links`: 1 row, plain TEXT columns, no enum-tag risk.
- `db_bundles`: 4 rows, plain TEXT/INTEGER/JSON-array columns, no enum-tag risk.
- Grepped `identities.rs` and `agents.rs` for every other `#[serde(rename_all
  = ...)]` — one more exists (`agents.rs:161`, `rename_all = "lowercase"`,
  which doesn't split on capitals the way `snake_case` does, so it isn't
  vulnerable to the same acronym trap) — not independently fuzzed further.

**Conclusion: none of the six data sources is currently broken on this
machine.** The fragility is real and previously proven, but not the live
cause today.

### And the frontend still can't tell "error" from "empty"

`MyAgentsList.tsx:245-246`:

```ts
const isLoading = () => rows() === undefined;
const isEmpty = () => !isLoading() && (rows() ?? []).length === 0;
```

Whether the resource settled to `[]` because the RPC *succeeded* with zero
rows, or because it *threw* and the `catch` block (`MyAgentsList.tsx:120-141`)
swallowed the error into `return []`, renders the **exact same** UI:
`EMPTY_GLOBAL` — *"No agents yet — pick a template below to create your first
one."* (`MyAgentsList.tsx:72-74`). #2296 added a `Logger.error` call inside
that catch, which is real progress for someone who checks logs — but a user
looking at the picker itself sees no difference at all between "you have zero
agents" and "the backend call just failed." No retry affordance, no error
banner.

---

## 5. What the user's *actual* reported session's log shows

The orchestrating investigation identified the exact log file and a ~9-second
session but hadn't read it. I did:
`~/.agentmux/logs/agentmuxsrv-v0.54.6.log.2026-07-27`, lines 1-53.

Confirmed directly from the log (all timestamps UTC, same file):

- `21:03:16.825` — process starts. `data_dir` is
  `channels\local-agenta-release-v0.54.6-347f00-e771f5b3\versions\0.54.6\data`
  — this channel slug is derived from the branch `agenta/release-v0.54.6`
  (the exact branch this retro is written on), confirming this log **is** the
  user's own reported build/session, not a different agent's.
- `21:03:16.981` — `"First launch: created initial data"` /
  `"agent seed: no agents found, seeding from manifest v11..."` → **this
  channel had genuinely never run before.** Confirmed fresh, first-ever boot.
- `21:03:16.945` / `.951` / `.960` / `.962` — `"registry: shared agent
  registry attached"`, `"def registry: global definition store attached"`,
  `"global transcripts: store attached"`, `"shared store: attached"` — **all
  four cross-channel stores attached successfully** in this exact process.
  The backend-side prerequisite for cross-channel "My Agents" sourcing was
  healthy in the reported session.
- `21:03:18.811` — `"WebSocket client connected"` (~2s after process start).
- **No RPC command names appear anywhere in this log** (not for this session,
  nor the two other fresh-channel sessions later in the same file at 22:33
  and 22:36) — RPC traffic isn't traced at INFO level in this build. I cannot
  confirm from the log whether `listrecentsessions` was ever issued, or what
  it returned.
- `21:03:25.372` (~6.5s after WS connect) — **two WARNs**: `"object not found
  in wstore; skipping broadcast"`, `otype:"workspace"`, contexts
  `"TabDeleted parent"` and `"ActiveTab/Reorder"`
  (`agentmux-srv/src/server/wave_obj_bridge.rs:336-361`). Tracing that code:
  a `TabDeleted` event tries to re-fetch and broadcast its **parent
  workspace**'s new state, and an `ActiveTabChanged`/`TabReordered` event does
  the same — but the workspace object was already gone by the time either
  fetch ran. This is consistent with the normal (if racy) bootstrap dance of
  tearing down the seeded default "Starter workspace" and replacing it with
  the real one on a genuinely first launch (the same seeded-workspace
  mechanism `fix(window): first window's title no longer leaks the unrenamed
  bootstrap workspace name (#2319)` — already in this exact 0.54.6 build —
  was independently patching around, for a *different* symptom, the same day).
- `21:03:25.479` — `"WebSocket client disconnected"`. Total process lifetime
  from start to disconnect: **~8.65 seconds** — matches "~9 seconds."

**What this proves:** the reported session was a genuinely fresh, first-ever
channel boot, whose backend-side cross-channel stores all attached cleanly,
and which underwent bootstrap workspace/tab churn (tab deleted, active tab
reordered) within the same few seconds before the window closed.
**What this does not prove:** whether `MyAgentsList` ever completed a fetch,
what it fetched, or whether the tab/workspace churn interrupted or remounted
the component mid-flight. The log format is the limiting factor, not the
absence of a plausible mechanism.

---

## 6. Verdict — which of the four hypotheses?

Reconciling against the four candidates posed at the start of this
investigation:

- **(a) A pinpointed code regression** — not found. Every registry/definition
  file and every identity/link/bundle row on this machine is currently valid;
  the one prior concrete bug of exactly this shape (#2296) is confirmed fixed
  in 0.54.6; the backend startup ordering (`agentmux-srv/src/main.rs:63-143`
  — `bootstrap::open_stores_and_migrate` runs to completion, synchronously,
  before `AGENTMUXSRV-ESTART` is even emitted and before `axum::serve` starts
  listening) **rules out** a startup race between registry-attach and the
  frontend's first RPC — by construction, no client can connect before the
  registry is either attached or not.
- **(b) A data/timing issue specific to fresh channels** — partially true:
  the log **confirms** this was a fresh channel with real first-launch
  bootstrap churn happening in the same few seconds. But I can't extend that
  into "and that's what caused an empty render," because RPC traffic isn't
  logged.
- **(c) Test-methodology artifact** — well-supported by direct evidence: an
  8.65-second session, with active tab/workspace churn 6.5s in, is a strong
  candidate for "closed before (or during) the picker's first successful
  fetch," independent of any bug.
- **(d) A real but narrower issue** — yes, but not the one initially
  suspected (a startup race, ruled out above). The real, narrower, *proven*
  issue is the **single-point-of-failure handler design** (§4): it has
  already caused this precise symptom once, the fix only patched the one
  instance found, and the frontend's error handling still can't distinguish
  "backend threw" from "genuinely empty."

**Honest bottom line:** I cannot pin a definitive, reproduced root cause for
*this specific* report. The most defensible summary is a hybrid of (c) and
(d): the direct evidence (an 8.65s session with mid-flight workspace churn)
best supports a **transient, first-launch-timing artifact** rather than a
standing bug — but the codebase carries a **confirmed, previously-exploited,
still-unfixed structural fragility** (any one bad identity/definition/memory
row anywhere silently zeroes the entire feature, indistinguishably from a
real empty state) that makes this class of incident likely to recur under
slightly different conditions, as it already has at least twice
(2026-06-13's migration-skip bug, 2026-07-25's serde-rename bug).

---

## 7. Found in passing, likely unrelated to this symptom — CORRECTION: already shipped

`git log --oneline --since="2026-07-25"` on the registry/bootstrap area
surfaced `aeb4f024` (`fix(auth): stop isolated boot from rewriting the global
registry, ...`), committed **2026-07-27 13:13** on branch
`agenta/remove-auto-login-trigger`. An earlier draft of this retro used
`git merge-base --is-ancestor aeb4f024 178f7cee` and concluded the fix was
**not** merged — that check is invalid here: `4db519b7` (the PR #2318 merge
commit) has a **single parent** (`git log 4db519b7 -1 --format=%P` →
`128633ce...`), meaning PR #2318 was **squash-merged**. A squash merge
produces a brand-new commit disconnected from the original branch's commit
graph, so none of that branch's original hashes (`aeb4f024` included) will
ever show up as an "ancestor" of main, regardless of whether their content
shipped.

**Corrected, directly verified**: `git show origin/main:agentmux-srv/src/test_support.rs`
(a file `aeb4f024` introduced) and `git show
origin/main:agentmux-srv/src/migrations/m0018_ambient_login_registry.rs`
(grepped for `isolated_auth_enabled`, found at line 50) both confirm **the
actual fix is present on `main`**, and therefore in the reported 0.54.6 build.
Recommendation #4 below ("merge aeb4f024") is **stale — no action needed**;
left in the numbered list struck through rather than silently deleted, since
the reasoning-error itself (trusting `--is-ancestor` across a squash merge) is
worth keeping visible for whoever reads this next.

The bug `aeb4f024` fixes, for context (already shipped, not a live risk):
`m0018_ambient_login_registry.rs` (before the fix) read `ctx.shared_store_path`
to compute which agents are oauth-linked, but under `AGENTMUX_ISOLATED_AUTH=1`
(this session's own opt-in dev-testing flag from PR #2318) that path points at
a fresh, empty, per-instance store — and then unconditionally wrote its
conclusion into the REAL global cross-channel definitions registry. Booting a
single isolated dev-test instance could have flipped every real agent
everywhere to `use_ambient_login=1`. This is very unlikely to explain the
reported "My Agents" symptom regardless (it requires `AGENTMUX_ISOLATED_AUTH=1`
to be set, which a normal `task package` build never sets; and even when
triggered it only flips a boolean on already-valid definitions, it doesn't drop
rows or invalidate JSON) — but it's now moot either way since the fix shipped.

---

## 8. What's confirmed vs. inferred (summary)

**Confirmed (code + logs + direct data inspection):**
- Registry/definition files: 100% schema-valid (24 + 23 files).
- Identity accounts / links / bundles: 100% parseable against current code
  (2 + 1 + 4 rows).
- `#2296`'s specific bug is fixed and present in 0.54.6; not currently live.
- The reported session was a genuine first-ever channel boot; all four
  cross-channel stores attached successfully in it; it underwent bootstrap
  workspace/tab churn ~6.5s after WS connect; it lasted ~8.65s total.
- No startup race is possible between registry-attach and the frontend's
  first RPC (backend ordering is fully synchronous and precedes the readiness
  signal).
- `session.rs`'s handler is structurally all-or-nothing across 6 data
  sources; the frontend cannot distinguish "RPC failed" from "genuinely
  empty" in its rendered UI (only in a console log, since #2296).
- `aeb4f024` (registry-corruption-under-isolation fix) exists, is real, and
  **is** in the 0.54.6 build the user tested (verified directly against
  `origin/main`'s file content, not commit ancestry — PR #2318 was
  squash-merged, so `--is-ancestor` checks against its original commits are
  invalid; see §7's correction).

**Inferred / not proven:**
- That the workspace/tab churn in the user's session actually interrupted
  the `MyAgentsList` fetch (plausible mechanism, no direct log evidence).
- That the user's impression of "empty" reflects a fully-settled
  `EMPTY_GLOBAL` render rather than a loading state glimpsed during a very
  short session.
- Whether repeating this exact build/launch today would reproduce anything —
  not attempted (this investigation, like the one before it, had no clean way
  to launch a genuinely fresh channel without an ambient-env leak; see §2).

---

## 9. Recommendations (not implemented — diagnostic retro only)

1. **Harden `COMMAND_LIST_RECENT_SESSIONS` to degrade per-source, not
   all-or-nothing.** `session.rs:206/215/218/231` — on error from
   `agent_def_list()`, `agent_identity_list_all()`, `identity_list()`, or
   `bundle_memory_list()`, log and substitute an empty map/vec (accepting
   degraded display: "(missing definition)"/"(ambient creds)"/"(vanilla
   CLI)" — all of which already exist as fallback strings for the
   *not-found* case, lines 248-265) instead of aborting the whole list.
   Given this exact failure mode has now caused this reported symptom at
   least once for a confirmed reason, and the structure hasn't changed,
   this is the highest-leverage fix.
2. **`MyAgentsList.tsx`: give the UI a way to distinguish "confirmed zero
   agents" from "fetch failed."** Currently both hit `EMPTY_GLOBAL`
   (`MyAgentsList.tsx:72-74`, `258-267`). At minimum, catch and surface a
   distinct error state/retry affordance rather than only logging to
   console (`MyAgentsList.tsx:128-140`).
3. **Add RPC-command-level tracing** (command name, success/failure, latency)
   at INFO for this handler at least — this investigation's biggest single
   evidentiary gap was that the user's own log couldn't show whether
   `listrecentsessions` ran, succeeded, or failed.
4. ~~Merge `aeb4f024`~~ — **stale, no action needed.** An earlier draft of
   this retro (§7) mistakenly concluded this fix hadn't shipped, based on
   `git merge-base --is-ancestor` across a squash-merged PR — invalid check,
   corrected in §7. The fix is already on `main` and in 0.54.6.
5. **Add an automated e2e regression test**: seed a global registry +
   definitions store with N valid agents, boot a *fresh* channel against
   them, assert "My Agents" renders N rows. This exact path (fresh channel +
   pre-populated global registry) has now broken three separate times
   (2026-06-13 migration-skip, 2026-06-16 snapshot-anchoring, 2026-07-25
   serde-rename) without ever having its own regression test.

---

## 10. Related docs

- `docs/retro/retro-local-build-isolation-regression-2026-06-09.md`
- `docs/retro/retro-per-build-launch-isolation-2026-06-13.md`
- `docs/analysis/ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13.md`
- `docs/retro/retro-legacy-agent-history-cross-channel-2026-06-16.md`
- `docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md`
- `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`
- `docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`
- `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` (§7's `aeb4f024`
  context)
- PR #2296 (`cb8c9eeb`) — the direct prior instance of this exact symptom.
- PR #2319 (`128633ce`) — same-day, same-build fix for a different
  fresh-channel-only timing bug (window title), cited in §5 as pattern
  corroboration, not as the cause.

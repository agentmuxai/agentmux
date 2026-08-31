# Report — AgentPicker "My Agents" audit: field order, missing sort, and two data-fallback gaps

**Date:** 2026-08-24
**Author:** AgentY
**Type:** Investigation + UX/architecture recommendations — fully implemented
as of 2026-08-31.
**Scope:** `frontend/app/view/agent/components/AgentPicker.tsx`,
`AgentPickerFilterBar.tsx`, `MyAgentsList.tsx`,
`frontend/app/view/agent/styles/_recent-sessions.scss`,
`agentmux-srv/src/server/agent_handlers/session.rs`

**Status update (2026-08-24, later same day):** §2 (field reorder), §3
(sort control), and §4 (distinct account-failure text) implemented —
see the PR that shipped this report's own recommendations. §5a (the
Haiku on-demand summary fallback) shipped earlier, separately, as PR #2786.

**Correction (2026-08-31):** this status line previously also claimed "§5
… shipped … PR #2786," which was inaccurate — PR #2786 shipped §5a only.
§5's own two bullets (the `filestore.stat()` `Err`/`Ok(None)` conflation,
and the cross-channel "no conversation snapshot" copy) were never actually
implemented despite the claim, and shipped separately just now: the stat()
error case sets a new `snapshot_check_failed` field (logged + tracked via
the existing `degraded` mechanism, reusing §4's precedent of giving a real
source failure its own distinct fallback text instead of reusing the
genuinely-empty case's copy), and a cross-channel row (`block_id_hint ===
""`) now reads "(history may exist in another version)" instead of the
flatly inaccurate "(no conversation snapshot)". See
`frontend/app/view/agent/components/MyAgentsList.tsx`'s `noSnapshotText`
and `agentmux-srv/src/server/agent_handlers/session.rs`'s
`snapshot_check_failed` handling.

Prompted by direct user feedback: "a lot of the fields are out of order,"
a request for name/launch-date/type sort controls at the right edge of the
filter bar, and two specific data gaps — "(ambient creds)" and "(no
conversation snapshot)" appearing where they shouldn't.

---

## 1. Current state — row layout

Each row in the "My Agents" list (`MyAgentsList.tsx:453-547`) renders, top
to bottom:

1. **Line 1** (`agent-recent-sessions-line1`, one flex row, baseline-aligned):
   provider icon → **name** (`max-width: 60%`, ellipsis) → active-badge (dot,
   if open elsewhere) → runtime badge ("HOST"/"SANDBOX" tag, if known) →
   **account/identity name** (`agent-recent-sessions-meta`, `flex: 1`, own
   ellipsis, fallback `"(ambient creds)"`).
2. **Preview line**: first user message, or `"(no user message yet)"` /
   `"(no conversation snapshot)"`.
3. **Line 3** (conditional): `"N messages"`.
4. **Timestamps row**: `Created` → `Last Launch` → `Last Active` (each
   conditionally shown; `Last Active` only when it's later than `Last
   Launch`).

No sort control exists anywhere in the picker today — `AgentPickerFilterBar.tsx`
is a single text input + clear button (lines 38-63), nothing else. Rows come
back from `ListRecentSessionsCommand` in whatever order the backend returns
them (not verified server-side ordering in this pass — worth confirming
before shipping a sort UI, since "sort" needs a defined base case to toggle
away from).

## 2. Field-order findings

**The account/identity field is the most likely source of "out of order."**
It shares one flex row with the name via `flex: 1` (`_recent-sessions.scss:166-174`),
so it's squeezed into whatever space the name, active-badge, and runtime
badge don't take — on a narrow tile (`minmax(240px, 1fr)` grid,
`_recent-sessions.scss:98`) this reads as an afterthought crammed at the
end of a crowded line, competing for space with the name's own ellipsis
rather than being reliably visible. It's also the ONE piece of
account-identifying info on the card, arguably as important as the name
itself for a user managing multiple accounts — currently it has the least
reliable visual space of anything on the row.

**Timestamp order is chronological (Created → Launch → Active), not
recency-first.** That's an internally consistent choice, but it means the
*most actionable* fact — when was this last active — is buried last,
after two less-actionable facts. Most comparable "recent items" UIs (VS
Code's Recent, GitHub's repo list, browser history) lead with the most
recent/relevant timestamp, not the oldest.

**Recommendation (design judgment, not a bug fix):**
- Move the account/identity name off the crowded name row entirely — pair
  it with the runtime badge on its own metadata line (or immediately
  under the name), consistently visible instead of flex-squeezed.
- Reorder timestamps to lead with whichever is most relevant right now:
  `Last Active` (or `Last Launch` if never active) first, `Created` last —
  the "when did I last touch this" question a picker exists to answer,
  answered first.
- Keep the runtime badge directly adjacent to the name (current position)
  — that pairing already reads well and matches the composer-strip's own
  badge placement convention.

This is a call worth confirming with the user before implementing — listed
as a recommendation, not committed to code in this pass.

## 3. Sort control — recommendation

No sort UI exists to extend, so this is new surface, not a fix. Best-practice
shape for "far right of the filter bar," matching this component's own
minimal, flat, bordered styling (`AgentPickerFilterBar.tsx`/`_picker.scss`):

- A single **"Sort ▾"** control (button + small dropdown, or a 3-way segmented
  toggle if space allows) right-aligned in `.agent-picker-filter-bar`, after
  the text input.
- Options: **Name (A→Z)**, **Recently launched** (default — matches today's
  implicit "most relevant first" intent, needs confirming what the backend's
  current unsorted order actually is), **Type** (groups Host vs. Sandbox —
  matches `RuntimeBadge`'s existing "host"/"container" vocabulary exactly,
  so the sort labels should read "Host" / "Sandbox" to match the tag wording
  the user already sees on each row, per `RuntimeBadge.tsx:28`).
- Persist the choice (this machine only, e.g. `localStorage`) — no existing
  precedent for a control exactly like this in the codebase to match against,
  so this would be a new small pattern, not an extension of one.
- Sorting should apply to the **already-fetched, filtered row set**
  (`filteredRows()` in `MyAgentsList.tsx:381-386`) client-side — no backend
  RPC change needed, since the full candidate set (up to `SEARCH_LIMIT` while
  filtering, `limit ?? 20` otherwise) is already in memory.

Not designed further than this (control placement + options) — full visual
spec/implementation is follow-up work, not part of this audit.

## 4. Bug: "(ambient creds)" can appear on agents that DO have a bound account

**Real, confirmed regression class — not by-design for the failure case.**

`identity_name` is populated in `session.rs:330-346` by looking up
`links_by_agent[definition_id]`, built from `identity_store.agent_identity_list_all()`.
When no link rows exist for a definition, the code hard-codes `"(ambient
creds)"` — correct when the agent genuinely has no bound identity.

**The bug:** if `agent_identity_list_all()` itself errors (`session.rs:282-290`
— the doc comment cites the exact `secret_ref` serde-tag mismatch behind
PR #2296), `links_by_agent` comes back empty for the WHOLE response, so
**every** row shows `"(ambient creds)"`, including agents with a real,
correctly-bound account. This exact failure mode is already documented in
`docs/retro/retro-my-agents-fresh-channel-regression-2026-07-27.md` §9 rec 1
— previously caused the whole list to look empty; the current code degrades
that source in isolation instead of aborting the request (an improvement),
but the wrong-text symptom on every row was never itself eliminated.

Secondary, narrower gap: the identity lookup joins by `agent_id` only, not
`provider` (`session.rs:330-344`), so a multi-provider agent could show a
stitched `"nameA, nameB"` instead of the identity actually used by that
specific instance — a display-accuracy issue, not the empty-string bug.

**Recommendation:** give the `identity_links`-source-failure case its own
distinct fallback text (e.g. `"(unknown account)"` or similar), instead of
reusing the same string genuinely-no-identity uses — mirroring the exact
pattern this codebase already applies one layer up (`FETCH_ERROR` vs.
`EMPTY_GLOBAL`/`EMPTY_FILTERED` in `MyAgentsList.tsx:63-71`, built
specifically so a backend failure never looks identical to a legitimate
empty state). Also worth a loud log line at the failure site if one isn't
already there (not confirmed in this pass).

## 5. Bug: "(no conversation snapshot)" can appear on agents that DO have history

Two distinct causes, one a real bug, one by-design-but-confusing:

**Real bug:** `has_snapshot` is computed from `filestore.stat(&inst.block_id,
"output.state.json")` (`session.rs:360-379`), which returns
`Result<Option<WaveFile>, StoreError>`. The match arm collapses `Err(...)`
(I/O error, lock contention, DB read failure) into the SAME branch as
`Ok(None)` (genuinely no snapshot) — a transient storage error on a row that
DOES have a real snapshot renders identically to "never had one," with no
log line. This is unconditional error-swallowing, and it isn't covered by
the `degraded` mechanism at all (filestore isn't one of the six tracked
sources).

**By-design, but the copy doesn't say so:** rows sourced from the
cross-channel registry (no matching local SQLite instance) get a synthetic
`AgentInstance` with `block_id: String::new()` (`session.rs:219-235`,
explicit comment: "Cross-channel rows arrive as synthetic instances"). An
empty `block_id` short-circuits to `has_snapshot: false` before `stat()` is
ever called — correct given this instance's own filestore genuinely has
nothing, but the row's real conversation may well exist in a *different*
channel's filestore. `"(no conversation snapshot)"` reads as "there is no
history," when the more accurate statement is "no history in THIS channel."

**Implemented (2026-08-31):**
- The error-swallowing is fixed: the match arm now distinguishes `Err` from
  `Ok(None)`, logs the error case (`tracing::warn!`), and marks the new
  `"snapshot_stat"` degraded source. The row carries a new
  `snapshot_check_failed: bool` field so the frontend can render a third,
  distinct state ("(couldn't check for history)") instead of silently
  reporting "no snapshot."
- The cross-channel case now reads "(history may exist in another version)"
  — exactly the wording proposed here — driven by the existing
  `block_id_hint === ""` signal the row already carried, no backend change
  needed for this half.

### 5a. Addendum — a real fallback source exists (correction to an earlier draft of this report)

An earlier pass of this investigation checked the **swarm** dispatch system
(`AgentDispatch`/`dispatch_name`) as a possible fallback preview source and
correctly ruled it out — it's live/in-memory only, keyed by `parent_block_id`/
a live request's `agent_id`, not `definition_id`, and dies with the block.

The user then pointed at the right thing: dispatch naming and terminal
`term:ambient_summary` generation aren't two independent Haiku features —
**they share one call site.** Confirmed: `generate_dispatch_name`,
`generate_subagent_name`, `register_session_activity_summary`, and
`generate_pushed_activity_summary` (`session.rs:156,215,308,374`) all funnel
through `invoke_ambient_haiku_call()` (`session.rs:670`) via the same
coalescing gateway, `crate::ambient::gateway()` (`agentmux-srv/src/ambient/mod.rs:61`),
keyed by `AmbientCallKey{entity_id, purpose}` — a deliberately reusable
choke point per `docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`
and `docs/specs/SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md:89-91`
("any future ambient-call purpose gets the same protection for free").

`invoke_ambient_haiku_call(cli_path, prompt, meta, cancel)` takes an
arbitrary caller-built prompt (not a fixed template) and returns raw Haiku
text — nothing about it is scoped to dispatches or terminals specifically.
**This makes a definition-keyed "last activity" summary genuinely buildable**,
closing the gap this report originally flagged as unbridgeable — with two
things still needed, neither of which exists today:

1. **A new call site** in `session.rs`, shaped like `generate_dispatch_name`
   (`session.rs:308-372`): build a prompt from the instance's latest
   `output.state.json`/recent output (reusing the same extraction helpers
   `read_task_prompt()`/`read_recent_activity_digest()` already use), call
   `invoke_ambient_haiku_call` through `ambient::gateway().admit(AmbientCallKey::new(definition_id, "definition_summary"), ...)`.
2. **New persistence** — the actual gap, not the Haiku call itself. Nothing
   today stores anything keyed by `definition_id` for this purpose (dispatch
   names live in in-process memory; `term:ambient_summary` lives in ephemeral
   block meta keyed by `block_id`). This needs a durable, `definition_id`-keyed
   store (a new SQLite table, or a field alongside `registry/def_store.rs`)
   holding `(definition_id, summary, updated_at)`, written by the new call
   site and read back in `session.rs:381-409` as the `preview` fallback
   whenever `has_snapshot` is false.

**Open design question, not yet resolved:** what triggers generation/refresh
— every turn-end (mirrors the existing `activity_summary` pushed-sweep
trigger, but multiplies Haiku spend across every agent definition, not just
open panes), only when a pane closes (cheaper, but stale until the agent is
reopened elsewhere), or on-demand the first time the picker actually needs
one for a snapshot-less row (cheapest, but adds latency to that row's first
render)? This is a cost/freshness tradeoff, not a technical constraint —
worth deciding before implementing.

## 6. Non-goals / not investigated this pass

- Server-side default row ordering (needed to pick a sensible sort default)
  — not confirmed.
- Whether `filestore.stat()`'s error rate in production is actually
  meaningful (no telemetry reviewed) — the fix is justified by the
  swallowing pattern itself (same class of bug the `identity_links`
  precedent already proved worth fixing), not by observed frequency.
- Visual mockup / exact CSS for the reordered row or the new sort control.
- Any change to `ListRecentSessionsCommand`'s RPC contract.

## 7. Suggested next steps, in order

All five shipped as of 2026-08-31.

1. ~~Confirm the proposed field-order change and sort-control placement with
   the user before implementing~~ — confirmed, implemented per §2/§3.
2. ~~Fix `has_snapshot`'s `Err`/`Ok(None)` conflation~~ — implemented per §5.
3. ~~Give the `identity_links`-failure case distinct fallback text~~ —
   implemented per §4.
4. ~~Implement the sort control~~ — implemented per §3.
5. ~~Reorder row fields per §2~~ — implemented.

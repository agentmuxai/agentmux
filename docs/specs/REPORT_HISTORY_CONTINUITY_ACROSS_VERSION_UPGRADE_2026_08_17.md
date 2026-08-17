# Conversation Continuity Across a Version/Channel Switch — Verification Report

**Date:** 2026-08-17
**Author:** AgentY (agent, `~/.agentmux/agents/agenty-0629j`), at operator request
**Status:** Report — independently re-verified against live `origin/main` code. No code changes in this doc.
**Ground truth basis:** `agentmuxai/agentmux` `origin/main`. Every citation below is a real file:line, checked
directly (not taken from a doc's word alone).
**Related (read in full before writing this):**
`docs/specs/REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md`,
`docs/specs/SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`,
`docs/specs/CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md`,
issue #2603 (closed, "COMPLETE", 6 steps: #2602, #2605, #2606, #2611, #2613, #2614).
**Motivation:** the operator asked, after closing an AgentY session and reopening it from "My Agents" in a
newer build, whether continuity is now solved given the recent (closed) work — since it's still resetting or
the conversation can't be found.

---

## 0. Verdict

**Not solved.** Issue #2603's six shipped steps fixed a real, serious bug (conversation *history* becoming
unreachable/fragmented once written), but they don't cover the operator's actual scenario. The live blocker on
a version/channel switch is one step earlier and more severe than "history is hard to find": **the launch is
refused outright** with a "no credentials — bind an account in the Armory" error, because the row that says
*which account this agent uses* lives in a per-channel-isolated database by default on every non-`"stable"`
channel (i.e. every local/dev/portable build — the exact case a version bump produces). History reachability
is moot because the spawn never gets far enough to read it.

The design docs anticipated a version of this (protocol spec's §4 "step 3": reuse the Mandatory-ABF
`agent_id → default_memory_id` binding so a fresh channel doesn't mint an unlinked identity) but that step was
never implemented as its own PR, and — more importantly — implementing it alone would not have fixed the
actual mechanism found here, which is one level below the bundle-id-freshness question: it's about which
*database* the binding row is even looked up in.

---

## 1. What issue #2603 actually fixed (confirmed shipped, still true)

| PR | What it did |
|---|---|
| #2602 | Fixed stale migration-phase doc comments; pinned the real `db_agents` read-split with a test. |
| #2605 | Split CREDENTIAL isolation from CONVERSATION HISTORY storage: `identity_history_dir(bundle_id, provider, subdir)` (`agentmux-common/src/data_paths.rs:405`) always resolves under the global `shared_dir`, regardless of `isolated_auth_enabled()`. |
| #2606 | `ClaudeHistoryAdapter` now also scans per-channel identity dirs when reading, closing the "written after 2026-08-06, invisible to the UI" gap. |
| #2611 | Fast `agent_id`-keyed history lookup index, instead of a full filesystem walk. |
| #2613 | Opt-in `bundle.export_for_agent_with_history` action, separate from the default portable-bundle export. |
| #2614 | Distilled the above into `CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md` for future PRs in this area. |

These are real fixes for a real problem — **if** a spawn reaches the point of writing/reading transcript
files, those transcripts now land in (and can be found in) one durable, global location keyed by `bundle_id`,
linked via a Windows junction / Unix symlink (`ensure_history_link`, `data_paths.rs:483`) so isolation of the
surrounding credential directory doesn't take the transcripts with it. This part of the pipeline is sound.

The gap is that **for the operator's scenario, the spawn never reaches that point at all.**

---

## 2. The actual mechanism that breaks a version/channel switch

### 2.1 One flag, two independent consumers

`isolated_auth_reason()` (`agentmux-common/src/data_paths.rs`, ~line 662) resolves to
`ChannelDefaultIsolated` whenever `AGENTMUX_CHANNEL != "stable"` — true for every local/dev/portable build,
per `scripts/package.sh`'s per-build channel minting (`CHANNEL="local-${BRANCH_SLUG}-${BRANCH_HASH}-${BUILD_ID}"`,
confirmed intentional and unrelated to this bug — see the companion report §1.1). This one flag drives two
things that are conceptually different but were never split apart the way CREDENTIAL vs CONVERSATION HISTORY
was in #2605:

1. `identities_dir()` — per-channel credential directories (the already-known, already-partially-fixed piece).
2. **`registry::resolve_shared_store_path()`** (`agentmux-srv/src/registry/paths.rs:63-72`) — when isolated,
   returns `<instance_dir>/identity-store.db` instead of the global `~/.agentmux/shared/store.db`.

`bootstrap.rs:656-699` opens whichever path #2 resolves to as `shared_store`, and only promotes it to the
app's `id_store` (the handle every "must survive across versions" read/write is supposed to go through, per
`server/mod.rs`'s own doc comment) if that store already has the `0011_shared_store_backfill` migration
applied. On a **fresh channel**, `identity-store.db` doesn't exist yet: it gets created empty and migrated
fresh, which trivially satisfies "backfill applied" against zero rows. Either way, `id_store` ends up pointing
at a per-channel file. **`db_agent_identity_links` and `db_accounts` — the tables that record "this agent uses
this OAuth account" — live in that per-channel-by-default file**, even though `migrations.rs:884-889`'s own
doc comment for that schema says this data "must survive across channels/versions."

### 2.2 What actually happens on relaunch (the real spawn path, not account-creation)

The relevant function for reattaching to an existing named agent is `inject_identity_env_with_broker`
(`agentmux-srv/src/identity/resolver/inject.rs:231`) — **not** `compute_and_ensure_account_dir`, which is only
called from the `auth.start` (explicit "connect an account") handlers
(`identity_handlers.rs:211,347`) and is irrelevant to an ordinary relaunch.

1. `resolve_bindings_for_instance` → `id_store.agent_identity_list_for_agent(&instance.definition_id)`
   (`inject.rs:166-167`; query at `identities.rs:530-553`). On a fresh channel this returns **empty** — not
   because the binding doesn't logically exist, but because it's asking the wrong (per-channel, brand-new)
   database.
2. The provider-gate check (`inject.rs:594-606`) sees no binding and calls `gate_oauth_failure`, returning
   `SpawnGateError::MissingCredentials` (`inject.rs:605`) — **the spawn is refused**, with the message pinned
   at `inject.rs:1094-1096` ("no credentials for `<provider>` ... bind an account for this provider in the
   Armory"). There used to be an `use_ambient_login` fallback that could have papered over exactly this case;
   it was explicitly retired (comment at `inject.rs:355-365`), so there is no rescue path left.
3. `link_history_if_isolated` (`inject.rs:509-521`, the mechanism #2605 added, confirmed running on every
   spawn now — not just account-creation) is **never reached**, because step 2 already aborted. The
   always-global `identity_history_dir()` that #2605-#2611 made reachable and fast to search is moot here; the
   spawn dies before asking about history at all.

Separately, the **agent definition itself** (`db_agent_definitions`/`db_agents`) is deliberately made global
via `registry::resolve_shared_definitions_dir()` — unconditional, not gated by `isolated_auth_enabled()` at all
(`bootstrap.rs:595-614`). This is why the agent still **appears** in "My Agents" after a version bump — the
definition survives. It's specifically the *account binding* that doesn't.

### 2.3 What the operator actually sees

- The agent shows up in "My Agents" in the new build (definitions are global — this part works).
- Clicking it fails the credential gate and refuses to spawn, surfacing "no credentials for this provider —
  bind an account in the Armory" (or, if the AgentAuthPanel path is reached instead of an outright block in
  some code paths, a login prompt) rather than continuing the prior conversation.
- The user re-connects the account. This mints a fresh binding in the new channel's own store — a *new*
  `bundle_id`/`account_id`, disconnected from the old one, which is exactly the "always resetting" symptom:
  even after re-login, the new bundle_id's `identity_history_dir()` is empty. The old transcripts are real and
  intact on disk (per #2605's global storage), but nothing in the fresh session's own account binding points
  at them — which is the "can't find the conversation" half of the complaint. Locating them today requires the
  same kind of manual cross-channel search the original 2026-08-16 report describes, or (once step 3 below
  ships) an agent_id-anchored lookup that doesn't yet exist.

---

## 3. Why the design docs' own recommended fix wouldn't have been sufficient either

`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md` §4 "step 3" proposed wiring `agent_id` to a
stable `bundle_id` via the Mandatory-ABF `default_memory_id` binding, so a fresh channel wouldn't mint an
unlinked identity. That step was never implemented (`db_agents.default_memory_id`/`memory_id` is confirmed,
independently, to be exclusively the ABF **portable-config** bundle — CLAUDE.md/skills/MCP/model — an
unrelated "bundle" concept from the OAuth `account_id`; `migrations.rs` v19, `agents.rs:81,121`). But even had
it shipped, it would only have addressed "which `bundle_id` does this agent get" — it would not by itself have
fixed §2.1's deeper issue, which is that `db_agent_identity_links`/`db_accounts` are looked up through
`id_store`, and `id_store`'s own routing (`resolve_shared_store_path`) still defaults to a per-channel file on
every non-stable build. A stable `bundle_id` recorded in a per-channel database that resets every version bump
doesn't help — the lookup that would resolve it never reaches a database that has the row.

---

## 4. Recommended fix, in the same shape as the #2605 split

Apply the exact pattern that already fixed CONVERSATION HISTORY to this one level up the stack:

1. **Split identity-*link* routing from identity-*credential* routing**, the same way #2605 split
   history-*content* routing from credential routing. `db_agent_identity_links`/`db_accounts` (the *pointer*
   — "agent X uses account Y") is arguably closer to DEFINITION scope (P1's taxonomy) than to CREDENTIAL scope
   — it names a relationship, not a secret. The actual OAuth token/PAT material can stay isolated per-channel
   for Armory testing exactly as `identities_dir()` already does; the *fact that a binding exists* should not.
   Concretely: `resolve_shared_store_path()` (or a new accessor) should resolve the identity-link tables'
   backing store globally regardless of `isolated_auth_enabled()`, mirroring `identity_history_dir()`'s
   unconditional resolution.
2. **Then** (not before) implement the deferred protocol step 3 — anchor `agent_id` to a durable `bundle_id`
   via the ABF-adjacent global binding — so that once the lookup in (1) can actually find a row, it finds the
   *same* `bundle_id` every time, keeping `identity_history_dir()` pointed at the same transcripts across every
   future version bump too.
3. **Regression test, per the checklist's own item 4** ("isolating credentials must not silently isolate
   [something else]"): a test asserting `db_agent_identity_links`/`db_accounts` reads/writes never vary by
   channel, mirroring the CREDENTIAL-vs-HISTORY test #2605 already added for `identities_dir()` vs
   `identity_history_dir()`. This is the same recommendation (P3) already made three times in this doc family
   for other scopes — extend it to this one explicitly, since it's the one still missing.

## 5. Immediate mitigation (before a real fix ships)

Same shape as the companion report's §4.6: explicitly set `AGENTMUX_ISOLATED_AUTH=0` in the launch environment
for identity-bound "named continuing agent" launches specifically (as opposed to disposable Armory test
agents), so they keep resolving `id_store` to the global store until the routing split in §4 lands. This does
not require touching the credential-isolation code path at all — it only needs to exempt continuing-agent
launches from the default that currently applies to every non-stable channel uniformly.

## 6. Open questions

- Is there a reason `db_agent_identity_links`/`db_accounts` were bundled into the same per-channel store as
  raw credential material in the first place, or is this simply the same "shares a directory/store, never
  reconciled" pattern the companion report's §2 table already documents four other times? (My reading of
  `resolve_shared_store_path()` and its callers found no scoping rationale specific to the link tables — they
  appear to have inherited the isolation incidentally, the same way `claude/projects/` did.)
- Should the fix in §4.1 land as a narrow, standalone hotfix (fastest path to unblocking the operator's actual
  symptom) or be sequenced as part of a restarted identity-persistence protocol effort (issue #2603's
  successor)? Given this is actively blocking a real, reported workflow today, the companion report's own
  precedent (§4.6, "fast-follow as a standalone hotfix") argues for the former.

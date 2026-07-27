# Cross-Branch Agent-Definition Gap — Why "Existing" Agents Fail Auth on a Fresh Dev Database

**Date:** 2026-07-27
**Author:** AgentA
**Status:** Report — root-cause investigation complete for the confirmed chain; one mechanism (how a
non-existent agent definition still appears launchable in the UI) remains an open question, flagged
explicitly in §5. No code changes in this PR.
**Ground truth basis:** `agentmuxai/agentmux` `main` at commit `38978e6b`, live investigation against a
running `task dev` instance on the `main` branch (`v0.54.5`, data dir
`C:\Users\area54\.agentmux\dev\main\bdbef7b72912a6f3\data\db`).
**Motivation:** manual end-to-end auth testing (post-merge of PR #2300/#2303/#2304) surfaced two symptoms —
an existing agent's browser opens immediately on mount with no click, and a completed login produces no
success notification. Reproduced with two different pre-existing agents ("Oozp", then "Parko") on the same
dev instance. This report traces both to a single, deeper cause that is **not** a regression in this
session's auth-notification work.

---

## 0. Executive summary

Both reported symptoms trace to the same root: **`task dev <branch>` keeps a separate, local SQLite
database per branch, and an agent created/used on one branch's database is invisible to every other
branch's database at the schema level** — even though its on-disk config folder is shared/global. When the
user opened "Oozp" then "Parko" (two different agents, both previously used successfully elsewhere) on this
session's `main`-branch dev instance, two database writes failed identically for both:

```
agent instance row create failed: createagentinstance: sqlite error: FOREIGN KEY constraint failed
direct identity link write-through failed: linkagentidentity: sqlite error: FOREIGN KEY constraint failed
```

Both failures are the same foreign key: `db_agent_instances.definition_id` and the identity-link table both
require a matching row in `db_agent_definitions(id)`. Querying this specific dev instance's database directly
(after a forced WAL checkpoint, to rule out an uncommitted-write artifact) confirms **neither agent has a row
there** — `db_agent_definitions` contains exactly 7 rows, all built-in providers (claude/codex/gemini/kimi/
pi/openclaw/copilot), none of them named "Oozp" or "Parko". An exhaustive search across every other database
file on this machine (every `task dev` branch, every per-build local channel, the `stable` channel, every
pre-migration backup and snapshot — over 100 files) found no row for either agent's ID anywhere. This is not
corruption of one specific agent's row; it is the complete absence of ever having created one, in any
database this machine currently holds.

**This fully explains the second symptom (no success notification) as a correct, not-broken response to bad
upstream state** — not a bug in the notification code this session built. It does not yet fully explain the
first symptom's exact UI mechanics (§5's open question), though the practical effect (an automatic login
attempt on a definition-less agent) is consistent with "this agent looks new to this database."

---

## 1. The failure chain, traced end to end

### 1.1 What happens on mount, concretely

Opening "Oozp" (a never-before-used-here agent, confirmed by its own log line
`"CLI not found locally, installing via npm"`) produces, in order:

1. `[fe] [agent] Launching agent definition Oozp (claude)` — the frontend already has an `AgentDefinition`
   object with a real UUID (`df2ea4f1-ca5c-47f4-aa3c-7638e877beeb`) before any backend call in this sequence.
2. `agent instance row create failed: createagentinstance: sqlite error: FOREIGN KEY constraint failed`
3. `direct identity link write-through failed: linkagentidentity: sqlite error: FOREIGN KEY constraint failed`
4. CLI install proceeds anyway (npm install succeeds) — the launch flow doesn't hard-abort on either failure.
5. `seed_provider_auth_from_global: seeded isolated dir from valid global login` — the credential seed
   **succeeds**, writing to the correct isolated dir
   (`shared\identities\669f62de-...\claude\.credentials.json`).
6. `open_login_terminal: spawning new console` — the flow falls through to opening a real terminal anyway
   (see §1.2 for why, despite step 5's apparent success).
7. `claude auth check: no credentials in provider dir, skipping CLI` — the auth-check that's supposed to
   confirm the login lands, but reports failure.
8. `cancel_cli_login` — the attempt concludes.

Repeating the exact same sequence with "Parko" (a different, also previously-used agent, ID
`b87a0ab3-35ef-4087-a8ff-724d3135e2ef`) at `2026-07-27T03:13:56` produced the identical two FK errors in the
identical order — confirmed systemic, not a one-agent data-corruption artifact.

### 1.2 Why step 5's success doesn't prevent step 6

`frontend/app/view/agent/flows/run-provider-login.ts`'s tier-2 path (`seedGlobalLogin`, called before
opening any terminal) does more than copy a file — on success it calls `persistSeededAccount` (with one
retry) to register the account in the database. **That registration needs the same
`db_agent_definitions`/identity-link rows that steps 2-3 already failed to create.** So even though the
credential genuinely lands on disk correctly, `persistSeededAccount` fails for the same underlying reason,
and the code — correctly, per its own documented fallback design — treats "seed succeeded but couldn't be
persisted" as not fully successful, and falls through to opening a terminal as tier 3's fallback.

### 1.3 Why the eventual auth-check reports "no credentials" despite a real seed

`onAccountRegistered` (the callback that updates `launch-flow.ts`'s `recheckAuthEnv` to point at the
newly-minted isolated directory before the post-login verification check) is only invoked **after a
successful `persistSeededAccount` call** — see `run-provider-login.ts:328-329` (tier 2) and `:387-390`
(tier 3, after the terminal-fallback poll). Since that persist call fails for the same FK reason as steps
2-3, `onAccountRegistered` never fires, `recheckAuthEnv` never gets updated away from its stale/default
value, and the final `CheckCliAuthCommand` recheck in `launch-flow.ts` queries the wrong directory —
reporting `authenticated: false` for a login that, per step 5's own log line, actually succeeded on disk.

**This is the direct explanation for "no success notification":** `onLoginSuccess` is only called when
`authenticated === true` (`launch-flow.ts:419-422`) — and correctly so, given the (incorrect, but
consistently propagated) information the verification check is working with. The gap is entirely upstream,
in the database layer, not in anything this session's `postSystemNotification`/`LaunchAuthState` work built.

---

## 2. Confirming this is a database-isolation artifact, not corruption

`agentmux-cef`'s own startup log states the data directory explicitly:

```
Using data_dir: C:\Users\area54\.agentmux\dev\main\bdbef7b72912a6f3\data
```

This matches `CLAUDE.md`'s own documented behavior: *"the dev data dir is keyed on the git branch
(`~/.agentmux/dev/<branch>/`)"* — each `task dev <branch>` invocation gets a fully separate SQLite database,
not a shared one. Cross-referencing against the doc's adjacent claim for `task package` builds — *"agents and
auth are GLOBAL (cross-channel work #1387-#1393): a fresh per-build data dir still shows every agent and
stays logged in"* — that global guarantee is stated specifically for **per-build local channels**
(`task package`), not for `task dev`'s per-branch databases. Nothing in the docs claims (and this
investigation found no evidence) that `task dev` branch databases share agent-definition rows with each
other or with the per-build/stable channels.

What **is** shared globally, confirmed directly: the on-disk config folder
(`~/.agentmux/agents/parko-0617i/`), which for Parko contains only its own git working directory (a personal
clone of this same repo, for its own agent sessions), a `.mcp.json`, and a `.claude/` directory — **no
agent-definition metadata file** (name, provider, icon, etc.) of any kind. That metadata is database-only.

### 2.1 Exhaustive search — the row exists nowhere on this machine

Searched every `objects.db` reachable on this machine for a `db_agent_definitions` or `db_agents` row
matching Parko's ID or name:

- Every `task dev <branch>` database under `~/.agentmux/dev/*/*/data/db/` — no match.
- Every per-build local channel under `~/.agentmux/channels/local-*/data/db/` — no match.
- The `stable` channel's live database and its two archived version snapshots — no match.
- The top-level shared databases (`~/.agentmux/db/objects.db`, `~/.agentmux/objects.db`) — no match (the
  second file doesn't even have the table, suggesting it's a stale/legacy artifact rather than a real
  candidate).
- Every `pre-migration-*` backup and every `*.bak` snapshot under `~/.agentmux/shared/backups/` and
  `~/.agentmux/snapshots/` (over 100 files spanning versions 0.38.4 through 0.54.4) — no match.

Combined with the forced-WAL-checkpoint re-verification on the actual `dev:main` database (§0), this rules
out both "the row exists but this specific query missed it" and "the row exists somewhere and just hasn't
been migrated here yet." As far as this machine's filesystem is concerned, Parko has genuinely never had a
`db_agent_definitions`/`db_agents` row created for it, on any database, at any point.

---

## 3. Schema reference (for whoever implements the fix)

Two tables currently both require a definition row, reflecting an in-progress consolidation
(`agentmux-srv/src/backend/storage/migrations.rs`'s own comments call this "Phase 3a"/"Phase 3b dual-write"):

```sql
-- Legacy (still FK-enforced by db_agent_instances and identity-link tables):
CREATE TABLE db_agent_definitions (
    id TEXT PRIMARY KEY, slug TEXT NOT NULL DEFAULT '', name TEXT NOT NULL,
    icon TEXT NOT NULL DEFAULT '✦', provider TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
    working_directory TEXT NOT NULL DEFAULT '', shell TEXT NOT NULL DEFAULT '',
    provider_flags TEXT NOT NULL DEFAULT '', auto_start INTEGER NOT NULL DEFAULT 0,
    restart_on_crash INTEGER NOT NULL DEFAULT 0, idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
    agent_type TEXT NOT NULL DEFAULT 'standalone', environment TEXT NOT NULL DEFAULT '',
    agent_bus_id TEXT NOT NULL DEFAULT '', is_seeded INTEGER NOT NULL DEFAULT 0,
    accounts TEXT NOT NULL DEFAULT '', parent_id TEXT NOT NULL DEFAULT '',
    branch_label TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0, user_hidden INTEGER NOT NULL DEFAULT 0,
    container_image TEXT NOT NULL DEFAULT '', container_volumes TEXT NOT NULL DEFAULT '[]',
    container_name TEXT NOT NULL DEFAULT ''
    -- (further columns not yet transcribed — read migrations.rs:179+ for the rest)
);

-- Phase 3a consolidated (db_agent_definitions + db_agent_instances collapsed into one row):
CREATE TABLE db_agents (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '', is_template INTEGER NOT NULL DEFAULT 0,
    parent_template_id TEXT NOT NULL DEFAULT '', provider TEXT NOT NULL,
    provider_flags TEXT NOT NULL DEFAULT '', shell TEXT NOT NULL DEFAULT '',
    environment TEXT NOT NULL DEFAULT '', agent_type TEXT NOT NULL DEFAULT 'standalone',
    agent_bus_id TEXT NOT NULL DEFAULT '', accounts TEXT NOT NULL DEFAULT '',
    auto_start INTEGER NOT NULL DEFAULT 0, restart_on_crash INTEGER NOT NULL DEFAULT 0,
    idle_timeout_minutes INTEGER NOT NULL DEFAULT 0, slug TEXT NOT NULL DEFAULT '',
    branch_label TEXT NOT NULL DEFAULT '', identity_id TEXT NOT NULL DEFAULT '',
    memory_id TEXT NOT NULL DEFAULT '', working_directory TEXT NOT NULL DEFAULT '',
    github_context TEXT NOT NULL DEFAULT '', instance_name TEXT NOT NULL DEFAULT '',
    last_block_id TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0, is_seeded INTEGER NOT NULL DEFAULT 0,
    user_hidden INTEGER NOT NULL DEFAULT 0, container_image TEXT NOT NULL DEFAULT '',
    container_volumes TEXT NOT NULL DEFAULT '[]', container_name TEXT NOT NULL DEFAULT ''
    -- (further columns not yet transcribed)
);
```

Every column except `id`/`name`/`provider` has a usable default, and the frontend's own
`launchAgentDefinition` already falls back gracefully when `working_directory` is empty
(`agent-model.ts:377-382`: derives `${agentmuxHome()}/agents/${slug}` when the persisted value is blank) —
so a minimal, schema-correct backfill row (id, name, provider, everything else defaulted) should be
sufficient to unblock the FK chain without needing to reconstruct every original field value.

The RPC command that lists agents for the UI (`listagents` / `COMMAND_LIST_AGENTS`,
`agentmux-srv/src/server/agent_handlers/core.rs:47-58`) calls `wstore.agent_def_list()` — i.e. it reads
`db_agent_definitions` directly from whichever database the connected srv instance has open. This is the
piece that doesn't yet add up (§5).

---

## 4. What this is *not*

- **Not a regression from PR #2300/#2303/#2304.** Nothing in this session's `LaunchAuthState`/notification
  work touches `createagentinstance`, `linkagentidentity`, or either agent-definition table. The `onLoginSuccess`
  gate behaving correctly (refusing to celebrate a login the verification check believes failed) is exactly
  the design intent from that work — it's just being fed bad information by an unrelated upstream failure.
- **Not corruption specific to one agent.** Two different, unrelated agents (Oozp, Parko) hit the identical
  two-error sequence. Whatever the underlying cause, it's systemic to this database, not a one-off bad row.
- **Not fixable by retrying the login.** Every retry re-hits the same missing-FK-target condition; the
  credential seed itself will keep succeeding, and the persist/verify step will keep failing, for as long as
  the definition row is absent.

---

## 5. Open question — how did the UI show these agents as launchable at all?

If `listagents` reads `db_agent_definitions` directly from the connected instance's database, and that table
provably has no row for either agent (§2.1's exhaustive search, plus a forced WAL checkpoint on the live
instance to rule out an uncommitted-write false negative), then the frontend should not have been able to
list — let alone launch — either one from this `dev:main` instance. Two hypotheses, neither yet confirmed by
direct evidence:

1. **Stale frontend cache.** The Solid store backing the agent list may not fully invalidate/re-fetch when
   the frontend reconnects to a different backend instance (e.g., surviving a hot-reload, or a cached
   `WaveObj` from a previous session's connection to a different branch's srv). This would mean the user saw
   a list item that was never actually valid for the currently-connected database, and the launch flow
   proceeded using that stale `AgentDefinition` object's fields without re-validating server-side that the
   definition row exists before starting the launch sequence.
2. **A second, undiscovered read path.** Some other code path might construct an `AgentDefinition`-shaped
   object from a source this investigation didn't find (e.g., directly from `db_agents` under a different
   query than the one checked, or from block/workspace metadata that outlived the agent's own definition
   row). Considered less likely given the exhaustive schema/table search in §2.1, but not ruled out.

This is the one piece worth confirming before implementing a fix — if it's (1), the real product-level bug is
**the frontend not re-validating agent existence against the currently-connected backend**, and any DB
backfill is a workaround for testing, not the actual fix. If it's (2), the backfill approach in §3 is closer
to a complete, permanent fix on its own.

---

## 6. Recommended next steps

1. **Resolve §5 first** — a quick way to test hypothesis (1): fully reload the frontend (hard refresh /
   restart the dev instance from cold, not just re-focus the window) and see whether Oozp/Parko still appear
   in the agent list at all. If they disappear, that confirms stale-cache; if they still appear, the second
   hypothesis needs investigating.
2. **For the immediate practical goal** ("get Parko working, then port the rest," per direct request): insert
   a minimal, schema-correct row into both `db_agent_definitions` and `db_agents` for Parko's known ID/name/
   provider (`b87a0ab3-35ef-4087-a8ff-724d3135e2ef` / "Parko" / "claude") in this `dev:main` database,
   relying on the schema's own defaults and the frontend's graceful working-directory fallback for everything
   else not currently known. Verify the FK chain clears (retry the login end-to-end) before repeating for
   other agents.
3. **Longer-term**, if hypothesis (1) is confirmed: this points at a real, generalizable gap — launching an
   agent should probably re-validate (or lazily create) its definition row against whichever database is
   actually connected, rather than trusting a cached `AgentDefinition` object's mere existence in the
   frontend's local state. That would make "an agent used on one dev branch also just works on another"
   actually true, instead of true only by accident when the databases happen to already agree.

---

## Appendix: research method

Live investigation against a running `task dev` instance (not a static code read alone): reproduced the
reported symptoms with two different agents, read the host's own structured JSON logs line-by-line across
both reproductions to build the exact call sequence in §1, then verified every claim about *absence* (the
missing DB row) by direct `sqlite3` queries — including a forced `PRAGMA wal_checkpoint(FULL)` specifically
to rule out a WAL-visibility false negative before concluding the row doesn't exist — rather than inferring
absence from a single query. The cross-machine database search in §2.1 was exhaustive by construction (every
`objects.db` file discoverable via `find`), not a sample. Source citations for the failure chain (§1.2, §1.3)
are direct reads of the current `run-provider-login.ts`/`launch-flow.ts` implementations, not the app's
runtime behavior alone — both were cross-checked against each other to confirm the log evidence and the code
path agree. §5 is explicitly flagged as unresolved rather than guessed at, since resolving it changes what
"the fix" actually is.

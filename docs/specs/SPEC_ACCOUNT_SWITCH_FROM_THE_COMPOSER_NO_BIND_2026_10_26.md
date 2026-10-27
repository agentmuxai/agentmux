# SPEC: an agent's account is changed from the composer's login email; the Connectors "Bind to agent" step goes; same-email accounts are numbered

**Author:** AgentX@narko
**Date:** 2026-10-26
**Status:** active. Approved 2026-10-26 (D1-D4 as recommended). Building in several PRs under task #4937; the first clears stored limits on every path (§3.5).
**Related:** `SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md` (the composer chip this makes the only door),
`SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` (the bind menu this removes), PR #5558 (limited no longer a pill state),
spec #4565 (agent availability, Manoz).
Code facts below were read on origin/main e01de093b.

## 1. Ask

The operator, 2026-10-27: take out the whole "Bind to agent" step in Connectors. An agent's account is changed by clicking
the login email in the agent pane composer, whether or not anything is bound. When several accounts have the same email,
give them numbers in a fixed notation wherever an account is shown in the agent pane. Agents get the same switch through
an API (agents are first class). Moving an agent to another account must not leave a stale out-of-quota state behind.

## 2. What is there today

- **The link is the data, "bind" is one door to it.** An agent's account is a row in `db_agent_identity_links` (PK
  `agent_id, provider`; `storage/identities.rs:597`). Doors that write it: the Connectors card and Identity-tab menu
  "Bind to Agent" (`view/identity/bind-to-agent-menu.ts`), the composer chip, the failure row's "Bind account", the Swarm
  banner (`identity.account.move_agents`), the first-login flow, `Agents spawn`, and `identity.account.upsert`.
- **The composer chip already switches** (`AgentComposerStrip.tsx`, `failure/useAccountBinding.ts`), but only for signed-in
  agents that are not busy, only to `oauth_config_dir` accounts with status valid, and the apply logic (unlink aliases,
  link, `mergeBlockEnv`, force-restart with `--resume`) lives in the **frontend** (`bindAccountToAgent`,
  `bind-to-agent-menu.ts:148-218`). Nothing an agent can call reaches it.
- **Used by.** `account-card.tsx:142` ("Used by N agents"), the detail panel's chips with an Unbind button
  (`identity-accounts-tab.tsx:412-437`), and the delete confirmation ("won't start until you bind another account").
- **No default account.** A new agent gets links from its seeded template or fork source, from the picker's `accountId`, or
  from the first-login flow; an unlinked agent stops at the spawn gate and shows the login UI.
- **No numbering.** Every OAuth login mints a new account (UUID, config dir), all named `<provider>-oauth`, and nothing
  dedupes by email (`m0012` merges only identical `(provider, secret_ref)`). Labels are `context.email || name`
  (`accountLabel`, `identity-model.ts:429`); srv's `account_matches` (`agent_spawn.rs:~552`) takes the first match.
- **Stored usage limit.** It is on the pane (`agent:last_failure`, block meta), not on the account or agent, and the
  account is not in the payload; the run account comes from `agent:run_account`, stamped at spawn, with a fallback to the
  agent's *current* links for older panes (`agent_state.rs:237`). `after_rebind` clears failures of panes whose run account
  is not the new one, but only from `linkagentidentity` and `move_agents`, and only for panes in a tab whose meta matches the
  agent id and provider (`account_episodes/mod.rs:507-539`).

## 3. Design

### 3.1 One switch, in srv

A single srv command, `agent.account.switch { agent, account, when }`, does what the frontend's `bindAccountToAgent` does
today (replace the link for that provider incl. legacy aliases, update the pane env, restart with `--resume`, clear stale
limits per §3.5, publish `AgentIdentitiesChanged`, audit row naming the caller). The composer, the failure row, the Swarm
banner (`move_agents` becomes a loop over it) and the agent API (§3.4) all call it. The frontend keeps no apply logic.

### 3.2 The composer chip is the one door

- The email is a button whenever the agent has a provider, signed in or not (a signed-out agent gets the same menu; today
  that is the failure row).
- The menu lists **every signed-in account of that provider** (not only OAuth-dir ones; invalid ones are shown disabled with
  the reason), the current one marked with a check, each with: the numbered label (§3.3), and its quota state ("limited until
  16:20", "signed out") taken from the same source the Swarm chip uses. A limited account can be chosen; the menu says so.
- A last item, "Sign in with another account…", starts the provider login (today's `run-provider-login`) and switches to the
  new account when it completes.
- **Mid-turn.** The operator asked that a switch takes effect at the next turn. If the agent is idle the switch applies at
  once (restart with `--resume`, as today). If a turn is running, the choice is stored as a pending switch, the chip shows
  "→ <label> after this turn", and srv applies it when the turn ends; a "Switch now" item stops the turn first. This is the
  same busy test `move_agents` already uses. A pending switch is cancelled by choosing the current account.

### 3.3 Numbering

- Accounts with the same provider **and** the same (case-folded) email form a group. When a group has two or more live
  accounts, each shows `email ·N` (middle dot, space before, no space after: `user@example.com ·2`). A lone account shows
  the bare email.
- `N` is assigned once, in srv, when the account is created or first seen with its email: `max(N in group) + 1`, stored in
  the account's `context.seq`. It is never reused and never changes on restart; deleting `·1` leaves `·2` as `·2`. Existing
  groups are numbered by creation time in a one-time migration (`m0042_number_same_email_accounts`, run-once record in
  `db_migrations`).
- Shown everywhere an account is named in the agent pane and its menus: the composer chip, the picker, the failure row, the
  Swarm banner, "Used by", spawn results. Tooltip adds the organisation when the two accounts differ by `organizationUuid`
  (`identity_key_from_oauth_dir`), because same email can be two organisations.
- `account_matches` accepts `email ·N` and refuses an ambiguous bare email with the list of candidates, instead of the
  first match.
- Not in scope: merging duplicate logins of one real identity (same `identity_key`). Listed as D3.

### 3.4 The agent API

- RPC `agent.account.switch` is open to agents (no operator-only gate). `Identity` MCP tool gets `switch`
  (`agent` optional = self, `account` = id or `email ·N`, `when` = `now|after_turn`, default `after_turn`) and `accounts` lists
  the numbered labels and quota state. Every call is recorded as the calling agent.
- `identity.self.unlink` and `identity.account.upsert`'s link both go through the same srv function, so no door skips §3.5.

### 3.5 Quota state follows the move

One function, `after_account_change(agent)`, is called by **every** path that changes a link (the switch, `linkagentidentity`,
`unlinkagentidentity`, `identity.self.unlink`, `identity.account.upsert`, the spawn link, first login). It:
1. clears `agent:last_failure` on **every pane of the agent** whose run account differs from the new one (not only panes in a
   tab whose meta matches), and stamps `agent:run_account` for panes that have none *before* the link changes, so the
   fallback to current links can't make an old failure look like it belongs to the new account;
2. leaves a failure that belongs to the new account alone (a limit on account B is still a limit on B);
3. re-checks the new account's availability and shows it in the chip.
Hypothesis for the stale limit behind #5558 (to be confirmed by test T5 before the fix, not assumed): an older pane without
`agent:run_account` has its run account inferred from the *new* link, so its old failure looks current.

### 3.6 What replaces each thing binding did

| Binding did | Becomes |
|---|---|
| Connectors card + Identity-tab "Bind to Agent" menu | Removed. A Connectors card keeps Sign in, rename, remove, and a read-only "Used by N agents" whose chips open that agent. |
| Unbind chip | Removed. An agent without an account is a sign-in state, fixed from the chip. |
| Default account for new agents | Unchanged sources (template, fork, picker). New: a first launch with no link and **exactly one** valid account for the provider adopts it; with several, the chip shows "Choose account" and the agent waits. See D1. |
| "Used by N agents" | Kept, read-only, derived from links (legacy blob still counted). |
| Swarm "move these agents" banner | Kept; it is the bulk form of §3.1, not binding. |
| Delete-account confirmation | Text becomes "N agents use this account and will need another one." |
| Existing bindings | **No migration of links**: they are the agent's current account. Only `m0042` (numbers) is new. |

### 3.7 As built (2026-10-27)

- **Numbering, no migration.** The account list (`listidentityaccounts`) numbers any account with an email that has no number, oldest first, and records the group's high-water mark (`context.seqmax`) on every member so the highest number is not handed out again after its account is deleted. An email change (other than case or padding) drops the number. A rewrite of an account's context keeps its number and can't lower the mark. This replaces the migration `m0042` of §3.3: the first list after upgrade numbers existing groups, and it also covers every later sign-in, whose email is only known after login.
- **The queue.** A switch asked for in a turn is stored on the agent's panes as `agent:pending_account` (and `agent:pending_account_by`) and applied by the account-episode refresh (every 15 s and on a turn event) once no pane of the agent is in a turn. It survives a restart. Choosing the current account cancels it.
- **Chip.** The email is a link whenever the agent is signed in and the provider has another signed-in account; a turn no longer disables it. A signed-out agent still gets the failure row's Bind account, not the chip menu.
- **Not yet.** "Sign in with another account…" in the menu; the failure row and the Swarm banner calling `agent.account.switch` (they use their own paths, which already clear stale limits through §3.5); numbered labels on the Connectors cards.

## 4. Edge cases

- Switching to an account of another provider: refused (menu never lists it; API returns an error naming the provider).
- Two panes of one agent: the switch applies to the agent, restarts each pane that is idle, and queues the busy ones.
- Account deleted while a switch is pending: the pending switch is dropped with a note.
- Template or seeded-parent links: switching writes the agent's own link; it never edits the template.
- Offline / signed-out target: shown disabled with the reason; the API returns it as an error.

## 5. Tests

- T1 srv: the switch replaces the link (incl. alias rows), restarts idle panes, queues a busy pane and applies it at turn end;
  cancelled by choosing the current account.
- T2 srv: every link-changing path calls `after_account_change` (table-driven over the paths in §3.5).
- T3 srv: numbering: `·N` assigned once, never reused after delete, stable across restart; migration numbers by creation time;
  `account_matches` refuses an ambiguous bare email and accepts `email ·2`.
- T4 frontend: chip opens for signed-in and signed-out agents; lists all signed-in accounts of the provider with the current
  one marked, quota text, numbered labels; pending-switch label; "Bind to Agent" absent from Connectors card and Identity tab.
- T5 srv regression: a pane without `agent:run_account` and a limit on account A, then a switch to B: the limit is gone and the
  chip shows B available; same for each §3.5 path.
- T6 MCP: `Identity switch` self and other agent, ambiguous email refused, attributed to the caller.
- Live check in my `task dev`: two Claude accounts with one email, switch while idle and mid-turn.

## 6. Phases (each its own PR with a changeset)

- **P1 (srv):** the switch command, `after_account_change` on all paths, numbering + `m0042`, `account_matches`. Tests T1-T3, T5.
- **P2 (frontend):** composer menu and pending state, numbered labels everywhere in the agent pane, remove the Bind menus and
  Unbind, read-only Used by, delete text. T4. Uses P1.
- **P3 (agents):** `Identity switch`, tool description, operator-config / help text, `app-api-manifest.json`. T6.

## 7. Decisions for the operator

Taken as recommended (AgentY, relaying the operator's standing "go with best recommendations", 2026-10-26); the operator may still object after reading.

- **D1. New agent with no account.** Recommended: adopt the provider's only valid account automatically; with several, wait
  for a choice in the chip. Alternative: always ask.
- **D2. Mid-turn.** Recommended: queue and apply after the turn, with "Switch now". Alternative: always switch at once (stops
  the turn).
- **D3. Duplicate logins of the same identity** (same account and organisation): numbered like any other here. Recommended:
  leave them; a later "merge duplicates" is separate.
- **D4. Notation** `email ·N`: recommended; alternatives `email (2)` or `email #2`.

## 8. Risks

- The restart on switch interrupts anything the agent was doing mid-tool; the busy test and the queue are the guard.
- Moving apply logic from the frontend to srv touches `bindAccountToAgent`, which the failure row and the Swarm banner also
  use; their tests (`recheck-after-bind`, `account_episodes/tests.rs`) are the safety net.
- `context.seq` is a new field on accounts that sync into `identity_store`; the mirror must carry it.

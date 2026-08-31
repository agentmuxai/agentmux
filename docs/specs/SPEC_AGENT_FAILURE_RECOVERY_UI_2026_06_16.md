# SPEC: Agent Failure Recovery UI (per-error-class actions)

- **Status:** Draft / proposed
- **Date:** 2026-06-16
- **Author:** AgentA
- **Area:** `frontend/app/view/agent` (pane UI + state), `frontend/app/view/accounts` (Armory), `agentmux-srv` agent failure event
- **Related:** `SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md` (the classifier — Phase 1/2, shipped #1353 + #1464), Armory / Accounts (#1478)

---

## 1. Goal

When an agent dies, the pane now **shows the real cause** (PR #1464: the `agentfailure`
wave event carries a classified `AgentFailure`, rendered today as red log lines). This spec
turns that diagnosis into **one-click recovery**: a per-error-class **recovery banner** in the
agent pane whose actions match the failure — *Login Again* / *Armory* for auth,
*Retry* (with a 5-second **auto-retry**) for transient throttling, and class-appropriate
actions for the rest.

> The user must never have to read a stderr tail and guess what to do. The error itself
> should offer the fix.

## 2. Current state (what exists)

| Piece | Where | Notes |
|---|---|---|
| Failure taxonomy `AgentFailure { code, title, detail, exitCode?, signal?, stderrTail?, retryable }` | `agentmux-srv/src/agents/failure.rs`; TS `frontend/types/gotypes.d.ts:78` | 11 classes; `retryable` already true for transient classes |
| `agentfailure` wave event (per block) | `wps::EVENT_AGENT_FAILURE`; emitted in `subprocess.rs` process-waiter | classified on non-zero exit OR stdout error `result` frame |
| Event → pane | `frontend/app/view/agent/hooks/useControllerStatusEvents.ts:42` | currently only `opts.log(...)` red text lines |
| Re-run a turn | `sendMessage()` `hooks/useAgentCommands.ts:231` → `RpcApi.AgentInputCommand` (`rpc-api.ts:1202`) → backend `agentinput` → `spawn_turn` (`subprocess.rs:345`) | resumes via `--resume <session_id>`; busy turns queue + drain on exit |
| Last user message | `documentAtom` last `UserMessageNode` (`view/agent/types.ts:279`) | the text to re-submit for "Retry" |
| Armory → Accounts | `openBundleManager()` (`modals/bundle-manager-modal.tsx:255`) | singleton modal; "Accounts" is the default section |
| Agent → account lookup | `RpcApi.ListAgentIdentitiesCommand({agent_id})` → `GetIdentityAccountCommand({id})` (`rpc-api.ts:792`, `:686`) | resolves the agent's provider account |
| Re-auth (OAuth) | `RpcApi.AccountOAuthStartCommand` + `AccountOAuthPollCommand` (`rpc-api.ts:747`, `:756`) | service-OAuth connect flow (#1478) |
| Re-auth (API key) | `RpcApi.AccountKeyVerifyCommand` (`rpc-api.ts:717`) | for `kind === "api_key"` accounts |
| **Existing** login-retry affordance | `AgentRetryBar` (`agent-view.tsx:993`) | a login retry button — **must be reconciled/unified** (§7) |
| Session-interrupted precedent | `session_recovery.rs`; `session:was_interrupted` → AgentControlBar banner | the "Resume" pattern this banner generalizes |

**The gap:** the rich failure is shown but inert. No actions, no auto-retry, and a second
(login-only) retry surface (`AgentRetryBar`) already exists in parallel.

## 3. The action matrix (core of this spec)

Each `FailureClass` maps to a banner with an icon, a primary action, and an optional
secondary action. `retryable` (from the classifier) drives whether **auto-retry** arms.

| `code` | Icon | Primary action | Secondary | Auto-retry | Rationale |
|---|---|---|---|---|---|
| `auth` | 🔐 | **Login Again** (re-auth the agent's account inline) | **Armory → Accounts** | no | 401 — creds rejected/expired; re-auth then offer Retry |
| `rate_limited` | ⏱ | **Retry now** | Dismiss | **yes, 5 s** | transient 429; retry shortly |
| `overloaded` | 🌀 | **Retry now** | Dismiss | **yes, 5 s** | transient 529; server overloaded |
| `network` | 🌐 | **Retry now** | Dismiss | **yes, 5 s** | transient connectivity |
| `usage_limit` | 🚫 | **Armory → Accounts** (switch account / upgrade) | Dismiss | no | plan/quota — not transient; a different account may have budget |
| `context_exceeded` | 📏 | **New session & retry** (fresh `session_id`, re-send) | Dismiss | no | window full; resuming won't help |
| `max_turns` | 🔁 | **Continue** (re-run, resumes via `--resume`) | Raise `--max-turns` → settings | no | hit the cap; continuing extends |
| `killed` | ⛔ | **Retry now** | Dismiss | no (caution) | OOM/signal; auto-looping could thrash memory |
| `spawn_failure` | 🧩 | **Provider setup** (open install/provider help) | Dismiss | no | CLI couldn't launch — needs fixing first |
| `no_output` | ❔ | **Retry now** | Dismiss | no | exited clean, no result — usually one-off |
| `unknown_non_zero` | ⚠ | **Retry now** | **Show details** (stderr tail) | no | fallback; expose the raw evidence |

Notes:
- **Retry** = re-submit the last `UserMessageNode` via `sendMessage(last.message)` → resumes
  the prior `session_id` (no context loss). If no last user message, fall back to re-spawn.
- **Login Again** runs the re-auth chain (§5.2) inline; on success it auto-offers Retry.
- **New session & retry**: clear `agent:sessionid` for the block, then re-send (starts fresh).

## 4. The recovery banner

### 4.1 Component — reuse `PaneRow`, do **not** build a bespoke banner
The pane-accessories system (`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15` §5.2) already
ships **`PaneRow`** — the shared status-accented accessory row (`sigil + title + meta + tail +
actions + optional inline-expanded body`) that `SessionDigestBanner`, the `ActivityDock` rows,
and the fork bar all render through. The failure surface maps **exactly** onto it:

| PaneRow slot | Failure content |
|---|---|
| `sigil` | per-class icon (🔐 / ⏱ / 🌐 / …) |
| `accent` | `error` (the 3px left border) |
| `title` | `failure.title` |
| `meta` | `code` · `[exit N]` / `[signal N]` · `(retryable)` |
| expanded body | `failure.detail` + collapsible `stderrTail` |
| `actions[]` | the per-class buttons from §3 (glyph + handler): Retry / Login Again / Armory / … |

So there is **no `AgentFailureBanner.tsx` / `_failure-banner.scss`** — the failure surface is a
**`PaneRow` descriptor**, produced by a pure `failure → PaneRow` function (à la
`digest-accessory.ts`) and fed from `lastFailure` (a transient accessory under the "derive from
a source of truth" rule — §5.3, like the digest's `loading`/`dismissed` signals). The auto-retry
(§6) is a `PaneRow` action whose label counts down.

### 4.2 Mount point — register into the pane **region** model (do not hand-place)
The pane has a declarative region system (`PaneRegions.tsx`, from
`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md`). The failure banner **registers a surface
into the `alert` region** — the same region that already hosts `AgentDecisionPanel`,
`AgentQuestionPanel`, and `AgentDisconnectedBanner` — rather than being positioned by JSX
order. Region priority (§4.4) orders it among its siblings. It supersedes `AgentRetryBar` (§7).

### 4.3 State
Add a first-class `lastFailure: AgentFailure | null` to `AgentPaneState`
(`agent-pane-state/types.ts:166`), sibling to `sessionStats`. New reducer command
`{ type: "TurnFailure"; failure: AgentFailure | null }` (`reducer.ts`, after `TurnEnd`),
which sets/clears `lastFailure`. Selector `currentFailure(state)`.

`useControllerStatusEvents.ts` (the `agentfailure` handler) dispatches
`{ type: "TurnFailure", failure }` into the pane state *in addition to* its existing log
line. A successful Retry / re-auth, or Dismiss, dispatches `{ type: "TurnFailure", failure: null }`.
A new turn starting (`TurnReset`) also clears `lastFailure`.

### 4.4 Relationship to AskUserQuestion & the pane-prompt family (architecture)

The failure banner is **not a one-off** — it's the newest member of a family of
**pane-anchored interactive prompts** (`AgentDecisionPanel`, `AgentQuestionPanel` /
AskUserQuestion, `AgentDisconnectedBanner`, `AgentRetryBar`, the `was_interrupted` resume
banner). The codebase already supplies most of the shared architecture; the principle is
**unify the presentation, keep the semantics distinct.**

- **Layout — already uniform; use it.** `PaneRegions.tsx`'s **`alert` region** already groups
  the working-row, decision panel, question panel, and disconnected banner. The failure banner
  *registers a surface* there (§4.2). At the layout level, "should it be uniform?" → *it
  already is*.
- **Component shape — `PaneRow` *is* the shared primitive (reuse it; don't invent one).** The
  pane-accessories system already ships **`PaneRow`** (sigil/title/meta/tail/actions/expanded-body
  + status accents); the digest, `ActivityDock`, and fork bar render through it. The failure
  banner is just **another `PaneRow`** (§4.1) — not a bespoke component. The richer interactive
  panels — `AgentDecisionPanel` and `AgentQuestionPanel` (AskUserQuestion) — are currently
  **bespoke multi-option** panels (radio fieldsets / free-text) that **exceed** PaneRow's
  single-row shape; they share the *region* (and the accessory concept), not the row primitive.
  So the failure banner reuses `PaneRow` **now**; converging the richer question/decision panels
  onto a shared *richer-prompt* primitive (or a PaneRow options slot) is a **separate, optional
  P3 refactor** — do not block the failure banner on it.
- **Semantics — keep distinct (do NOT over-unify).** AskUserQuestion + tool-decision are
  **in-turn, control-protocol** prompts: the agent is *blocked waiting*; they're driven by
  `ToolNode`s in `awaiting_answer`/pending-decision states (part of the conversation document);
  the outcome routes **back to the CLI** (`ToolDecisionCommand` / a `control_response` carrying
  the answer — `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md`). The failure banner is **post-turn,
  local recovery**: the agent is *dead*; it's driven by `lastFailure` pane state (not a document
  ToolNode); its actions (retry / re-auth / navigate) are *local* and route nowhere. Folding the
  failure into the ToolNode/control-protocol model would be a leaky abstraction. **Share the
  region + `PanePrompt`; keep the data sources separate.**
- **Priority / coexistence.** Sharing the `alert` region means precedence must be defined: a
  **live AskUserQuestion / tool-decision the agent is waiting on outranks** a post-mortem failure
  banner (an in-turn prompt and a dead agent are usually mutually exclusive, but a queued
  question from a prior turn can coexist). The region model owns the ordering; the failure
  banner sorts **below** live in-turn prompts.

## 5. Actions — wiring

### 5.1 Retry
```
const last = lastUserMessage(documentAtom());   // last UserMessageNode
if (last) await commands.sendMessage(last.message, false);  // → AgentInputCommand → spawn_turn
else      await RpcApi.ControllerResyncCommand(... forcerestart ...); // fallback
dispatchPane(blockId, { type: "TurnFailure", failure: null });
```

### 5.2 Login Again (auth)
```
const ids = await RpcApi.ListAgentIdentitiesCommand(client, { agent_id });
const acct = await RpcApi.GetIdentityAccountCommand(client, { id: ids.find(i=>i.provider==="claude").account_id });
if (acct.kind === "oauth") {
  const { sessionId } = await RpcApi.AccountOAuthStartCommand(client, { provider: acct.provider, name: acct.name });
  // poll AccountOAuthPollCommand(sessionId) until success|failed
} else {
  openBundleManager();   // API-key accounts: send them to Accounts to re-enter the key
}
// on success → toast + offer Retry (do NOT auto-retry auth; user just acted)
```
**Backend enhancement (recommended):** include the agent's resolved `account_id` (and
`provider`) on the `AgentFailure` for `code === "auth"` so the button targets the exact
account without a second lookup. New optional fields `accountId?`, `provider?` on
`AgentFailure` (additive, snake→camel as today).

### 5.3 Armory → Accounts
```
import { openBundleManager } from "@/app/modals/bundle-manager-modal";
onClick={() => openBundleManager()}   // opens the singleton modal on the Accounts section
```
*Stretch:* extend `openBundleManager(opts?: { section?: "accounts"; provider?: string })` to
deep-link to a provider's add/connect form.

## 6. Auto-retry (transient classes)

For `rate_limited | overloaded | network` (and any future `retryable` class), the primary
button becomes a **countdown that fires itself**:

- On banner mount for a transient failure: start a **5 s countdown**; button label
  `Retry now (4s…)` updating each second.
- **Click → retry immediately** (cancel the timer, run §5.1).
- **At 0 → auto-fire** the same Retry.
- **Dismiss/×** cancels the timer (no retry).
- **Bounded:** a fixed ladder of auto-retries, then the banner stays with a
  manual **Retry now** only (no more auto-firing) and a note "auto-retry stopped after N
  attempts." Prevents an infinite hammer on a sustained 429.
  - *As designed (2026-06):* 2 auto-retries, backoff `5s → 10s`.
  - *As shipped (2026-08-31):* 5 auto-retries, `5s → 15s → 30s → 60s → 120s`,
    each ±20% jittered — ~3.9 min of coverage. The original ~15s was shorter
    than a typical 429/529 episode, so genuinely transient failures exhausted
    the budget while still retryable; jitter keeps a Fleet/cron burst from
    retrying in lockstep. The live value is `AUTO_RETRY_BACKOFF_S` in
    `frontend/app/view/agent/hooks/useAgentFailure.ts` — treat that as the
    source of truth over this line. See
    `docs/reports/REPORT_TRANSIENT_API_FAILURE_RETRY_STATE_2026_08_31.md`.
- Implementation: a `createSignal` countdown + `setInterval`/`setTimeout` in the banner,
  cleaned up on `onCleanup` and on `lastFailure` change. Attempt count tracked on
  `AgentPaneState` (e.g. `failureAutoRetries: number`, reset on `TurnReset`).

> The user's ask — "retry button you can click immediately, or it retries after 5 s
> automatically" — is exactly this: one control, two ways to fire.

## 7. Reconciliation with `AgentRetryBar`

`AgentRetryBar` (`agent-view.tsx:993`) is a login-retry bar predating the classifier. This
spec **subsumes** it: the recovery banner handles `auth` (Login Again / Armory) and
all other classes uniformly. Plan: route `AgentRetryBar`'s login affordance into the banner's
`auth` case and remove the standalone bar (or keep it rendering the banner) to avoid two
competing surfaces. Likewise, the `session:was_interrupted` "Resume" banner
(`session_recovery.rs`) shares the Retry mechanism and should adopt the same component later.

## 8. Phasing

- **P1 — Banner + transient recovery.** Failure **`PaneRow` descriptor** (`failure → PaneRow`,
  reusing the accessory primitive) + `lastFailure` state + `TurnFailure` dispatch; **Retry** +
  **5 s auto-retry** for `rate_limited`/`overloaded`/`network`; generic **Retry / Show-details**
  for the rest. (Delivers the user's headline ask.)
- **P2 — Auth actions.** **Login Again** (OAuth/API-key re-auth) + **Armory → Accounts**;
  backend `accountId`/`provider` on `AgentFailure`; auto-offer Retry after re-auth.
- **P3 — Specialized actions.** `usage_limit` (account switch), `context_exceeded`
  (new-session & retry), `max_turns` (Continue / raise cap), `spawn_failure` (provider setup);
  retire `AgentRetryBar`; unify the `was_interrupted` banner.

## 9. Open questions

1. **Auth re-auth UX** — inline (poll in the banner) vs. opening Armory and letting the
   user complete there? Proposal: try inline OAuth; fall back to `openBundleManager()`.
2. **Auto-retry default-on?** Proposal: on for transient classes, capped (2× backoff). A
   `settings.json` toggle (`agent:autoRetryTransient`) is a stretch.
3. **`killed` (OOM)** — auto-retry off by default (thrash risk); manual only. Confirm.
4. **Deep-link** into Accounts for a specific provider — worth the `openBundleManager` API
   change now, or P3?
5. **Multi-provider agents** (codex/gemini) — `Login Again` must target the *failing*
   provider; the backend `provider` field (§5.2) resolves this.

## 10. Non-goals

- Auto-retry **orchestration/backoff policy** beyond the simple 2× cap (a richer policy is a
  follow-up; the classifier already emits `retryable`).
- Reconciling the stale `db_agent_instances.status='running'` liveness bug (separate).
- Changing the classifier taxonomy (this is purely the **UI/action** layer on top of it).

## 11. Dev/test plan

- Build with `task dev` (hot-reload) to iterate the banner + countdown.
- Repro classes deterministically: `auth` → de-auth an agent (the live Spixo 401 case);
  `rate_limited`/`network` → stub the provider; `killed` → SIGKILL the child. Confirm each
  banner + action. Unit-test the reducer `TurnFailure` arm and the auto-retry cap.

# SPEC: MuxBus — GitHub PR review notifications (end-to-end MVP)

**Date:** 2026-06-20
**Status:** proposed — Planned (superseded in part — see note below)
**Author:** smike

> **2026-08-07 note:** what this doc says about how the cloud consumer
> matches a PR to an agent describes the original plan and is no longer
> accurate; the consumer side is designed in the private cloud repo. The
> PR-body tag convention (§5) still applies. The rest of this doc (delivery architecture, M1/M4/M5 gaps) still
> reflects real historical planning context but has not been re-verified
> against current code as of this note.

---

## 1. Goal

When a GitHub PR review arrives (approved / changes_requested / commented), the
agent that opened the PR receives an injected notification inside its running
Claude session — no manual polling, no checking GitHub, no delay.

---

## 2. Current state — what's already built

### 2.1 Cloud (agentmux-cloud)

| Component | Status |
|-----------|--------|
| GitHub webhook consumer | ✅ deployed |
| PR review handler | ✅ routes review→injection |
| MuxBus injection API `/reactive/inject` | ✅ production |
| Pending poll endpoint `/reactive/pending/:id` | ✅ production |
| WebSocket wake signal `/ws` | ✅ production |
| Sign-in user pool | ✅ production |

### 2.2 Desktop app (agentmux)

| Component | File | Status |
|-----------|------|--------|
| `cloud_subscriber.rs` | WS + poll + inject + ACK loop | ✅ fully implemented |
| `muxbus_handlers.rs` | login / status / disconnect RPCs | ✅ fully implemented |
| `storage/muxbus.rs` | SQLite credentials (`db_muxbus_credentials`) | ✅ |
| `inject_muxbus_env()` | Injects `MUXBUS_TOKEN` into agent spawn env | ✅ |
| `reactive.rs` | `add_agent()` on reactive-register, `remove_agent()` on deregister | ✅ |
| `AgentMuxConnectPanel.tsx` | Full connect/disconnect UI | ✅ but gated |
| Accounts gallery tile | Armory → Accounts → AgentMux tile | ✅ exists |

### 2.3 How the delivery path works (end-to-end, when connected)

```
GitHub PR review
    ↓  (webhook)
agentmux-cloud GitHub consumer
    → resolves the PR to an agent → "smike"
    → POST /reactive/inject { target_agent: "smike", message: "PR #123 approved" }
    → wakes connected WS clients

agentmux-srv cloud_subscriber.rs (running in the desktop app)
    ← receives { type: "inject_available" }
    → GET /reactive/pending/smike  (with MUXBUS_TOKEN)
    → ReactiveHandler.inject_message(req)   ← injects into Claude session
    → POST /reactive/ack { injection_ids: [...] }
```

The entire delivery chain is implemented. What's broken are the connection
points at either end.

---

## 3. Gaps

### 3.1 Build vars not set → sign-in disabled  ← P0 blocker

`AgentMuxConnectPanel.tsx` reads:
```typescript
const MUXBUS_CLIENT_ID =
    (import.meta.env.VITE_MUXBUS_CLIENT_ID as string | undefined) ?? "";
```

When `VITE_MUXBUS_CLIENT_ID` is empty (which it is in all current builds), the
connect panel shows:
> "AgentMux Cloud sign-in isn't configured in this build (client ID missing)."

The button is disabled. Nobody can sign in.

**Fix:** Set `VITE_MUXBUS_COGNITO_DOMAIN` and `VITE_MUXBUS_CLIENT_ID` in the
build. These should be baked in via `.env.production` or the Taskfile build
step — not committed as secrets, but as **non-secret public OAuth client IDs**
(Cognito PKCE client IDs are public by design; the PKCE verifier is the secret).

### 3.2 Routing a review to the right agent  ← P0 blocker

The consumer needs a reliable way to tell which agent opened a PR, for any
agent name. The agent-side answer is to embed the agent's own id in the PR
body as a hidden HTML comment (see §5):
```
<!-- agentmux:agent_id=smike-06122 -->
```

How the consumer reads that tag and resolves a PR to an agent is designed
in the private cloud repo.

### 3.3 Reactive auto-registration gap  ← P1

`reactive.rs:232` calls `cloud_subscriber.add_agent(agent_id)` when an agent
registers for reactive delivery. But this registration only happens when the
agent explicitly calls the reactive subscribe endpoint
(`POST /api/v1/reactive/subscribe`). Agents that never call this (e.g., those
only using MCP injection) are invisible to the cloud subscriber.

**Fix:** Call `add_agent()` at agent-start time in `agent_handlers.rs`, not only
on explicit reactive subscribe. Every running agent should be visible to the
cloud subscriber regardless of whether it's used the reactive API.

### 3.4 GitHub webhook setup is manual  ← P1

Users must manually configure a GitHub org/repo webhook pointing to the cloud
consumer endpoint. There is no setup flow, no documentation surface in the UI,
and no validation that the webhook is working.

**Fix:** Add a "Connect GitHub" step in Armory → Accounts → GitHub tile
(currently OAuth / PAT only). The step shows the webhook URL + secret to paste
into GitHub, and a "Test" button that confirms delivery.

Alternatively, use a GitHub App installation flow that auto-configures the webhook.

### 3.5 No dedicated notification UX  ← P2

Injections arrive as plain text in the agent's Claude session — the agent
processes them as incoming messages. The user only knows a review arrived if
they're actively watching the agent pane. There is no:
- Toast notification ("🔔 PR #1234 approved by reagent")
- Badge on the agent pane tab
- "Jump to PR" affordance

This is acceptable for MVP (the agent acts on the review autonomously), but
should be addressed for team use.

---

## 4. MVP scope

Minimal set of changes to make the use case work end-to-end:

| # | Change | Where | Effort |
|---|--------|--------|--------|
| M1 | Set `VITE_MUXBUS_CLIENT_ID` + `VITE_MUXBUS_COGNITO_DOMAIN` in builds | `.env.production` / Taskfile | 30 min |
| M2 | Embed agent ID in PR bodies (§3.2) | Convention + agent prompt / git hook | 1 day |
| M3 | Cloud consumer reads the PR-body tag | agentmux-cloud | 2h |
| M4 | Auto-register agents with cloud subscriber at startup | `agentmux-srv/src/server/agent_handlers.rs` | 2h |
| M5 | Document GitHub webhook setup | `docs/` or Armory help text | 1h |

P1 additions (post-MVP):
| # | Change | Effort |
|---|--------|--------|
| P1b | Armory webhook setup flow | 2 days |
| P1c | Toast notification on injection delivery | 1 day |

---

## 5. M2 — Embedding agent ID in PR bodies

Convention: all agent-opened PRs include the following in the PR body:

```markdown
<!-- agentmux:agent_id=smike-06122 -->
```

This can be enforced via:
- Agent CLAUDE.md instruction: "always include `<!-- agentmux:agent_id=$AGENTMUX_AGENT_BUS_ID -->` in PR bodies"
- A git hook in `scripts/` that injects it into `gh pr create` calls
- The `agentmux-mcp` `Shell` wrapper (future: intercept `gh pr create` and append)

The cloud consumer reads the tag from the PR body; how it uses it is
designed in the private cloud repo.

---

## 6. M4 — Auto-register agents at startup

In `agent_handlers.rs`, when an agent controller starts (after `block_id` is
assigned and the agent process is spawned), call:

```rust
if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
    sub.add_agent(&block_id);
}
```

And on agent stop/dispose:
```rust
if let Some(sub) = crate::muxbus::cloud_subscriber::get_global_subscriber() {
    sub.remove_agent(&block_id);
}
```

This ensures every running agent is subscribed to cloud injection from the moment
it starts, regardless of whether it ever calls the reactive API.

---

## 7. Notification message format

The cloud consumer currently sends an "urgent jekt" message.
The format should be standardised so agents can parse it consistently:

```
🔔 GitHub PR review — [ACTION]

PR #[NUMBER]: [TITLE]
Reviewer: [REVIEWER_LOGIN]
Branch: [HEAD_BRANCH] → [BASE_BRANCH]
URL: [PR_URL]

[REVIEW_BODY if present, truncated to 500 chars]

---
This notification was delivered via AgentMux Cloud.
```

Actions: `approved`, `changes_requested`, `commented`, `dismissed`.

---

## 8. Armory — not a blocker

The Armory redesign (`docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md`) is
**not required** for this MVP. The accounts gallery tile and `AgentMuxConnectPanel`
already exist and work correctly once `VITE_MUXBUS_CLIENT_ID` is set (M1).

Armory work becomes relevant for:
- GitHub webhook setup UI (P1b)
- Subscription management / tier display

---

## 9. Files to change

### agentmux-cloud (separate repo)

- GitHub consumer: read the agent ID from the PR body tag (M3). Designed
  in the private cloud repo.

### agentmux (this repo)

| File | Change |
|------|--------|
| `.env.production` (or Taskfile) | Set `VITE_MUXBUS_CLIENT_ID` + `VITE_MUXBUS_COGNITO_DOMAIN` (M1) |
| `agentmux-srv/src/server/agent_handlers.rs` | `add_agent()` at spawn, `remove_agent()` at dispose (M4) |

---

## 10. Acceptance criteria (MVP)

- User can sign into AgentMux Cloud from Armory → Accounts → AgentMux tile
- Agent that opens a PR has `<!-- agentmux:agent_id=X -->` in the PR body
- When a reviewer approves / requests changes on that PR, the agent receives an
  injected message within ~5 seconds (WS wake) or ~30 seconds (polling fallback)
- The injected message follows the §7 format and includes PR URL
- Delivery works when the desktop app is running; gracefully queues when offline
  (cloud holds injections until next poll)
- Agents not registered for reactive delivery still receive injections (M4)

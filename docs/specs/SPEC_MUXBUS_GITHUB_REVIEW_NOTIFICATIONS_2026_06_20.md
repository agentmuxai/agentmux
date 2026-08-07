# SPEC: MuxBus — GitHub PR review notifications (end-to-end MVP)

**Date:** 2026-06-20
**Status:** Planned (superseded in part — see note below)
**Author:** smike

> **2026-08-07 note:** §3.2 and §6's "Priority: extracted ID > static
> mapping > regex pattern" (tag checked before username) describes the
> ORIGINAL implementation and is no longer accurate — the priority was
> reversed (username-first, tag-fallback) and agent-mapping.ts's numbered
> pattern was made host-agnostic. See
> `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` for the current behavior.
> The rest of this doc (delivery architecture, M1/M4/M5 gaps) still
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

| Component | File | Status |
|-----------|------|--------|
| GitHub webhook consumer | `consumers/github/handler.ts` | ✅ deployed |
| PR review handler | `consumers/github/events/review.ts` | ✅ routes review→injection |
| Agent-ID mapping | `consumers/github/agent-mapping.ts` | ⚠️ hardcoded — see §3.2 |
| MuxBus injection API | `server/src/index.ts` `/reactive/inject` | ✅ production |
| Pending poll endpoint | `server/src/index.ts` `/reactive/pending/:id` | ✅ production |
| WebSocket wake signal | `server/src/index.ts` `/ws` | ✅ zero-metadata broadcast |
| Cognito user pool | CDK stack | ✅ `muxbus-auth-prod` |

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
agentmux-cloud consumers/github/events/review.ts
    → getAgentId(pr.author) → "smike"
    → POST /reactive/inject { target_agent: "smike", message: "PR #123 approved" }
    → broadcastInjectAvailable() to all connected WS clients

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

### 3.2 Agent-mapping is hardcoded and incomplete  ← P0 blocker

`consumers/github/agent-mapping.ts` only handles:
- GitHub App bots: `agent{x|y|a-g|1-5}-workflow[bot]`
- PAT usernames: `agent{x|y|a-g|1-5}-asaf`

Any agent outside this naming scheme (e.g., `smike-06122`, `parko-workflow[bot]`,
`a5af-asaf`) gets `undefined` from `getAgentId()` and the injection is silently
dropped. The agent is never notified.

**Fix options (in order of preference):**

**Option A — Dynamic mapping via agent `block_id` in PR description (preferred)**

When an agent opens a PR, embed its own `AGENTMUX_AGENT_BUS_ID` in the PR body
as a hidden HTML comment:
```
<!-- agentmux-agent-id: smike-06122 -->
```

The webhook consumer reads this from the PR body and uses it directly as the
`target_agent` — no static mapping needed, works for any agent name.

**Option B — Cloud user-configurable mapping**

Add a REST endpoint `POST /mapping` (authenticated) where a user can register
`{ github_username: "smike-asaf", agent_id: "smike-06122" }`. The consumer
consults this table before falling back to the static patterns.

**Option C — Pattern-based extraction (stopgap)**

Extend the regex to extract the agent prefix from any `{prefix}-workflow[bot]`
or `{prefix}-{github_handle}` username. Less precise but covers the long tail.

Recommendation: **Option A for new PRs** (zero cloud changes needed) +
**Option B for the full solution** (user-configurable via Armory).

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
| M2 | Embed agent ID in PR bodies (Option A) | Convention + agent prompt / git hook | 1 day |
| M3 | Update cloud `agent-mapping.ts` to extract from PR body | `consumers/github/events/review.ts` | 2h |
| M4 | Auto-register agents with cloud subscriber at startup | `agentmux-srv/src/server/agent_handlers.rs` | 2h |
| M5 | Document GitHub webhook setup | `docs/` or Armory help text | 1h |

P1 additions (post-MVP):
| # | Change | Effort |
|---|--------|--------|
| P1a | User-configurable mapping API in cloud (Option B) | 1 day |
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

The cloud consumer (`review.ts`) reads the PR body from the webhook payload
(`payload.pull_request.body`) and extracts the agent ID with:

```typescript
const AGENT_ID_RE = /<!--\s*agentmux:agent_id=([^\s>]+)\s*-->/;
function extractAgentIdFromBody(body: string | null): string | undefined {
    if (!body) return undefined;
    return body.match(AGENT_ID_RE)?.[1];
}
```

Priority: extracted ID > static mapping > regex pattern.

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

The cloud consumer (`review.ts`) currently sends an "urgent jekt" message.
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
- User-configurable agent-to-GitHub mapping (P1a)
- GitHub webhook setup UI (P1b)
- Subscription management / tier display

---

## 9. Files to change

### agentmux-cloud (separate repo)

| File | Change |
|------|--------|
| `consumers/github/events/review.ts` | Extract agent ID from PR body (M3); fall back to static mapping |
| `consumers/github/agent-mapping.ts` | Add `extractAgentIdFromBody()` helper |

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

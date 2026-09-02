# Spec: Warden Widget

**Branch:** `agenty/governance-widget-spec`
**Status:** Draft — design / discovery
**Date:** 2026-05-25
**Author:** AgentY
**Depends on:** `specs/lan-awareness-and-embedded-jekt-api.md` (Phases 1–2)

---

## Name

**Warden** — one who watches over an institution. Authority + observation,
matches the monitor-and-control role across boundaries (Host / LAN / Internet).
Fits the existing single-word, evocative widget vocabulary (`swarm`, `drone`,
`forge`) without bureaucratic baggage. The *concept* it implements is still
called "governance" in this doc — the widget is just the surface.

---

## TL;DR

The **Warden** widget monitors and controls every AgentMux instance reachable
from this host — across three trust layers: **Host**, **LAN**, and **Internet**.
It renders the same 3-layer fabric that the jekt cascade already uses (tiers 1-2
/ tier 3 / tier 4) and exposes per-layer controls: identity, policy, audit,
kill-switch, quotas.

**Sequencing:** networking first. The widget design is unblocked, but LAN-layer
features have no substrate until `lan-awareness-and-embedded-jekt-api.md` Phase 1
(mDNS discovery) and Phase 2 (embedded MCP server) ship. We can build the widget shell
+ Host layer in parallel with networking work and light up LAN/Internet as their
substrates arrive.

---

## Why a Warden — and why now

AgentMux is moving from "one process talking to a cloud relay" to a **mesh of
instances** (host process + LAN peers + opt-in cloud relay). With more agents in more
places, the operator needs a single place to:

- See **who** is running (which agents, on which instance, with which identity)
- See **what** they are doing (jekt activity, message bus traffic, tool calls)
- Decide **who can talk to whom** (cross-instance and cross-layer policy)
- Hit **stop** on a runaway agent, an entire instance, or a whole layer
- Keep an **audit trail** that is immutable and reviewable

Today these capabilities are scattered (settings.json, swarm widget, logs, AgentBus
console) or absent (no LAN policy because LAN doesn't exist yet). The Warden
consolidates them on the same conceptual surface the jekt cascade already uses, so
operators learn one mental model.

---

## The 3-layer governance model

| Layer | Trust | Substrate | Jekt tier | Reachability | Auth |
|------|------|-----------|----------|--------------|------|
| **L1 — Host** | Trusted | This `agentmuxsrv-rs` process: PTYs, ReactiveHandler, MessageBus | Tiers 1–2 | < 1 ms, same process | `X-AuthKey` (existing) |
| **L2 — LAN** | Semi-trusted | mDNS-discovered peers on `_agentmux._tcp.local` | Tier 3 | 1–10 ms over LAN HTTP | Shared LAN key (proposed) |
| **L3 — Internet** | Untrusted | AgentBus cloud relay (`agentbus.asaf.cc`) | Tier 4 | 100 ms – 15 s | `AGENTBUS_TOKEN` |

The trust gradient is deliberate: defaults tighten as you move outward. A host-local
jekt is allowed by default; a LAN jekt requires the LAN to be enrolled; an
internet/cloud jekt requires explicit opt-in token. The widget renders this gradient
literally — three stacked panels, each with its own toggles, lists, and audit feed.

---

## Networking first? (the question the user asked)

**Yes — networking first, widget design in parallel.**

Concretely:

1. **`lan-awareness-and-embedded-jekt-api.md` Phase 1 (mDNS discovery)** is a hard
   prerequisite for the L2 panel. Without `LanDiscovery` + `GET /api/lan-instances`,
   the widget has no peers to list and no way to address them.
2. **Phase 2 (embedded MCP server)** is the prerequisite for cross-instance policy
   propagation — once peers expose a uniform MCP/HTTP surface, the widget can push
   governance state (allowlists, quotas, kill flags) to peers.
3. **Phase 3 (LAN forwarding)** is when L2 control actions actually take effect
   end-to-end.

Build order:

```
Networking:   [Phase 1: mDNS] → [Phase 2: MCP/SSE] → [Phase 3: LAN forward] → [Phase 4: cloud fallback]
Warden:       [Shell + L1]    → [L2 read-only]    → [L2 control]           → [L3 read/control]
```

The widget shell, L1 (Host) monitoring, and L1 controls (kill-agent, pause-instance)
can ship as soon as we agree on the design — they do not need LAN. L2 and L3 light up
behind their networking counterparts.

---

## Best-practices research

Distilled from CNCF (Kubernetes RBAC, NetworkPolicy, OPA), zero-trust networking,
capability-based security, and existing AI-agent control planes (LangSmith, AgentOps,
Anthropic admin console).

Principles we adopt:

1. **Defense in depth across layers.** Each layer enforces its own policy. A LAN-layer
   deny is not bypassable by an Internet-layer allow.
2. **Capabilities, not roles.** Each agent gets an explicit capability set
   (`can.jekt.lan`, `can.send.cloud`, `can.exec.tool:bash`). Roles are syntactic sugar
   over capability bundles.
3. **Policy as data.** Governance state lives in a versioned, signable document
   (`governance.json` under the config dir) — not embedded in code. Hot-reloadable
   like `settings.json`.
4. **Audit is append-only.** Every action that crosses a layer boundary writes an
   event to `~/.agentmux/audit/YYYY-MM-DD.jsonl`. Never mutated, only rotated.
5. **Kill switches at every boundary.** Per agent, per instance, per layer. The
   widget's primary affordance is a big stop button at each scope.
6. **Quotas before kills.** Soft controls (rate limits, token budgets, message
   ceilings) run first; kills are the escape hatch.
7. **Human-in-the-loop for sensitive ops.** A configurable allowlist of
   tool-calls/jekts that require approval — surfaced as a notification with
   `approve / deny / always` actions.
8. **Identity provenance.** Every agent advertises (a) who spawned it, (b) the
   capability bundle at spawn time, (c) the parent instance's signature. Surfaced as
   the first column in every agent list.
9. **Drift detection.** When a peer's advertised capability set differs from what we
   expect (e.g., LAN peer claims `can.exec.tool:rm`), surface as a warning chip.
10. **Closed by default at boundary 3.** Internet-layer governance starts denying
    everything except an explicit allowlist. Network effects must be earned per agent.

What we deliberately don't do (yet): full OPA/Rego integration, mTLS between
instances, distributed consensus on policy. These are listed in Future Work.

---

## Widget design

### Placement

`defwidget@warden` — **Pinned** tier (per `CLAUDE.md`'s "every surfaced widget
is pinned" default; collapses to icon-only on narrow title bars). Icon:
`shield-halved` or `gavel`. Label: `warden`.

### Surface

A single pane, three vertically-stacked, collapsible sections — one per layer. Each
section has identical structure so the operator learns one row:

```
┌─ Warden ────────────────────────────────────────────────── [⏻ kill all]
│
│ ▼ HOST                              ● 3 agents · 0 alerts   [⏸ pause] [⏻]
│   ┌──────────────────────────────────────────────────────────────────┐
│   │ agent     identity        caps                  jekts/min  state │
│   │ ──────────────────────────────────────────────────────────────── │
│   │ agent1    user:asaf  ✓    jekt.host,send.host       12      active│
│   │ agent2    user:asaf  ✓    jekt.host,send.host        4      idle  │
│   │ forge1    user:asaf  ✓    jekt.host                  0      ready │
│   └──────────────────────────────────────────────────────────────────┘
│   policy: [allow host jekt ✓]  [require approval for tool:bash ✗]
│   audit:  last 5 events ▸
│
│ ▼ LAN                                ◆ 2 peers · 1 alert    [⏸ pause] [⏻]
│   ┌──────────────────────────────────────────────────────────────────┐
│   │ peer        host         version   agents  caps          state   │
│   │ ──────────────────────────────────────────────────────────────── │
│   │ ◆ desk-mac  agentx-asaf  0.38.4    2       jekt.lan ✓    healthy │
│   │ ◆ pi-lab    pi-builder   0.38.2    1       jekt.lan ⚠    drift   │
│   └──────────────────────────────────────────────────────────────────┘
│   policy: [LAN enrolled ✓]  [shared key set ✓]  [allow inbound jekt ✓]
│   audit:  last 5 events ▸
│
│ ▼ INTERNET                           ☁ AgentBus · disabled  [enable…]
│   AgentBus relay is not configured. Enable to allow cross-network
│   jekt/send via agentbus.asaf.cc. Closed by default.
│   [Set AGENTBUS_TOKEN] [Configure allowlist]
│
└──────────────────────────────────────────────────────────────────────
```

### Affordances per layer

| Action | L1 Host | L2 LAN | L3 Internet |
|--------|---------|--------|-------------|
| List agents/peers | ✓ | ✓ | ✓ (via AgentBus list) |
| Show identity provenance | ✓ | ✓ | ✓ |
| Show capability set | ✓ | ✓ | ✓ |
| Live jekt/msg counters | ✓ | ✓ | ✓ |
| Pause inbound | ✓ | ✓ | ✓ |
| Kill (force-stop agent / drop peer / disable layer) | ✓ | ✓ | ✓ |
| Edit policy | ✓ | ✓ | ✓ |
| Stream audit events | ✓ | ✓ | ✓ |
| Approval queue | ✓ | ✓ | ✓ |

Each row is right-click-actionable: `View jekt history`, `Revoke capability`,
`Quarantine`, `Open settings`.

### Top-level controls

- `⏻ kill all` — emergency stop. Drops all inbound across all layers, pauses every
  agent's PTY, writes a `kill-all` event to audit. Re-enable is manual.
- Per-layer `⏸ pause` — stop accepting new jekts/messages at that layer; existing
  in-flight finish.
- Per-layer `⏻` — full layer disable (no inbound or outbound).

---

## Architecture

### Frontend

- View key: `governance`. Registered like `swarm` (block-rendered, not a special-case
  like `settings`/`devtools`).
- Atoms in `frontend/state/governance.ts`:
  - `governanceLayersAtom` → `{ host, lan, internet }` summaries
  - `governancePolicyAtom` → live mirror of `governance.json`
  - `governanceAuditTailAtom` → last 50 audit events per layer (ring buffer)
- New RPCs:
  - `GetGovernanceSnapshot` → one-shot read of all three layers
  - `SetGovernancePolicy` → write to `governance.json` (validated, signed)
  - `GovernanceAction` → `{ scope, target, action }` e.g. `kill agent1`,
    `pause-layer lan`
  - `SubscribeGovernanceEvents` → WS topic streaming audit events
- Reuses existing components: `BlockTable`, `Chip`, `PolicyToggle` (new, small).

### Backend (agentmuxsrv-rs)

New module: `agentmuxsrv-rs/src/backend/governance/`

```
governance/
├── mod.rs              // facade
├── policy.rs           // GovernancePolicy struct, load/save/validate
├── enforcer.rs         // hooks into ReactiveHandler, MessageBus, LanDiscovery
├── audit.rs            // append-only JSONL writer + WS broadcaster
└── snapshot.rs         // GetGovernanceSnapshot assembly
```

Integration points (read-only at first; enforce in Phase 2):

| Call site | Purpose |
|----------|---------|
| `ReactiveHandler::inject_block` | Consult `enforcer::allow_jekt(agent, scope)` before PTY write |
| `MessageBus::send` | Consult `enforcer::allow_send(from, to, layer)` |
| `LanDiscovery` peer-discovered hook | Tag peer with `pending|enrolled|quarantined`, fire `lan.peer.discovered` audit |
| `agentbus_client::*` (cloud fallback) | Consult `enforcer::allow_cloud(agent)` |
| Any `kill_all` invocation | Write audit, broadcast `governance.kill_all` event |

### Policy file: `~/.agentmux/config/governance.json`

```jsonc
{
  "version": 1,
  "host": {
    "default": "allow",
    "agents": {
      "agent1": { "caps": ["jekt.host", "send.host", "tool:*"] },
      "forge1": { "caps": ["jekt.host"] }
    },
    "approval_required": ["tool:bash", "tool:rm"]
  },
  "lan": {
    "enabled": false,
    "shared_key": null,
    "allow_inbound": false,
    "allow_outbound": true,
    "trusted_peers": []
  },
  "internet": {
    "enabled": false,
    "agentbus_token_env": "AGENTBUS_TOKEN",
    "outbound_allowlist": [],
    "inbound_allowlist": []
  },
  "quotas": {
    "jekt_per_minute": { "host": 600, "lan": 120, "internet": 30 }
  },
  "audit": {
    "retain_days": 30,
    "dir": "~/.agentmux/audit"
  }
}
```

Schema lives at `schema/governance.json` (strict, `additionalProperties: false`,
loaded by `wconfig.rs`'s `ConfigWatcher`).

### Audit log

`~/.agentmux/audit/YYYY-MM-DD.jsonl`. One JSON object per line:

```jsonc
{ "ts": "2026-05-25T20:14:03Z",
  "layer": "lan",
  "kind": "jekt.allowed",
  "from": { "instance": "desk-mac", "agent": "agent3" },
  "to":   { "instance": "this",     "agent": "agent1" },
  "policy_rule": "lan.allow_inbound",
  "request_id": "..." }
```

Append-only at the API level (writes go through `audit::Writer`, which never
truncates and never rewrites). Rotation is by date file; retention is `audit.retain_days`.

---

## Implementation phases

### Phase 0 — Spec acceptance (this doc)
Review, refine, merge to `specs/`. Naming decided (Warden); icon selection
still open (`shield-halved` vs `gavel`).

### Phase 1 — Shell + L1 read-only
Depends on: nothing.

- Register `defwidget@warden` in `widgets.json`
- Wire `GetGovernanceSnapshot` RPC returning Host layer only
- Render shell with three sections; LAN/Internet sections stubbed as "requires
  networking (PR #lan-discovery)"
- Stream Host audit events
- No enforcement yet — observation only

### Phase 2 — L1 controls + policy file
Depends on: Phase 1.

- `governance.json` schema + watcher
- `SetGovernancePolicy` RPC
- `enforcer::allow_*` hooks in ReactiveHandler / MessageBus (Host layer)
- `GovernanceAction` RPC: `kill agent`, `pause host`, `kill all`
- Approval queue for `approval_required` capabilities — surfaced as a desktop
  notification + in-widget toast

### Phase 3 — L2 read-only
Depends on: `lan-awareness-and-embedded-jekt-api.md` Phase 1 (mDNS).

- LAN section lists discovered peers from `LanDiscovery`
- Per-peer drift detection (version, declared caps, last-seen)
- Per-peer audit (peer discovered, peer lost, peer drift)
- No control of remote peers yet

### Phase 4 — L2 control
Depends on: `lan-awareness-and-embedded-jekt-api.md` Phase 2 (embedded MCP) +
Phase 3 (LAN forward).

- Shared LAN key — enrollment ceremony (operator copies a key between instances)
- Inbound jekt/send enforcement at the LAN boundary
- Quarantine peer (drop all inbound from peer + don't forward to peer)
- Policy push: when this instance updates `governance.lan.*`, broadcast to
  enrolled peers (each peer applies it locally — no distributed consensus, just
  best-effort fan-out with version vectors)

### Phase 5 — L3 (Internet) read + control
Depends on: `lan-awareness-and-embedded-jekt-api.md` Phase 4 (cloud fallback).

- Enable/disable cloud relay from widget
- Outbound allowlist per agent
- Inbound allowlist (who can reach my agents from cloud)
- Token management surface (read from env, never display, only set/clear)

### Phase 6 — Polish
- Per-row history drawer
- Quota tuning UI
- Export audit log as CSV
- Right-click menu actions

---

## Open questions

1. **Widget vs first-class panel?** ~~Pinned status~~ now resolved — per
   `CLAUDE.md`'s "every surfaced widget is pinned" default, Warden ships
   pinned from the start.
2. **LAN enrollment UX.** mDNS discovers peers automatically, but enrollment (sharing
   a key) needs a flow. Options: QR code on host, copy/paste key, optimistic
   trust-on-first-use. TOFU is the lowest-friction path for dev; explicit key is
   better for shared/office LANs.
3. **Where does the policy author edit?** Two paths: in-widget toggle UI (limited)
   or `governance.json` in external editor (full). Both should work; the toggle UI
   covers the 80% case.
4. **Capability syntax.** `tool:bash` vs `tool.bash` vs `bash`. Pick one and stick
   with it — Kubernetes uses `verb:noun:resource`, MCP uses `tool.name`. Recommend
   matching MCP for consistency (`tool.bash`).
5. **Audit privacy.** Audit events may contain prompts. Default to redacting
   message bodies; allow opt-in full logging per agent for debugging.
6. **Quota enforcement granularity.** Per-minute is easy; per-agent-session may
   matter more. Start with per-minute global + per-layer; add per-agent later.
7. **Relationship to `swarm` widget.** Both touch agent control, but Swarm is
   workflow (task queues, lifecycle); Warden is policy (who can do what).
   Keep separate; cross-link in the UI ("Swarm tasks for agent1 →").
8. **Reagent / CI integration.** Should auto-approve/auto-kill rules be expressible
   here, or stay in reagent? Lean: Warden describes the boundary; reagent
   describes review behavior. Separate concerns.

---

## Acceptance criteria

- [ ] Phase 1 — `defwidget@warden` appears in More dropdown
- [ ] Phase 1 — Widget renders three layer sections, Host populated, LAN/Internet
      stubbed with informative message
- [ ] Phase 1 — Host section lists every agent with identity, caps, jekt/min rate,
      state
- [ ] Phase 1 — Audit events stream live for Host actions
- [ ] Phase 2 — `governance.json` validates against schema, hot-reloads on save
- [ ] Phase 2 — Kill-agent action stops PTY and writes audit event
- [ ] Phase 2 — Approval-required capability surfaces a notification with
      approve/deny/always
- [ ] Phase 2 — `⏻ kill all` halts every agent within 500 ms
- [ ] Phase 3 — LAN peers appear with drift indicators when mDNS is up
- [ ] Phase 4 — Quarantined peer cannot deliver inbound jekt, audit shows
      `lan.jekt.denied`
- [ ] Phase 5 — Internet layer toggle on/off works; cloud fallback respects
      allowlists
- [ ] Audit log retained per `audit.retain_days`, rotated daily

---

## Out of scope (future work)

- OPA/Rego policy language (current syntax is sufficient for v1)
- mTLS between instances (shared LAN key is the v1 model)
- Distributed consensus on policy (best-effort fan-out is fine for v1)
- Cross-org federation (a different problem; AgentBus is the model for now)
- A standalone "governance API" for external tools (could come later via MCP
  resource exposure)

---

## Key files (to be created)

| File | Role |
|------|------|
| `agentmuxsrv-rs/src/backend/governance/mod.rs` | Module facade |
| `agentmuxsrv-rs/src/backend/governance/policy.rs` | `GovernancePolicy` + load/save |
| `agentmuxsrv-rs/src/backend/governance/enforcer.rs` | Allow/deny hooks |
| `agentmuxsrv-rs/src/backend/governance/audit.rs` | Append-only JSONL writer |
| `agentmuxsrv-rs/src/server/governance.rs` | HTTP/WS endpoints |
| `agentmuxsrv-rs/src/config/widgets.json` | Add `defwidget@warden` |
| `schema/governance.json` | JSON Schema for `governance.json` |
| `frontend/app/widget/governance/` | React view + atoms |
| `frontend/state/governance.ts` | Atoms + RPC bindings |

## References

- `specs/lan-awareness-and-embedded-jekt-api.md` — 3-tier jekt cascade + LAN mDNS
- `specs/swarm-orchestration.md` — adjacent control plane (workflow, not policy)
- `specs/SPEC_SETTINGS_WIDGET.md` — widget bar conventions
- `specs/local-messagebus-architecture.md` — what we're governing on the inside
- CNCF: Kubernetes RBAC, NetworkPolicy
- OWASP zero-trust networking guidance
- MCP capability model (tool naming convention)

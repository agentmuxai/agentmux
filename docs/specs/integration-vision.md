# AgentMux Integration Vision

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  AgentMux  (desktop shell)                                  │
│                                                             │
│  Terminal │ Agent │ OpenClaw │ Settings │ DevTools          │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │ Terminal     │  │ Agent View   │  │ OpenClaw     │      │
│  │ (any shell)  │  │ (Claude/etc) │  │ Widget       │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                                             │
│  Inter-pane messaging (MCP tools, peer-to-peer):           │
│    send_message · inject_terminal · broadcast · list_agents │
└─────────────────────────────────────────────────────────────┘
          │                                    │
          │ WebView (localhost:18789)           │ MCP tools
          ▼                                    ▼
    OpenClaw Gateway                   AgentMux MCP Server
    - agent sessions                   - send_message (mailbox)
    - external messaging               - inject_terminal (direct)
    - memory / context engine          - broadcast_message
    - skills (ClawHub)                 - list_agents
    - channels (Telegram, Discord…)    - read_messages
          │
          ▼ (future)
    Forge
    - Docker container agent management
    - Multi-agent workspace orchestration
    - Dev-tools integration
    - Built on AgentMux + OpenClaw
```

---

## Messaging layers (complementary, not competing)

Three distinct layers, each solving a different problem:

| Layer | What it does | Implementation |
|---|---|---|
| **Inter-pane messaging** | Peer agents talk to each other inside AgentMux | AgentMux MCP tools (`send_message`, `inject_terminal`, `broadcast`, `list_agents`, `read_messages`) |
| **External channel messaging** | Agents reach humans via messaging apps | OpenClaw `message` tool (Telegram, WhatsApp, Discord, Slack, iMessage, Signal…) |
| **Agent spawning/streaming** | Parent agent spawns and controls child agents | OpenClaw ACP (Agent Communication Protocol) — hierarchical, parent→child |

A pane agent can use all three:
- AgentMux MCP → coordinate with AgentB in the next pane (peer-to-peer)
- OpenClaw `message` → report results to the user on Telegram
- OpenClaw ACP → spin up a subagent to handle a subtask

**AgentMux inter-pane MCP tools are not replaced by OpenClaw.** They serve the
desktop-local, peer-to-peer coordination use case that OpenClaw's messaging
systems don't cover.

---

## What OpenClaw replaces

| a5af/claw | OpenClaw equivalent |
|---|---|
| `claw agentx` — launch agent session | OpenClaw agent sessions with identity |
| `claw deploy` — push workspace files | OpenClaw skills / hooks |
| Per-agent GitHub CLI / AWS config | OpenClaw per-agent config |
| `claw status` | `openclaw status` / Control UI at localhost:18789 |
| mux/ject inter-agent messaging | **Not replaced** — AgentMux MCP tools handle this |

**a5af/claw → archive once OpenClaw covers daily agent-launch workflows.**
Track remaining gaps (Docker management, AWS profile isolation) as Forge backlog items.

---

## OpenClaw memory/context engine

Two-layer system worth integrating into AgentMux agent views:

### Layer 1: Compaction (in-session)

When a session grows too long, compaction summarizes old messages with the LLM
and replaces them with a summary block. Recent messages are kept verbatim.

Key mechanisms:
- **Adaptive chunking** — splits messages by equal token share; handles large
  individual messages by reducing chunk ratio dynamically
- **Identifier preservation** — explicit LLM instruction to never shorten/reconstruct
  UUIDs, hashes, tokens, URLs, file paths (LLMs silently hallucinate these in summaries)
- **Tool result stripping** — `toolResult.details` stripped before summarization
  (security: prevents prompt injection via tool outputs; efficiency: they're huge)
- **Orphaned tool_use repair** — when old chunks are dropped, `repairToolUseResultPairing`
  removes orphaned `tool_result` messages whose `tool_use` was in the pruned chunk
  (without this, Anthropic API throws `unexpected tool_use_id`)
- **Progressive fallback** — full summary → partial (skip oversized) → metadata note
- **Safety margin 1.2×** — compensates for `chars/4` tokenizer underestimation

Trigger: `afterTurn` hook checks token count; fires `compact()` if above threshold.

### Layer 2: Vector search (long-term, cross-session)

Per-agent SQLite at `~/.openclaw/memory/<agentId>.sqlite`.

- **Hybrid search** — 0.7 vector + 0.3 BM25 text, normalized weights
- **MMR (Maximal Marginal Relevance)** — diversity-aware retrieval; avoids
  returning near-duplicate results (λ=0.7 toward relevance, 0.3 toward diversity)
- **Temporal decay** — `e^(-t/halfLife)` scoring penalty; recent memories surface
  over older equally-relevant ones (default half-life: 30 days)
- **Embedding providers** — OpenAI, Gemini, Voyage, Mistral, Ollama, local; auto-detect + fallback
- **Delta sync** — re-indexes sessions only when >100KB or >50 messages changed
- **QMD backend** — optional MCP-based persistent daemon for low-latency heavy use

### ContextEngine interface (the abstraction)

Pluggable contract over both layers. Plugin slot:
`config.plugins.slots.contextEngine` — swap the entire engine.

```typescript
interface ContextEngine {
  bootstrap()           // load session + import history on startup
  ingest()              // store each message as it arrives
  ingestBatch()         // store a completed turn as a unit
  afterTurn()           // post-turn: persist, decide whether to compact
  assemble()            // build trimmed context under token budget
  compact()             // summarize + prune old turns
  prepareSubagentSpawn() // share context with child agents
  onSubagentEnded()     // clean up child context
  dispose()
}
```

### Integration priority for AgentMux agent views

| Mechanism | Priority | Reason |
|---|---|---|
| Compaction + identifier preservation | High | Sessions grow long; ID corruption is a real bug |
| Tool result stripping | High | Security + token savings |
| Orphaned tool_use repair | High | Silent failures after compaction without this |
| `ContextEngine` plugin interface | High | Clean abstraction to build against |
| Hybrid vector+BM25 search | Medium | Needs SQLite + embedding API |
| Temporal decay + MMR | Medium | Matters at scale |
| QMD backend | Low | Niche power-user feature |

---

## OpenClaw widget

See `docs/specs/openclaw-widget.md`.

If OpenClaw already running → WebView at `localhost:18789` (full live state).
Otherwise → install + setup flow.

---

## Forge (future)

Forge is the product layer built on AgentMux + OpenClaw:
- Docker container agent management (claw's container commands, not in OpenClaw)
- Multi-agent workspace orchestration
- dev-tools bundled as Forge utilities (`@a5af/file-tools`, `@a5af/workspace-health`, etc.)
- First-class skills for deployment, secrets, infra monitoring

Forge is NOT a separate app — it's a set of AgentMux widgets + OpenClaw skills
+ backend commands, surfaced through the existing widget system.

---

## Immediate next steps

1. **OpenClaw widget** — install/setup/dashboard view (see openclaw-widget.md)
2. **a5af/claw** — migrate daily workflows to OpenClaw; archive repo when done
3. **Context compaction** — implement Layer 1 (compaction) in AgentMux agent views
4. **Forge backlog** — track what claw did that OpenClaw doesn't cover (Docker, AWS profiles)

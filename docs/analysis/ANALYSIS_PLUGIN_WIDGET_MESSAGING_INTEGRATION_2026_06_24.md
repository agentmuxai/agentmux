# Analysis: Plugin System, Widget Architecture & Messaging Integration Sizing

**Date:** 2026-06-24  
**Status:** Reference / Decision Document  
**Covers:** Plugin system state, toolchain widget catalog, messaging integration sizing, installability options

---

## 1. Current Binary Baseline

| Build | Size |
|-------|------|
| `agentmux-srv` debug (current) | **43 MB** |
| `agentmux-srv` release (estimated) | **~10–15 MB** (no debug symbols, LTO enabled) |

Debug builds carry full DWARF symbols and no optimization — the 43 MB is not indicative of shipped size. Release with `opt-level = "z"` + `lto = "thin"` + `strip = true` typically produces 3–5× smaller binaries for a Rust app of this complexity.

---

## 2. What agentmux-srv Already Ships (Relevant Primitives)

This is the key fact for the messaging integration sizing question. The srv already compiles in every primitive needed to implement messaging bridges:

| Already in Cargo.toml | Used for messaging |
|-----------------------|-------------------|
| `tokio-tungstenite` (with `rustls-tls-webpki-roots`) | Discord Gateway WS, Slack Socket Mode WS |
| `reqwest` (with `rustls-tls`, `json`, `multipart`) | All REST API calls (Telegram, Discord, Slack, WhatsApp Graph API) |
| `axum` (with `ws`) | WhatsApp webhook HTTP receiver, Teams Bot Framework receiver |
| `serde_json` | All JSON encoding/decoding |
| `sha2` + `hex` | WhatsApp `X-Hub-Signature-256` validation |
| `keyring` | Storing bot tokens securely |
| `tokio` full | Async runtime for all bridge event loops |

**Conclusion: implementing all five messaging bridges using existing primitives adds zero new crate dependencies to the Rust binary.**

---

## 3. Messaging Bridge Sizes in agentmux-srv

### If implemented raw (using existing primitives only)

| Platform | New Rust LOC | New crates | Binary size increase |
|----------|-------------|------------|---------------------|
| Telegram | ~500 LOC | 0 | ~0.1–0.2 MB |
| Discord | ~700 LOC | 0 | ~0.15–0.3 MB |
| Slack | ~600 LOC | 0 | ~0.1–0.2 MB |
| WhatsApp (Cloud API) | ~800 LOC | 0 | ~0.15–0.25 MB |
| Teams | ~1,200 LOC | `jsonwebtoken` (JWT validation) | ~0.3–0.5 MB |
| **Total (all 5)** | **~3,800 LOC** | **0–1 new crate** | **~0.8–1.4 MB** |

Raw implementations are viable because the bridges are protocol state machines on top of existing HTTP/WS primitives. The logic is: connect → authenticate → event loop → normalize to AgentMux envelope → push to reactive bus.

### If using framework crates (teloxide, twilight-rs, serenity)

| Platform | Framework | New transitive crates | Binary size increase |
|----------|-----------|----------------------|---------------------|
| Telegram | `teloxide` | ~60–80 crates | +3–6 MB |
| Discord | `twilight-gateway` + `twilight-http` + `twilight-model` | ~20–30 crates | +1–3 MB |
| Discord | `serenity` | ~40–60 crates | +3–5 MB |
| Slack | No official Rust SDK | — | — (raw only) |
| WhatsApp | No Rust SDK | — | — (raw only) |
| Teams | `botbuilder` (deprecated) | Not recommended | — |

**Recommendation:** implement Telegram and Discord raw for the srv integration. The bridge protocol is simple enough that a framework adds more cost than convenience. Framework crates shine when you need the full feature surface (voice, caching, middleware); bridges only need event ingestion + REST send.

### WhatsApp Baileys (Node.js path)

Baileys runs as a Node.js subprocess (or in the Electron main process). It does not affect the Rust binary size. Its Node.js footprint:

| Component | Size on disk |
|-----------|-------------|
| `@whiskeysockets/baileys` package | ~15–25 MB (includes libsignal, crypto deps) |
| Runs as | Electron main process or Node subprocess |
| Rust binary impact | 0 |

### Slack (if Node.js path preferred)

| Component | Size |
|-----------|------|
| `@slack/socket-mode` + `@slack/web-api` | ~8 MB |
| Rust binary impact | 0 |

---

## 4. Plugin / Widget System — Current State

### What exists today

| Component | Status | Notes |
|-----------|--------|-------|
| `widgets.json` (server config) | ✅ Shipped | 9 built-in widgets defined here |
| `block-registry.ts` (frontend) | ✅ Shipped | Static compile-time map, `registerBlockView()` hook exists |
| `registerBlockView()` hook | ✅ Available | Runtime-callable; used for external widget dynamic registration |
| `widget-catalog.ts` | ✅ Spec ready (not yet impl.) | External widget catalog (ComfyUI, JupyterLab, Grafana, n8n, etc.) |
| `toolchain-modal.tsx` | ✅ Spec ready (not yet impl.) | Toolchain Manager modal with External Widgets section |
| `widget.health` RPC | ✅ Spec ready | HTTP health check for localhost widget servers |
| `widget.install` RPC | ✅ Spec ready | pip/npm install streaming |
| Plugin manifest format | ❌ Not designed | No per-plugin `manifest.json` |
| Plugin directory scanning | ❌ Not built | No `~/.agentmux/plugins/` discovery |
| Plugin marketplace | ❌ Not built | Referenced as future work |
| Third-party plugin API | ❌ Not spec'd | `RESEARCH_PLUGGABLE_WIDGET_API_2026_06_21.md` not written |

### The two-tier widget model (existing architecture)

**Tier 1 — Built-in widgets** (`widgets.json` + compiled into Rust binary + `block-registry.ts`)  
Agent, Swarm, Browser, Editor, Terminal, Sysinfo, Drone, Help, Warden.  
These are part of the application. Zero setup, always available.

**Tier 2 — External widgets** (`widget-catalog.ts` + health-check + browser embed)  
ComfyUI, JupyterLab, Open WebUI, LangFlow, Flowise, MLflow, n8n, Grafana, Qdrant, Portainer.  
These run as independent localhost HTTP servers. AgentMux detects them via health check and embeds them in a CEF browser pane. No Rust binary changes needed — the browser widget handles the embed.

---

## 5. Toolchain System

### SPEC_TOOLCHAIN_MANAGER_2026-06-15.md

Solves the "GUI-launch PATH stripping" problem (macOS launchd gives agentmux-srv `/usr/bin:/bin:/usr/sbin:/sbin`, so nvm/Homebrew CLIs not found). Deliverables:

- `resolve_login_path()` in `agentmux-common`: captures full login-shell PATH + well-known dirs
- Called at host startup before srv spawns — one enrichment, everything downstream inherits
- **Toolchain Manager modal** (hamburger → Toolchain Manager):

```
┌─ Toolchain Manager ─────────────────────────────────────────┐
│  Environment     PATH: login-shell ✓  /opt/homebrew/bin … │
│  Core Tools      Node 22 ✓  npm 10 ✓  Git 2.44 ✓  Docker ✓│
│  Agent CLIs      Claude 2.1 ✓  Gemini ✓  Codex …          │
│  External Widgets                                            │
│    ComfyUI    [Installed ✓] [Running ✓]  [Open Pane]        │
│    JupyterLab [Installed ✓] [Not running]  [Launch] [Open]  │
│    Grafana    [Not installed]            [Install]           │
└─────────────────────────────────────────────────────────────┘
```

### SPEC_TOOLCHAIN_MANAGER_EXTERNAL_WIDGETS_2026_06_22.md

Extends the modal with external widgets. Each widget entry in `widget-catalog.ts` specifies:
- `id`, `label`, `icon`
- `healthCheckPath` + `defaultPort` (for detection)
- `embedPath` (URL path to embed in CEF pane)
- `install`: pip/npm/manual + package names
- `requires`: prerequisite core tools (e.g., `["python"]`)
- `cliCommand`: binary to detect on PATH

Detection flow: `ResolveCliCommand()` → `widget.health` RPC → `registerBlockView()` → pane available.

---

## 6. Where Messaging Integrations Fit

### Pane layer — zero new code needed

Each messaging app is a pre-configured browser widget (CEF pane). Five new `widgets.json` entries using `"view": "browser"`:

```json
"defwidget@discord":   { "blockdef": { "meta": { "view": "browser", "url": "https://discord.com/app" } } }
"defwidget@slack":     { "blockdef": { "meta": { "view": "browser", "url": "https://app.slack.com/" } } }
"defwidget@telegram":  { "blockdef": { "meta": { "view": "browser", "url": "https://web.telegram.org/" } } }
"defwidget@whatsapp":  { "blockdef": { "meta": { "view": "browser", "url": "https://web.whatsapp.com/" } } }
"defwidget@teams":     { "blockdef": { "meta": { "view": "browser", "url": "https://teams.microsoft.com/" } } }
```

Cost: **~50 lines in widgets.json**. Drag/drop, layout, multi-pane, all inherited from the browser widget. User sees and uses the real app interface unchanged.

### Bridge layer — background daemons for agent routing

Bridges let the AgentMux agent participate in conversations visible in the CEF pane. Agent messages appear as the bot/integration account in the native UI.

These bridges are a **new tier** beyond what the existing external widget catalog supports. External widgets are HTTP servers you embed; bridges are background daemons with no HTTP UI surface. The catalog model needs a `daemon: true` flag to accommodate this.

---

## 7. Three-Tier Widget Architecture (Proposed)

| Tier | What | Examples | Integration method |
|------|------|----------|-------------------|
| **1 — Built-in** | Compiled into srv + in widgets.json | Agent, Warden, Drone, Browser | Always available |
| **2 — External UI** | Localhost HTTP server embedded in CEF | ComfyUI, Grafana, JupyterLab, n8n | widget-catalog.ts + health check |
| **3 — Background daemon** | Background process, no HTTP UI, agent integration | Telegram bridge, Discord bridge, Slack bridge | widget-catalog.ts + `daemon: true` |
| **Future — Plugin** | Installable package (.amx or similar) | Community plugins | Plugin API (not yet designed) |

Tier 3 extends the existing external widget catalog pattern with one new field:

```typescript
interface ExternalWidget {
    id: string;
    label: string;
    icon: string;
    daemon?: true;           // NEW: no embed URL, background process only
    embedPath?: string;      // existing: for Tier 2 HTTP-server widgets
    healthCheckPath?: string;
    bridgeType?: 'rust' | 'node'; // which runtime hosts this daemon
    ...
}
```

---

## 8. Installability Options by Platform

### Option A — Built-in (Tier 1) for Telegram + Discord

Bridges compiled into agentmux-srv. Cost: ~1,200 LOC Rust, 0 new crates, ~0.3–0.5 MB binary increase. Users with no interest in messaging pay a small binary size tax. Simplest path.

**Best for:** Telegram, Discord (small, no public URL, no external deps, Rust primitives already present)

### Option B — Toolchain catalog extension (Tier 3) for Slack + WhatsApp Baileys

Slack bridge runs as a Node.js subprocess (no official Rust SDK; @slack/socket-mode is the right choice). WhatsApp Baileys runs as a Node.js subprocess. These map naturally to the external widget catalog extended with `daemon: true`.

```typescript
{
    id: "slack-bridge",
    label: "Slack",
    daemon: true,
    bridgeType: 'node',
    requires: ["node"],
    install: { method: "npm", packages: ["@slack/socket-mode", "@slack/web-api"] },
    cliCommand: null,  // no standalone CLI; spawned directly
}
```

**Best for:** Slack (Node.js ecosystem), WhatsApp Baileys (Baileys is Node.js-only)

### Option C — Tier 3 with tunnel management for WhatsApp Cloud API + Teams

These require a public HTTPS URL. Tunnel management (cloudflared subprocess, Dev Tunnels) is needed alongside the bridge. More setup friction; should be Tier 3 optional with clear setup guidance in Toolchain Manager.

**Best for:** Users who want the official/compliant path for WhatsApp or already have an M365 tenant for Teams

### Option D — Future plugin system

Once `RESEARCH_PLUGGABLE_WIDGET_API_2026_06_21.md` is written and implemented, every messaging bridge becomes an installable `.amx` package (or similar). Community can build bridges for Signal, Matrix, iMessage, etc. without touching the AgentMux repo.

---

## 9. Recommended Integration Plan

### Immediate (pane-only, one PR)

Add 5 `widgets.json` entries. 50 lines. Zero risk. Users can open any messaging app in a pane immediately. No bridge, no agent integration yet — just the native UI in a tile.

- Discord: `https://discord.com/app`
- Slack: `https://app.slack.com/`
- Telegram: `https://web.telegram.org/`
- WhatsApp: `https://web.whatsapp.com/`
- Teams: `https://teams.microsoft.com/`

### Phase 1 (bridges, P1 — Telegram + Discord)

Build raw Rust bridges in agentmux-srv under `src/messaging/`:

```
agentmux-srv/src/messaging/
├── mod.rs              # BridgeTrait + BridgeHealth
├── telegram/
│   ├── mod.rs
│   ├── poll.rs         # getUpdates long-polling loop
│   ├── send.rs         # sendMessage REST calls
│   └── types.rs        # Update, Message, InlineKeyboard
└── discord/
    ├── mod.rs
    ├── gateway.rs      # WS connect, identify, heartbeat, resume
    ├── rest.rs         # POST /channels/{id}/messages
    └── types.rs        # GatewayEvent, Message, Embed
```

~1,200 LOC Rust, 0 new crates, ~0.3–0.5 MB binary increase. Settings UI: token input, channel/chat ID, allowed IDs allowlist.

### Phase 2 (bridges, P2 — Slack + WhatsApp Baileys)

Extend toolchain catalog with `daemon: true` Node.js bridges. Slack uses `@slack/socket-mode`. WhatsApp uses `@whiskeysockets/baileys`. Both spawned as managed Node.js subprocesses. No Rust binary impact.

Requires: `daemon: true` field on `ExternalWidget` type + subprocess lifecycle management in Toolchain Manager.

### Phase 3 (bridges, P3 — WhatsApp Cloud API + Teams)

Both require tunnel management. Implement `TunnelManager` (cloudflared subprocess for WhatsApp, Dev Tunnels for Teams). These are the most complex integrations; serve enterprise users with existing M365 tenants or business WhatsApp accounts.

### Future (plugin system)

Design and implement `RESEARCH_PLUGGABLE_WIDGET_API_2026_06_21.md`. Community can then publish bridges for Signal, Matrix, iMessage, WeChat, etc. as installable packages.

---

## 10. Summary: Everything in One View

```
AgentMux Widget/Plugin Tiers
├── Tier 1 — Built-in (compiled into srv)
│   ├── Agent, Swarm, Browser, Editor, Terminal      ← shipped
│   ├── Sysinfo, Drone, Help, Warden                 ← shipped
│   ├── Telegram bridge                              ← Phase 1 (this work)
│   └── Discord bridge                               ← Phase 1 (this work)
│
├── Tier 2 — External UI (Toolchain Manager → External Widgets)
│   ├── ComfyUI, JupyterLab, Open WebUI              ← spec ready
│   ├── LangFlow, Flowise, MLflow, n8n               ← spec ready
│   ├── Grafana, Qdrant, Portainer                   ← spec ready
│   └── [5 messaging pane entries]                   ← immediate (widgets.json only)
│       discord.com/app, app.slack.com, web.telegram.org,
│       web.whatsapp.com, teams.microsoft.com
│
├── Tier 3 — Background daemon (Toolchain Manager → daemon:true extension)
│   ├── Slack bridge (Node.js, @slack/socket-mode)   ← Phase 2
│   ├── WhatsApp Baileys (Node.js, baileys)          ← Phase 2
│   ├── WhatsApp Cloud API + Cloudflare Tunnel       ← Phase 3
│   └── Teams Bot + Dev Tunnel                       ← Phase 3
│
└── Future — Plugin packages (.amx)
    ├── Community messaging: Signal, Matrix, iMessage ← requires plugin API
    └── Community tools: anything                     ← requires plugin API
```

### Binary size budget (all messaging work)

| Phase | Rust binary delta | Node.js disk |
|-------|-----------------|--------------|
| Immediate (pane entries) | 0 | 0 |
| Phase 1 (Telegram + Discord raw) | **+0.3–0.5 MB** | 0 |
| Phase 2 (Slack + WhatsApp Baileys) | 0 | +23–33 MB on disk (Node deps, not in binary) |
| Phase 3 (WhatsApp Cloud + Teams) | +0.3–0.5 MB | 0 |
| **Total Rust binary impact** | **~0.6–1.0 MB** | — |

All Rust bridges implemented raw using existing `tokio-tungstenite` + `reqwest` + `axum` + `serde_json` primitives. Zero new crate dependencies for Phases 1 and 3. The entire messaging integration adds under **1 MB** to the release binary.

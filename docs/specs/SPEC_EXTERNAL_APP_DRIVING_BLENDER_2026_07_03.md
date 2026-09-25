# Spec: Integrated External App Driving — Blender as the Flagship Case

**Date:** 2026-07-03
**Author:** AgentY (original), merged with Geometry Nodes pane addendum by Claude (agent1-06309), 2026-08-10
**Status:** proposed — nothing Blender-specific has shipped: no Blender entry exists in `frontend/app/view/mcp/mcp-preload-catalog.ts`, no Blender skill, no Blender code anywhere in `agentmux-srv/`, `agentmux-mcp/` or `frontend/app/`. Re-verified against main 2026-09-25 (see the note below). The generic machinery Phase 1 plugs into has shipped (MCP Server primitive, preload catalog); only the Blender entry itself has not.
**Type:** Research + proposal. This is the design doc a Phase-1 implementation PR should follow.
**Purpose:** Scope a new capability class for AgentMux — letting an agent *drive* a third-party GUI desktop application (not just a CLI or a web page) — using Blender as the concrete worked example, and show exactly where it plugs into the existing Armory/primitive model.

---

> **Re-verified against main on 2026-09-25.** This doc was written 2026-07-03
> (merged 2026-08-10) and landed in the repo only now; several facts it states
> had drifted. What changed in this pass (research content that is still true is
> untouched):
>
> - **Status line added** (`proposed`); dead `specs/` paths repaired (the
>   top-level `specs/` tree was folded into `docs/specs/` — see
>   `docs/specs/README.md`). `SPEC_MCP_HOTLOAD_2026_07_03.md` (cited in §4.7)
>   exists nowhere in the repo or its git history — the citation is now stated
>   as unresolvable rather than pointed at a dead path.
> - **§2 / §4.4: "no computer-use / window-capture" is no longer true.** One-shot
>   capture of a foreign window now exists (`CaptureWindow`, `DiscoverWindows`,
>   capture-tier model). Driving a foreign window (input injection) still does
>   not; `compuse` (`docs/specs/computer-use-pane.md`) is still unbuilt.
> - **§4.5: `PtyShell*` MCP tools now exist**; `Shell` itself is still pipe-based.
> - **§4.1 / §4.2: the MCP/Skill primitives grew a catalog tier and Bundle-level
>   binding** since this was written; the "no Bundle-level binding yet" claim is
>   removed.
> - **§4.3: the pane registry was rewritten** (`pane-tab-registry.ts`); the
>   stale line-number citation is replaced by a description of the current shape.
> - **§5 Phase 1: now trivially small.** The flat preload-catalog mechanism
>   shipped for Ableton Live and TouchDesigner; a Blender entry is one more
>   object literal (still unbuilt). The §4.8 Geometry Nodes pane is unaffected
>   and unbuilt (and was rejected, §0.1).
> - **Line-number citations** that could not be cheaply re-verified were replaced
>   with symbol/file names throughout.
> - **Not re-verified:** the external research (Blender ecosystem, §3), the
>   `blender-mcp` version pin, and §6.1's description of muxbus tiers beyond the
>   LAN tier (see §6.1).

---

## 0. Status note: provenance and merge history

This file was written by AgentY on 2026-07-03 but never landed in the repo — it
was committed into a path-mangled stray checkout in AgentY's own workspace, so
`git log`/full-repo grep correctly found nothing (confirmed via commit history,
branch search, and the GitHub code index — see the investigation in issue
[#2504](https://github.com/agentmuxai/agentmux/issues/2504)). Two merged specs
(`SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`,
`REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md`) cited this file's
§3.1, §3.2, §3.3, §4.1, §4.2, §4.6, §4.7, §5, and §7 open question 1 anyway —
those citations resolve correctly against the section numbering below, which is
AgentY's original, unchanged.

Two things were added in this merge pass, both new content, not corrections to
the original:

1. **§4.8 — Geometry Nodes editor: a synthetic, MCP-backed pane.** The original
   doesn't address AgentMux showing Blender's Geometry Nodes editor specifically
   — this section does, building on §4.3/§4.4's existing embedding-vs-live-view
   analysis rather than duplicating it.
2. **§3.3 live-verification update.** AgentY drove this exact bridge over a real
   session on 2026-08-09/10 and confirmed several facts §3.3 originally flagged
   as ported/assumed. Updated inline, marked with the verification date.

Everything else is AgentY's original text, unedited.

### 0.1 Scoping decision (2026-08-10): tighten Phase 1, defer the rest

After weighing cost against value, Phase 1 is narrowed to the two cheapest,
highest-value pieces — the Phase 0 headless-batch recipe and a **flat** MCP
catalog entry (the shape Ableton/TouchDesigner actually shipped, not the
heavier `AppConnector` machinery §4.7 designs) plus its companion Skill. The
heavier and more speculative pieces are explicitly deferred or rejected, not
silently dropped — each is annotated in place below rather than deleted, so
the reasoning stays available if one of them gets re-proposed later:

- **§4.6 (Toolchain "Connectors" pane) — deferred.** Unbuilt scope, not proven
  necessary. Two real precedents (Ableton, TouchDesigner) shipped without it.
- **§4.7 (`AppConnector`/`ConnectorSetupStep` schema) — deferred.** Same
  reasoning; §7 open question 3 (which asked exactly this) is now resolved
  against building it for a first connector.
- **§4.8 (Geometry Nodes synthetic pane) — rejected, not being built.**
  Explicit product decision: AgentMux is not building a Geometry Nodes pane.
  §4.8's analysis stays in the doc as the record of what was considered and
  why it wasn't a live-embed candidate, in case the underlying "synthetic
  node-graph from MCP introspection" pattern is useful for a different pane
  later — but no work should be scoped against it for Blender.
- **§6 (remote/cross-machine driving) — no change, already deferred** in the
  original text; restated here for completeness, not newly cut.

Phase 1, as actually scoped now (and unchanged by the 2026-09-25 re-verification — the catalog mechanism has since shipped for Ableton Live and TouchDesigner, so this is now just one more entry): one entry in `mcp-preload-catalog.ts`
(`id, name, transport, config, prereqNote, riskNote, docsUrl` — see §5), one
companion Skill (§4.2), and the version-pin verification pass §3.3/§7 already
flags as outstanding. See §5 for the rewritten Phase 1/2/3 plan.

---

## 1. Problem statement

AgentMux agents already drive two categories of external surface:

- **Web apps**, via the `browser` pane (CEF-embedded Chromium — see §4.3).
- **CLIs/servers**, via the `term` pane (real PTY), the pipe-based `Shell`/`ShellInput`/`ShellStatus` MCP tools (`agentmux-srv/src/backend/shell_node.rs`), and — since 2026-09 — the real-PTY `PtyShell*` MCP tools (§4.5).

Neither covers a **native, stateful, GUI desktop application** with its own scripting surface — Blender, but also things like a DAW, a CAD tool, or Figma's desktop app. These apps aren't a page you can point CEF at, and driving them well is not simply "open a bigger shell." This spec asks: what's the right integration shape, and does Blender's own ecosystem already show the answer?

Short answer: **yes** — Blender's MCP ecosystem has converged on a pattern in 2026, and it maps cleanly onto AgentMux's existing **MCP Server primitive** (`docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md`). No new primitive is required to make it *work*; a new pane type is optional and should come later, for supervision, not function.

---

## 2. How "driving a GUI app" is best done in 2026 (general research)

Three modalities exist industry-wide, and the 2026 consensus is that they're complementary, not competing:

1. **Native scripting API / app-specific bridge** (what Blender uses: `bpy`). Fastest, most reliable, but per-app — you write it once per application.
2. **Accessibility tree / UI Automation** (Windows UIA, macOS AX). Structured, app-agnostic, but coverage varies per app/toolkit and is Windows/macOS-asymmetric.
3. **Pure "computer use"** (screenshot in → click-coordinate out). Universal — works on literally any GUI — but brittle under DPI scaling, multi-monitor layouts, and app-drawn (non-native) widgets.

Microsoft's UFO² architecture is the clearest 2026 reference point: a `HostAgent` decomposes a task, hands each sub-task to an app-specialized `AppAgent` that prefers the app's native API, and falls back to a hybrid UI-Automation + vision pipeline only when no native hook exists — run inside a picture-in-picture virtual desktop so the human can watch without losing their own session. That "prefer native API, fall back to vision, supervise via a side view rather than full takeover" shape is the one this spec adopts for AgentMux.

**Implication for AgentMux:** don't build a generic screenshot/click-coordinate driver first. Blender exposes a rich native API (`bpy`) — use it. Save vision/computer-use for apps with no scripting surface, as a later, separate spec.

> **State of the platform, as of 2026-09-25 (the original 2026-07-03 text said none of this existed):** the *screenshot* leg of modality 3 now exists in a narrow form — `CaptureWindow`/`DiscoverWindows` (`agentmux-mcp/src/window_capture.rs`, built on the `xcap` crate) give an agent a one-shot PNG of any window on the machine, Blender's included, governed by a capture-tier model with audit logging (`docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`, status `active`, Phase 1 implemented). The *click/type* leg does not: `UIScreenshot`/`UIClick`/`UIQuery` reach only AgentMux's OWN DOM (own pane plus shared chrome), and nothing synthesizes input into a foreign window. The `compuse` pane (§4.4) that would is still a proposal. Whether `xcap` returns usable pixels for Blender's GPU-drawn window has not been tested by this spec.

Sources: [Zylos: Computer Use and GUI Agents in 2026](https://zylos.ai/research/2026-02-08-computer-use-gui-agents/), [Zylos: GUI AI Agents & Computer Use, State of the Art 2025-2026](https://zylos.ai/research/2026-01-09-gui-ai-agents-computer-use), [Microsoft: Where AI meets GUI](https://medium.com/data-science-at-microsoft/where-ai-meets-gui-an-overview-of-computer-using-agents-3085d3bbe332), [API Agents vs. GUI Agents (arXiv 2503.11069)](https://arxiv.org/pdf/2503.11069).

---

## 3. Case study: Blender

### 3.1 The architecture Blender's own ecosystem has converged on

Every serious 2026 implementation (the Blender Foundation's own official MCP server, plus community ones like `djeada/blender-mcp-server`, `glonorce/Blender_mcp`, `PatrykIti/blender-ai-mcp`) uses the same three-tier shape:

```
MCP client (AgentMux agent) ──stdio/HTTP──► MCP server (Python process) ──TCP/localhost──► Blender add-on (runs inside bpy)
```

Key implementation facts that any AgentMux integration must respect:

- **Thread safety is non-negotiable.** The add-on's TCP listener runs on a background thread; it must never call `bpy` directly. Every implementation marshals actual Blender API calls onto the main thread via `bpy.app.timers` (a thread-safe producer/consumer queue: socket thread enqueues, timer callback drains and executes). Skipping this crashes Blender (`EXCEPTION_ACCESS_VIOLATION`).
- **Curated macro tools beat raw script generation.** Early "AI + Blender" demos let the LLM emit raw `bpy` scripts. 2026 production servers instead expose a layered tool set: small atomic tools (mostly hidden), **macro tools** as the LLM-facing surface (`create_object`, `assign_material`, `render_scene`, `get_scene_graph`), and bounded **workflow tools** for multi-step tasks — because raw operators are context-sensitive (fail depending on active object/mode/selection), drift across Blender versions, and give poor error feedback.
- **Data API over Operators for stability-sensitive work** (e.g. physics/fluid sims) — `bpy.ops.mesh.primitive_*_add` can destabilize view-layer state in a live session; direct data-API calls or a separate headless `blender -b` subprocess are preferred for heavy bakes.
- **Transport choice is local-vs-remote, not a hard requirement.** Local single-user (AgentMux's actual case) → stdio-launched bridge + localhost TCP to the add-on is simplest. HTTP/SSE only matters for remote/streaming scenarios AgentMux doesn't need yet.

### 3.2 Security — this is the load-bearing constraint

The Blender Foundation's own docs state it plainly: their MCP server **executes LLM-generated code in Blender with no guardrails**, and recommend running it in a VM or on a machine with no access to sensitive data. This is not a hypothetical risk — `bpy` has full filesystem and network access from inside Blender's process.

This directly shapes the AgentMux integration (see §5, Phase 1): expose curated macro tools by default; gate any raw-Python-execution tool behind an explicit, scarier opt-in — the same posture AgentMux already takes with `FORBIDDEN`-style checks in the MCP primitive backend (`agentmux-srv/src/server/app_api/mcp.rs`: the `FORBIDDEN:` reserved-name check on `mcp.upsert` and the global-server mutation/delete guards).

Sources: [Blender.org: MCP Server](https://www.blender.org/lab/mcp-server/), [djeada/blender-mcp-server](https://github.com/djeada/blender-mcp-server), [glonorce/Blender_mcp](https://github.com/glonorce/Blender_mcp), [PatrykIti/blender-ai-mcp](https://github.com/PatrykIti/blender-ai-mcp), [Eigent: Claude for Creative Work — Blender MCP Connector Guide 2026](https://www.eigent.ai/blog/claude-blender-mcp).

### 3.3 Operational reality: this is NOT a "launch Blender for me" integration

Two load-bearing facts, verified against `ahujasid/blender-mcp` (the reference implementation) and its derivatives — these change what Phase 1's setup flow has to be, not just a footnote:

- **The MCP bridge does not launch Blender.** The bridge process (`uvx blender-mcp` or equivalent — what AgentMux's stdio `command`/`args` config spawns, §4.1) is a thin client that connects *out* to a socket the Blender add-on opens. **A human must already have Blender open, with the add-on's socket server started**, before the bridge can connect — either by clicking "Start MCP Server" in the add-on's sidebar panel, or via an "auto-start on launch" preference some newer add-ons expose. If Blender isn't already running, binding/using the MCP server just times out. This is the opposite of the Discord/Slack/Telegram "browser preset" pattern (§4.3) — there, opening the pane *is* opening the app; here, opening the pane can do nothing until a human has separately opened Blender.
- **`blender --background` is a dead end for this integration.** The whole design depends on Blender's live GUI event loop periodically draining a command queue via `bpy.app.timers` (§3.1); headless/background mode doesn't run that loop the same way, so queued commands would simply never execute. (Linux escape hatch some projects use: a virtual display via `xvfb-run` — still a real event loop, just not an on-screen one.) This means Phase 0's `Shell`-driven headless recipe (§5) and the interactive MCP path (§5 Phase 1) are **not** two tiers of the same mechanism — they're mutually exclusive: headless batch jobs use plain `blender -b --python script.py` with no add-on involved at all; interactive MCP driving requires a full, visible, running Blender instance. A user can't "upgrade" a headless run into an interactive one.
- **The window needs to stay alive and un-minimized, not necessarily focused.** `bpy.app.timers` polling is throttled by Blender's own redraw/event loop, which the OS can slow down when the window is minimized or unfocused for a while — the widely-reported symptom is a queued command sitting stuck until the window gets an input event or regains focus ("first command doesn't go through," "had to click into the window"). A second, sharper issue: operators invoked from inside a timer don't get a fully-populated `bpy.context` (window/screen aren't set) unless the window is the active one, so some operators can misbehave independent of the throttling. Net guidance for users: **keep the Blender window open and visible** (not minimized) while an agent is driving it; don't rely on it working reliably in the background.

  > **Live-verification update (2026-08-10, AgentY):** driven directly against a
  > real install — **Blender 5.0.1 + "Blender MCP" addon `bl_info` v1.2**, addon
  > listening on **TCP 9876** localhost after "Connect to MCP Server" in the
  > N-panel, confirmed at the socket level (raw JSON `{"type": <cmd>, "params":
  > {...}}` → `{"status": "success", "result": ...}`). Tools verified working:
  > `get_scene_info`, `get_object_info`, `get_viewport_screenshot` (params
  > `max_size`, `filepath`, `format`; captures the 3D viewport area only), and
  > `execute_code` — ~95% of real work in that session routed through
  > `execute_code` (mesh analysis, node-group construction, `bpy.ops` renders).
  > Over several dozen `execute_code` calls across two days with no modal
  > dialogs open, **this section's threading caveat didn't bite once** — zero
  > first-command retries or timer stalls observed. The mechanism is real (per
  > the architecture in §3.1), but Phase 1's troubleshooting copy should read
  > "occasionally, after connecting" rather than "expect this every time."
  > **Still unverified:** the exact `uvx blender-mcp` *server package* version to
  > pin (§5 Phase 1, §7 open question 1) — this session drove the addon socket
  > directly, not through the pip/uvx package, so the addon side is now
  > confirmed but the stdio-bridge side's version pin still needs its own
  > hands-on pass before it ships as fact in a catalog entry's `config`.

**Implication for §4.4's "let Blender keep its own OS window" recommendation:** that's still right, but it's not merely a UX nicety — an interactive Blender MCP session functionally *requires* that window to stay visible and non-minimized for reliable delivery. Phase 1's UX should make Blender's connection state visible in the agent pane (e.g. "Blender: connected" / "Blender: not responding — is the window minimized?") rather than let a silently-stalled command queue look like the agent hung.

Sources: [ahujasid/blender-mcp](https://github.com/ahujasid/blender-mcp), [DeepWiki: ahujasid/blender-mcp Quick Start Guide](https://deepwiki.com/ahujasid/blender-mcp/1.1-installation-and-setup), [Oli97430/blender-mcp-addon](https://github.com/Oli97430/blender-mcp-addon), [Blender docs: bpy.app.timers](https://docs.blender.org/api/current/bpy.app.timers.html), [Blender dev tracker T79690: Context in Timers](https://developer.blender.org/T79690).

---

## 4. Where this plugs into AgentMux's existing architecture

AgentMux already has a primitive built for exactly this job: the **MCP Server primitive** (Phase 1 of the composable-agent-model refactor, shipped — `docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md` §2). Originally verified against the backend on 2026-07-03; re-checked against main 2026-09-25 (the primitive has grown a catalog tier and Bundle-level binding since — noted inline):

### 4.1 MCP Server primitive (the plug point)

- **Schema** (`McpServer` in `agentmux-srv/src/backend/storage/mcp_servers.rs`): `id`, `name`, `transport` (`"stdio"` default, or SSE/HTTP), `config` (JSON string — for stdio: `command`/`args`/`env`; for SSE: `url`/`headers`), `is_global`, timestamps. Unchanged since this spec was written.
- **Storage**: `db_mcp_servers` (the server definition) + `db_agent_mcp_ref` (agent-to-server binding, many-to-many). Two things changed after 2026-07-03: catalog rows now live in the identity store, so the ref tables no longer carry a foreign key to the local `db_mcp_servers` (migration `m0032_drop_catalog_fk_from_ref_tables`, `docs/specs/SPEC_DURABLE_BINDINGS_2026_09_10.md` §5.4); and a parallel `db_bundle_mcp_ref` table binds servers to a Bundle. An agent's effective servers (`effective_mcp_servers`) = its own bound rows + its Bundle's referenced rows + every `is_global` row.
- **App API** (`agentmux-srv/src/server/app_api/mcp.rs`, command names in `agentmux-srv/src/backend/rpc_types/commands.rs`): the original agent-scoped `mcp.list` / `mcp.get` / `mcp.upsert` / `mcp.delete` / `mcp.bind` / `mcp.unbind`, plus (added since) `mcp.probe` and an Armory-facing catalog tier — `mcp.catalog.list` / `.upsert` / `.delete` / `.probe` / `.bind` / `.unbind` / `.list_for_agent` and the bundle-scoped `.bind_to_bundle` / `.unbind_from_bundle` / `.list_for_bundle` / `.upsert_for_bundle`. Non-global servers can only be mutated/deleted by an agent directly bound to them; an agent may only *bind* a server that is either global or already its own (the `FORBIDDEN: can only bind global MCP servers` guard in `mcp.bind`) — i.e. you can't bind-and-thereby-read another agent's private server config.
- **Binding is now agent-level or Bundle-level.** (The original text said no Bundle-level binding existed, citing the v1 non-goals of `docs/specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`; Bundle binding has since shipped — `mcp.catalog.bind_to_bundle`, per `docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`.) Either way binding keys off the agent *definition* or Bundle id, not a pane instance. A Blender MCP Server can therefore be bound per-agent or shipped in a Bundle.
- **Delivery**: refs are resolved into a concrete `.mcp.json` only at pane-open time — `agentmux-srv/src/server/app_api/agent_open.rs` calls `effective_mcp_servers` then `agent_config::build_mcp_config_from_refs` (`agentmux-srv/src/backend/agent_config.rs`); the synthetic reserved `"agentmux"` entry is always auto-injected by `build_mcp_config_from_refs` (the name is rejected on `mcp.upsert`). This is the same `.mcp.json` mechanism every agent workspace uses.
- **No tool-allowlist field exists on this primitive at all** — `config` is an opaque JSON blob with no structured allowlist/permission shape in the DB row. Tool restriction for a curated-macro-tools-only Blender server (§3.2, §5) would have to happen either inside the bridge process itself (simplest, Phase 1) or wait on the **Policy primitive**, which the composable-model proposal named as a 7th primitive for exactly this (hooks/`.claude/settings.json`-style permissions) but explicitly **deferred** (`docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` §3.4/§9.6). Flagged as an open question in §6.

**A "Blender MCP Server" is just a new row in this table** — `transport: "stdio"`, `config: {"command": "uvx", "args": ["blender-mcp"], "env": {...}}` (or an AgentMux-maintained equivalent bridge). No schema change needed for the wire mechanism.

**Trust boundary any new Blender-specific *AgentMux* tool must follow** (as opposed to the MCP-server-row approach above, which needs none of this): if instead of a pure MCP server row you wanted a native `mcp__agentmux__*`-style tool (e.g. `AppDrive`), `docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md` §5 documents the required shape — `agentmux-mcp` holds identity out-of-band (`AGENTMUX_AGENT_ID`/`AGENTMUX_BLOCKID`/auth key) and POSTs to a REST route on `agentmux-srv`, which stamps the caller's identity **server-side** from `block_id` (never trusting agent-supplied JSON) before dispatching to an `app_api` handler. Any first-party Blender tool should be built this way, not as a bespoke side channel.

### 4.2 Skill primitive (the complementary vehicle)

Schema (`Skill` in `agentmux-srv/src/backend/storage/skills.rs`): `id`, `name`, `trigger`, `skill_type`, `description`, `content`, `is_global` (skills are now standalone catalog rows bound to agents/Bundles, like MCP servers, rather than per-agent rows carrying an `agent_id` as in the original text), loaded on demand by trigger match — structurally the same "load only when invoked" model as Claude Code's own Skills (which this session's tool list demonstrates). A "Blender workflow" Skill (e.g. "when asked to model/render/rig in Blender, follow this checklist and prefer these macro tools over raw Python") is the natural place to encode the safety posture from §3.2 and the "macro tools over raw scripts" guidance from §3.1, without hardcoding it into the MCP server itself.

### 4.3 Pane/widget system — no shortcut exists for native apps

View types are registered through the single pane-tab registry, `frontend/app/block/pane-tab-registry.ts` (`registerPaneTab`, `legacyAdapter`, `getPaneTab`; contract in `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`, status `active`). `frontend/app/block/block-registry.ts` registers the built-ins through it — `term`, `agent`, `browser`, `editor`, `sysinfo`, `help`, `launcher`, `swarm`, `memory`, `media`, `identity`, `drone`, `warden`, `toolchain`, `armory`, `settings` — and `block.tsx`/`blockutil.tsx` read everything else (label, icon, keep-alive lifecycle, capabilities) from the registry. (The original text cited `block-registry.ts:27-43` as a flat view-type-to-ViewModel map; that file was rewritten around the registry, so the line reference is gone.) Widget entries in `agentmux-srv/src/config/widgets.json` are mostly 1:1 with view types, **except** Discord/Slack/Telegram/WhatsApp/Teams, which are all just the `browser` view pointed at a different pinned URL — a "browser preset" trick that works because those are web apps.

**Blender is not a web app**, so that trick doesn't generalize: there is no URL to point CEF at. Visibility into a running Blender session needs either (a) a new pane type, or (b) no pane at all — just tool-call transcript in the agent pane, same as any other MCP tool call today. §5 recommends starting with (b).

The Browser pane itself, for reference, is a genuinely native embedded window on Windows — CEF's own `WindowInfo::set_as_child(parent_hwnd, &rect)` API creates a real `WS_CHILD` HWND (`agentmux-cef/src/browser_pane/creation.rs`), kept in sync with pane layout via `SetWindowPos` on resize (`agentmux-cef/src/browser_pane/`), with a `SetWindowRgn`+`CombineRgn` "hole-punch" trick to let DOM overlays (e.g. dropdowns) render on top of a sub-rect of it (`agentmux-cef/src/browser_panes/clip.rs`). On macOS/Wayland the same pane instead uses CEF's Views-overlay compositing, not a real OS child window — native child is unsupported there (comment in `creation.rs`). (As of 2026-08-10 these lived in a single `browser_panes.rs`; that file has since been split into the `browser_panes/` and `browser_pane/` modules.) This matters for §4.4: everything AgentMux embeds today is a window **CEF itself created**; nothing reparents a foreign process's window.

### 4.4 Native window embedding — reality check, and the team's own precedent for avoiding it

Reparenting an arbitrary external process's top-level window (Blender's, which uses its own "Ghost" windowing layer, not CEF) via `SetParent`/`WS_CHILD`-flip is a materially different, unproven, fragile piece of work — it would be a direct generalization of the `SetWindowPos`/`SetWindowRgn`/WndProc-subclass primitives already proven in `agentmux-cef/src/browser_panes/` and `agentmux-cef/src/browser_pane/hwnd.rs`, but on a window AgentMux doesn't own, which breaks with per-app titlebar/DPI/input-focus quirks (exactly the OS-level friction the GUI-agent research in §2 flags for Windows/macOS/Wayland). **Do not attempt this for Phase 1.**

Useful precedent: the team already hit an analogous "get a window to visually coexist with a pane, without reparenting" problem for the Browser pane's own z-order, and chose **not** to reparent — `docs/specs/SPEC_AGENT_BROWSER_CONTROL_2026_04_17.md` "Option A" (frameless popup) instead creates a separate frameless, always-on-top, `WS_EX_TOOLWINDOW`/`WS_EX_NOACTIVATE` popup window that tracks a pane's rect via resize/move listeners — no `SetParent` at all. For Blender the equivalent, even simpler move is: **don't create or reposition anything — let Blender keep its own independent OS window**, and give the user a manual "snap it alongside" affordance if wanted; AgentMux's pane shows only the tool-call transcript (§4.5) and, optionally, a polled screenshot (§5 Phase 2). This is lower-risk than even Option A and matches the team's demonstrated preference for position-tracking over reparenting.

**Don't build a bespoke pixel-vision fallback for Blender either** — one already exists as an unimplemented, previously-scoped proposal: `docs/specs/computer-use-pane.md` (a generic `compuse` pane: `xcap` for screenshots, `enigo` for mouse/keyboard synthesis, driving Anthropic's `computer_20251124` tool loop, with session-scoped app-approval and app-tier warnings). This is exactly the "vision fallback" leg of the hybrid model recommended in §2 — once built, it would let an agent fall back to clicking around Blender's UI for anything the `bpy`/MCP surface can't reach, with no Blender-specific code required. Treat it as complementary, sequenced work, not something this spec should re-scope: Blender-native MCP (§5 Phase 1) covers the 90% case reliably; `compuse`, if/when built, is the generic escape hatch for the remaining 10%, for Blender or any other app.

> **Status of `compuse` and its building blocks, verified 2026-09-25.** `docs/specs/computer-use-pane.md` is still an unbuilt proposal (a captured GitHub issue, #261; it carries no `**Status:**` line, and no `compuse` view, `enigo` dependency or `computer_20251124` tool loop exists in the repo). Its **capture half has, however, shipped by a different route**: the `agentmux-mcp` crate depends on `xcap` (0.4, not the `0.2` the proposal names) and exposes `DiscoverWindows` (list top-level windows with pid/title/exe) and `CaptureWindow` (PNG of any window, by pid or title match) — see `agentmux-mcp/src/window_capture.rs` and `docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md` (status `active`; capture tiers: T1 another pane in the caller's own instance, T2 another AgentMux instance, T3 another OS user's window — the one tier closed by default, T4 a non-AgentMux app window — allowed but always audited; PRs #2709, #2810, #2845). Blender's window would be tier T4 (`ForeignApp`). So option 2 of the live-view paths below (OS-level window capture) is no longer new platform work — an agent can already take a Blender window screenshot on demand, without any MCP connection to Blender. What does **not** exist: a periodically-refreshing pane view of that capture, and any input synthesis into a foreign window (the `enigo` half). The `UIScreenshot`/`UIClick`/`UIQuery` tools (#2662) are not part of this — they reach only AgentMux's own DOM.

**"Embedding" vs "a live view" are different asks with very different risk — don't conflate them.** Everything above (§4.3, §4.4) is about *interactive* embedding — Blender's window living inside a pane, clickable, reparented. A *read-only* view of what Blender is doing is a materially easier problem with two viable paths, neither of which touches reparenting:

1. **Zero new platform code**: the MCP bridge asks Blender to render its own viewport to an image (most `blender-mcp`-style servers can trigger a `bpy.ops.render.opengl`-style capture) and the agent polls that periodically into its pane transcript. This is exactly §5 Phase 2 — worth stating explicitly here: it already *is* the answer to "can I see a live view," no separate feature needed.
2. **Partly built (as of 2026-09-25) — the one-shot capture exists; only the live-refreshing pane view is new work**: OS-level window capture — grab pixels from Blender's specific window handle (Windows: `PrintWindow`/Desktop Duplication API) on an interval and render them as a live-refreshing image in a pane, entirely independent of the MCP connection. (`CaptureWindow` already does the grab on demand, via `xcap`; what is missing is the interval polling and the pane that renders it. Whether `xcap` returns usable pixels for Blender's GPU-drawn window has not been tested by this spec.) Because this never touches `SetParent`, never reparents, and never routes input, it sidesteps every fragility concern raised above — Blender's window keeps running exactly as if AgentMux weren't there; AgentMux is just periodically taking its picture. This is not new invention: it is a strict subset of the `xcap`-based capture half of `docs/specs/computer-use-pane.md` — skipping the `enigo` input-synthesis half and the Anthropic tool loop entirely — and that capture half now ships as `CaptureWindow`. If a "watch Blender live" feature is wanted before the full `compuse` pane ships, the remaining slice is just a polling pane over the existing capture path.

### 4.5 Shell primitive — the zero-cost fallback that already works today (batch only)

Three distinct shell mechanisms exist in this codebase — don't conflate them:

- **Terminal pane** (`term` view): a genuine PTY-backed shell (ConPTY on Windows, native pty elsewhere — `agentmux-srv/src/backend/blockcontroller/shell/`), rendered via xterm.js. This is what a human sees when they open a Terminal pane.
- **Agent-facing `Shell`/`ShellInput`/`ShellStatus`/`ShellStop` MCP tools** (what an agent calls for one-off commands): backed by `agentmux-srv/src/backend/shell_node.rs`, which still uses `tokio::process::Command` with **captured pipes — explicitly NOT a PTY** (its module doc comment calls PTY support an unstarted "Phase 3 follow-up" for *this* runner). `ShellInput` only writes line+newline to stdin, and only if the process was started with `capture_stdin=true`.
- **Agent-facing `PtyShell`/`PtyShellInput`/`PtyShellRead`/`PtyShellResize`/`PtyShellStatus`/`PtyShellStop` MCP tools** (new since the original write-up; schemas in `agentmux-mcp/src/tool_schemas.rs`, design and status in `docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md`): a **real PTY**, attached to the agent pane's own interactive shell — the same terminal a user sees in the shell drawer — so programs that check `isatty`, prompt for input, or are REPLs work. The original text's claim that "there's no PTY on the agent-tool side" is therefore only half true today: it holds for `Shell`, not for `PtyShell`.

**This already lets any agent run `blender --background --python script.py` today, with zero AgentMux changes** — good for batch jobs (render a scene, export an asset, run a headless simulation) where each invocation is stateless and output is plain stdout/stderr text; the plain `Shell` tool is the right fit. What none of the shell tools do is drive Blender's *GUI*: a PTY gives real terminal semantics, but it doesn't help hold a single live GUI Blender session open across many small back-and-forth tool calls with structured results (that's the MCP path, §4.1). Interactive GUI driving was never in scope for these tools and isn't unblocked by PTY support.

### 4.6 Toolchain pane — a good home for setup/detection, but not a drop-in

> **Status (2026-08-10): deferred, not in Phase 1 scope.** See §0.1. Kept as
> design reference in case a second/third connector later justifies the
> investment — Phase 1 ships the flat catalog entry from §5 instead.

The Toolchain pane (`toolchain` view, `frontend/app/view/toolchain/toolchain-view.tsx`) already does exactly the "detect installed → detect running → act" flow §3.3's manual setup steps need — but checked against its actual code, it's built around a shape that doesn't quite fit Blender:

- **Core Tools section** (in `toolchain-view.tsx`; catalog in `frontend/app/view/agent/providers/toolchain-catalog.ts`): pure CLI/path detection via the `ResolveCliCommand` RPC (shells out to `where <cmd>` / checks known install dirs, `agentmux-srv/src/server/cli_handlers.rs`). **This part is a clean drop-in** — add Blender and `uv`/`uvx` as catalog entries and get "installed / not installed, version, [Install]" rows for free, no backend changes. (Confirmed both are actually present on this dev machine already — Blender 5.0, `uv 0.9.4`.)
- **External Widgets section** (in `toolchain-view.tsx`; catalog in `frontend/app/view/agent/providers/widget-catalog.ts`) looks like the obvious reuse target but isn't: its `widget.health` RPC is **hardcoded to an HTTP GET** on the widget's port (`cli_handlers.rs`: `format!("http://127.0.0.1:{}{}", port, path)`, via `reqwest`), and its action button is hardcoded to `createBlock({ meta: { view: "browser", url: ... } })` (`toolchain-view.tsx`). (Re-checked 2026-09-25: both still true. Line numbers from the original text dropped; the files have since moved or been edited.) That's the right shape for local tools with a **web UI** (its own doc comment cites Grafana/Qdrant/Flowise) — detect, HTTP-health-check, open in a Browser pane. Blender's add-on speaks raw TCP/JSON on a socket (default port 9876, §3.3 — confirmed live 2026-08-10), not HTTP, and there's no page to browse to.

**Net**: this needs a new sibling category next to Core Tools and External Widgets — call it "Connectors" — not a reskin of External Widgets. Two small, well-scoped additions:
1. A TCP-connect health check (a new RPC or a `widget.health` variant that does a raw socket connect instead of an HTTP GET) to detect "is the Blender add-on's server actually listening." (Since superseded in spirit: the `mcp.probe` / `mcp.catalog.probe` RPC — a real MCP `initialize` + `tools/list` handshake, `agentmux-srv/src/backend/mcp_probe.rs` — now covers "is this server up and speaking MCP" for any registered server, which is why the flat catalog entry in §5 needs no new health RPC. It probes the *bridge*, so a stopped Blender add-on shows up as a probe failure with the entry's `prereqNote` next to it.)
2. An action button wired to the now-shipped `mcp.upsert`/`mcp.bind` RPC (§4.1, landed in `#1948`) — "Connect" binds the Blender MCP Server to the current agent — instead of `createBlock({view:"browser"})`.

This is a small, additive change to an existing pane, not new pane-type work, and it's the natural place to also surface §3.3's "Blender: connected / not responding" status live, since the Toolchain pane already polls and displays exactly this kind of state for every other row.

### 4.7 A general "Connector" pattern — not just for Blender

> **Status (2026-08-10): deferred, not in Phase 1 scope.** See §0.1 and §7
> open question 3 (now resolved: don't build this for a first connector).
> The schema below stays documented for whoever adds a *second* app-connector
> and finds the flat `McpPreloadEntry` shape (§5) genuinely insufficient —
> until then it's speculative, not scoped work.

Blender won't be the last third-party app that needs a manual, multi-step, human-in-the-GUI setup dance (§3.3 lived through exactly this during this spec's own write-up). The catalog-driven shape `toolchain-catalog.ts`/`widget-catalog.ts` (`frontend/app/view/agent/providers/`) already use for Core Tools/External Widgets generalizes cleanly to this — a third catalog, `connector-catalog.ts`, data-driven the same way:

```ts
interface ConnectorSetupStep {
    id: string;
    label: string;                 // "Install the Blender add-on"
    instructions: string;          // short markdown, rendered inline — not a link-only step
    actionLabel?: string;          // "Download add-on"
    actionUrl?: string;
    copyCommand?: string;          // for CLI-driven steps
    verify: { kind: "tcp" | "http" | "cli" | "file-exists"; /* probe params */ } | { kind: "manual" };
}

interface AppConnector {
    id: string;                     // "blender"
    label: string; icon: string; description: string; docsUrl: string;
    prerequisites: string[];        // Core Tool ids required first, e.g. ["uv"]
    setupSteps: ConnectorSetupStep[];
    liveCheck: { kind: "tcp" | "http"; port: number; path?: string };   // ongoing "Connected" pill
    mcpTemplate: { transport: "stdio" | "url"; command?: string; args?: string[]; urlTemplate?: string };
    riskNote?: string;              // shown as an unmissable callout, not fine print
    troubleshooting?: { symptom: string; explanation: string }[];
    verifiedAgainst?: string;       // "Blender 5.0.1 + Blender MCP addon v1.2, checked 2026-08-10"
}
```

**UI pattern** (a third Toolchain section, "Connectors," alongside Core Tools and External Widgets):

- **Collapsed row** when not fully connected: icon + name + one status pill — `Not started` / `N of M steps done` / `Connected`. Matches the existing row style exactly (same component family as `renderRow` in `toolchain-view.tsx`).
- **Expanded**: a numbered checklist, one row per `setupStep`. Each step shows live, auto-polled status wherever `verify` has a real probe (`tcp`/`http`/`cli`/`file-exists`) — **never a bare "I did this" checkbox when a technical signal exists**. This is exactly the discipline this conversation just followed by hand: I didn't take "it's enabled" at face value, I ran `netstat` and confirmed port 9876 was actually listening before saying so. The UI should hold itself to the same standard — a step only turns green because it's true, not because the user clicked something.
- Steps with no automatable signal (`verify: {kind: "manual"}`, e.g. "click Install in the Preferences dialog") still get a manual "mark as done" affordance, but it's the fallback, not the default.
- **Risk callout**: any connector with a `riskNote` renders it as a persistent, visually distinct warning block (not a tooltip, not collapsed by default) — the Blender entry's would carry §3.2's "executes code with no guardrails" warning verbatim.
- **Troubleshooting accordion**: seeded with real, known failure modes, not generic advice — Blender's entries would include §3.3's "window minimized → commands stall" and "first command sometimes doesn't go through" (now downgraded per the 2026-08-10 verification to "occasionally," not "expect this").
- **Terminal action**: once all steps verify true, a single **Connect** button appears and fires `mcp.upsert` (pre-filled from `mcpTemplate` — no hand-typed JSON, closing the exact gap flagged in §5 Phase 1's setup-UX note) followed by `mcp.bind`, in one click.
- Once connected, the row collapses back to a single line with a live status pill sourced from `liveCheck` (reusing §4.6's proposed TCP-probe RPC) and a "Disconnect" action — mirroring how Core Tools rows collapse once `found`.

**Caveat this design inherits**: as written, "Connect" registers the server (`mcp.upsert`/`mcp.bind`) but the *current* agent session won't actually see the new tools until it restarts — Claude Code only reads `.mcp.json` at process start. a hot-load feasibility study (`SPEC_MCP_HOTLOAD_2026_07_03.md`, reported at the time as "feasible, and smaller than expected — AgentMux already has session-continuity infrastructure built for crash recovery that a hotload feature can reuse") was cited here, but **that file does not exist anywhere in the repo or its git history** (checked 2026-09-25; it was likely another never-committed doc, like this one was), so the verdict cannot be re-verified and no path can be cited. Treat the hot-load feasibility claim as unconfirmed. Until that ships, "Connect" should be honest in its own UI copy that a restart is still required.

**Principles for whoever adds the next connector** (Figma, a DAW, a CAD tool, per §5 Phase 3):
1. **Auto-verify over self-report, always, whenever a technical signal exists.** A step without a real probe is a UX smell, not a default.
2. **Progressive disclosure.** Show the next actionable step; collapse what's already verified.
3. **Read-only, idempotent polling.** Checking status must never have a side effect — re-checking ten times in a row is always safe.
4. **Unmissable risk disclosure for anything code-exec-capable inside the third-party app** — a persistent callout, not fine print, not a one-time dismissible dialog that never resurfaces.
5. **No hand-typed JSON when the catalog already knows the shape.** The `mcpTemplate` exists specifically so "Connect" replaces "paste this JSON into a textarea" (§5 Phase 1's flagged gap).
6. **Version/staleness marker.** Third-party setup flows drift (this spec alone cited five different Blender bridge projects with slightly different steps) — `verifiedAgainst` makes staleness visible instead of silent.
7. **Troubleshooting seeded from real symptoms**, sourced from the same research/testing that produced the connector, not generic "check your configuration" text.

This doesn't need to be built for Blender alone to be worth designing this way now — the marginal cost of making the Blender entry data-driven against this shape instead of one-off is small, and it's what turns "one hardcoded Add Blender button" (§5 Phase 1, §7 open question 3) into an actual extensible catalog.

### 4.8 Geometry Nodes editor — a synthetic, MCP-backed pane

> **Decision (2026-08-10): rejected — not being built.** AgentMux is not
> building a Geometry Nodes pane, synthetic or otherwise. The analysis below
> is kept as the record of what was considered (product asked the question
> directly; this is the answer) and why a literal embed specifically doesn't
> work, in case the underlying "reconstruct a graph from MCP-introspected
> data, render with an existing node-canvas" pattern is useful elsewhere
> later. No Phase 1/2/3 work is scoped against it for Blender — §5 has been
> updated to remove it from the phased plan.

A live product question, not covered above: can AgentMux show Blender's
**Geometry Nodes editor** — the node-graph editor for procedural geometry — as
a pane inside AgentMux? Short answer: **not as a literal embed. A synthetic,
data-driven reconstruction is feasible and is the recommended path**, building
on §4.4's embedding-vs-live-view distinction rather than duplicating it.

**Why a literal embed doesn't apply here, specifically.** Geometry Nodes is one
editor *area type* among several inside Blender's single application window
(`bpy.types.SpaceNodeEditor` with `tree_type='GeometryNodeTree'`) — not a
separate window or process. §4.4 already ruled out reparenting Blender's
top-level window as fragile and unproven; detaching *one area* of that window
is strictly harder still (Blender's Ghost windowing layer exposes no API to
stream a single area's pixels or route input to it independent of the rest of
the window), so §4.4's "do not attempt this for Phase 1" verdict applies here
at least as strongly.

**§4.4's two read-only options both technically work, with one caveat specific
to this editor.** Blender does support maximizing a single area to fill the
window (`Ctrl+Space`), so a user could switch an area to Geometry Nodes and
maximize it, then §4.4 point 2's window-capture approach would show
approximately that editor and nothing else. This works but stays pixel-only —
no semantic access to the node graph, no round-trip write path, and it
requires the user to have arranged Blender's window that way first. Worth
keeping as a documented fallback for cases needing true visual fidelity
(node previews, color swatches) a synthetic render can't replicate — not the
primary recommendation.

**Recommended path: introspect the node tree via the MCP connector, render it
with AgentMux's own node-canvas.** AgentMux already has the building block this
needs — the Drone view (`frontend/app/view/drone/`) is a custom SolidJS
node-canvas (real DOM nodes, `nodrag`/`nowheel` editing, progressive
disclosure), explicitly designed by researching "ComfyUI / LiteGraph /
**Blender** / React-Flow custom nodes" as prior art
(`SPEC_DRONE_INLINE_NODE_PARAMS_2026_06_05.md` §0). The proposal:

1. Use `execute_code` (§3.2 — the same tool ~95% of real Blender work already
   routes through per the 2026-08-10 verification session) to walk the active
   `GeometryNodeTree` — `node_group.nodes`, each node's `inputs`/`outputs` and
   default values, `node_group.links` — and serialize it to JSON.
2. Render that JSON with AgentMux's own node-canvas primitives (Drone's
   machinery), as a new AgentMux-native pane — not a Blender embed. A real,
   interactive, agent-legible graph, not a pixel mirror.
3. **Read-only first.** A live mirror of the geometry node tree, refreshed by
   re-running the introspection call, ships real value — an agent or user can
   see current graph state without switching to Blender — without touching the
   write path. This is Phase 2/3 scope (§5), depends on Phase 1's connector
   existing to source data from.
4. **Read-write is a separate, harder decision, not assumed.** Editing the
   synthetic graph and pushing changes back would round-trip through
   `execute_code` — every edit becomes an arbitrary-Python-exec call
   constructed from pane state, inheriting §3.2's full risk posture. This needs
   its own risk review (validating generated Python, or a narrower dedicated
   node-editing RPC if the upstream server ever exposes one) before being
   scoped — see the new open question in §7.

**Bottom line:** no embed of Blender's actual Geometry Nodes UI is possible or
recommended, even with a native-window trick. A synthetic pane sourced from the
same MCP connector §5 Phase 1 ships, rendered with machinery already built for
Drone, is buildable, gives an interactive rather than pixel-only result, and is
the right Phase 2/3 scope once the connector exists to feed it data.

---

## 5. Proposed phased plan

**Phase 0 — document, ship nothing.** Write up the `blender -b --python script.py` pattern via the `Shell` tool as a supported "batch Blender job" recipe (candidate home: a Skill, or a short doc), **explicitly labeled as headless-only and unrelated to the interactive MCP path** (§3.3 — the two don't compose). No code. Covers headless render/export/simulation tasks immediately.

**Phase 1 — Blender as an MCP Server catalog entry, built to the cheap, proven shape (rescoped 2026-08-10 — see §0.1).**

> **Re-verified 2026-09-25: Phase 1 is now trivially small, and still unbuilt.** The flat preload-catalog mechanism it depends on has shipped — `MCP_PRELOAD_CATALOG` in `frontend/app/view/mcp/mcp-preload-catalog.ts` holds exactly two entries today, `ableton-live` and `touchdesigner` (specs: `docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md`, `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`). There is **no `blender` entry**, no Blender skill, and no other Blender-specific code. The picker (`McpCatalogPicker.tsx`), the `prereqNote`/`riskNote` rendering and the `mcp.catalog.probe` status pill all exist, so Phase 1 is one object literal plus a Skill, plus the still-open `uvx blender-mcp` version-pin check. The §4.8 Geometry Nodes pane is unaffected: rejected (§0.1) and unbuilt — no Geometry Nodes or Blender pane exists in `frontend/app/`.

Ship to the same flat `McpPreloadEntry` interface Ableton and TouchDesigner
actually use in production (`frontend/app/view/mcp/mcp-preload-catalog.ts`) —
`id, name, transport, config, prereqNote, riskNote, docsUrl` — not §4.7's
heavier `AppConnector` schema. No new RPC, no new Toolchain section, no
checklist UI. Concretely, one new entry appended to
`MCP_PRELOAD_CATALOG`:

```ts
{
    id: "blender",
    name: "Blender",
    transport: "stdio",
    config: { command: "uvx", args: ["blender-mcp"] },  // version pin: see below
    prereqNote:
        "Requires Blender already running, with the Blender MCP add-on installed " +
        "and connected via the BlenderMCP tab in the 3D viewport sidebar (N-panel) — " +
        "click \"Connect to MCP Server\" each time Blender restarts. " +
        "This bridge cannot launch Blender or install the add-on for you.",
    riskNote:
        "This connector's primary tool, execute_code, runs arbitrary Python " +
        "inside the running Blender instance with full bpy access and no " +
        "sandboxing — not an opt-in extra, the tool most other capabilities " +
        "(scene edits, node-tree changes, asset placement) route through.",
    docsUrl: "https://github.com/ahujasid/blender-mcp",
}
```

Remaining Phase 1 work, all small:
- **Version pin.** Addon side is live-verified (§3.3, 2026-08-10: Blender
  5.0.1 + addon v1.2, port 9876). The `uvx blender-mcp` stdio-server
  package's exact version still needs its own hands-on pass before the
  `config` above ships as fact (§7 open question 1).
- **Companion Skill** (§4.2): safety posture, macro-tool-first guidance, and
  the manual-launch-order requirement from §3.3 ("Blender must already be
  open with its add-on server started; don't try `blender --background`;
  don't minimize the window while working") — an agent needs this as much as
  a human does, or it burns turns retrying a connection that can't succeed
  without a human action first.
- **Nothing else.** Setup/connect already works through the Armory's MCP
  Servers tab (`mcp-manager.tsx`/`McpCatalogPicker.tsx`) exactly as it does
  for Ableton/TouchDesigner today — this catalog entry is the whole gap.

**Phase 2 — supervision, not embedding.** If users want to *watch* the agent work in Blender, add scene-screenshot polling (`get_viewport_screenshot`, rendered inline in the agent pane transcript, or just prompted for in the Skill) rather than window embedding (§4.4) — Blender keeps its own independent OS window throughout. Near-zero incremental cost once Phase 1 ships, since the tool call already exists. (As of 2026-09-25 a second, MCP-independent route also exists: the agent can call `DiscoverWindows` + `CaptureWindow` (§4.4) to screenshot Blender's whole window, not just the 3D viewport that `get_viewport_screenshot` returns — useful when the add-on socket is stalled, since it does not depend on Blender's timer loop.)

**Phase 3 — generalize beyond Blender, and reconnect with `compuse` (unscoped, no near-term plan).** If a second creative-app connector is ever requested, that's the trigger to revisit whether §4.6/§4.7's heavier catalog machinery is worth building (§0.1) — not before. Separately — and on its own timeline, not gated by this work — if/when the already-scoped but still unbuilt `compuse` pane (`docs/specs/computer-use-pane.md`; capture half already shipped as `CaptureWindow`, input half not — §4.4) ships, it becomes the generic vision-based fallback for driving *any* app, Blender included, wherever a native scripting hook doesn't reach.

---

## 6. Remote / cross-machine driving (e.g. "PC A's agent drives Blender running on PC B")

A natural extension: Blender runs on a different physical machine (PC B, itself running AgentMux) than the agent doing the driving (PC A). The question is whether **muxbus** — AgentMux's own cross-machine agent-messaging layer — is the right transport for this. Checked against the actual muxbus spec/code; short answer: **not as it exists today**, and not without new work that should be scoped separately.

### 6.1 What muxbus actually is (verified)

Muxbus is a **jekt** delivery system for agent-to-agent *coordination messages* — it is not a generic tunnel or RPC transport. Per `docs/specs/SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md` §2–4:

- **Tier 1** (same sidecar): delivery = literally writing `\r <message text> \r` into the target agent's PTY, via `ReactiveHandler`, rate-limited to 10 req/sec (token bucket).
- **Tier 2** (same host, different sidecar): HTTP POST to a peer's `/agentmux/reactive/inject`.
- **Tier 3** (LAN): speced, **not built** as of 2026-07 (`docs/specs/lan-awareness-and-embedded-jekt-api.md`). *(Updated 2026-09-25: a LAN delivery tier has since shipped — `DELIVERY=lan` jekts, LAN peer discovery in `agentmux-srv/src/backend/lan_discovery.rs`, per-agent LAN signing per `docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`. The conclusion below is unaffected: still coordination text, not a stream transport.)*
- **Tier 4** (WAN/cloud): `muxbus.agentmux.ai`, and — as of today — **pull-only**: the receiving side polls `/reactive/pending/{id}`, adding 0–5s latency per message (the NATS-leaf-node push model in the same spec §3 is the proposed fix, not yet shipped).
- *(Tier descriptions above are as of the 2026-07 write-up; only the LAN tier was re-checked on 2026-09-25 — WAN pull-only latency and the tier-1 rate limit were not re-verified.)*
- Every delivered message gets wrapped in a human-visible `[JEKT:FROM=... TIER=... MSGID=...]` marker and injected as text into the target **agent's own input stream** (`docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §3.1) — the trust/tier model (this repo's own `CLAUDE.md` Jekt rules) exists specifically so a *human* can tell a jekt from a typed message and gate sensitive actions on human confirmation.

None of this matches what an MCP connection needs: a low-latency, ordered, bidirectional stream of many small structured JSON-RPC frames per tool call, delivered to a **process**, not injected as text into an **agent's** input. Concretely, muxbus has no concept of "deliver these bytes to the Blender bridge on PC B" — its only addressable endpoint is an agent's PTY/input stream. Wrapping every `create_object`/`get_scene_graph` call as a jekt would also collide with the trust model: tier-1's 10 req/sec limiter would throttle a real interactive session, and a human being asked to confirm a "sensitive"-flagged jekt for every tool call (per the auto-escalation rules) would make the feature unusable. **Muxbus, as built, is the wrong layer for this.**

### 6.2 What already works today, with zero AgentMux code changes

MCP itself already has a remote-server story that AgentMux's MCP Server primitive already supports: `transport: "url"` (SSE/HTTP), not just `stdio` (§4.1, the `transport` field of `McpServer` in `mcp_servers.rs`). If PC A and PC B have IP-level reachability (same LAN, a VPN, Tailscale, etc.), the fix is entirely configuration:

- On PC B: run the Blender MCP bridge bound to a network-reachable interface (not `127.0.0.1`-only, which is the default in every reference implementation checked in §3.1 — for good reason, see below) and put some auth in front of it (bearer token at minimum).
- On PC A: register the Blender MCP Server row with `transport: "url"`, `config: {"url": "http://<PC-B-LAN-IP>:<port>/sse", "headers": {"Authorization": "Bearer ..."}}`.

**This is a real security escalation, not just a networking detail.** §3.2 already flags that a *local* `bpy`-exec surface has "no guardrails." Binding that same surface to a LAN interface turns it into a **network-reachable, unauthenticated-by-default code-execution surface** unless the bridge or a fronting proxy adds real auth — every OSS bridge in §3.1 defaults to localhost-only specifically to avoid this. Any doc/UI for this configuration must say so explicitly, not just describe the URL field.

### 6.3 The WAN/NAT case — where muxbus-the-infrastructure (not muxbus-the-jekt-protocol) might actually matter, as future work

If PC A and PC B aren't on the same network and a VPN isn't viable, the real problem is NAT traversal/reachability — which is exactly what muxbus's **cloud relay + Cognito auth** infrastructure (Tier 4) already solves, just for a different payload shape (jekt text messages). A purpose-built "remote MCP tunnel over muxbus" is a plausible future feature — reusing the existing relay/auth plumbing to carry opaque MCP frames between two designated sidecars instead of jekt text — but it:

- **Does not exist today.** No code path forwards raw bytes/JSON-RPC frames through muxbus; only structured jekt messages.
- **Needs a different trust model than jekt's**, since per-tool-call human confirmation doesn't scale — more likely a one-time "allow PC A to reach PC B's Blender bridge" authorization, checked once when the binding is created, not per call.
- **Is materially new scope** (a relay/tunnel primitive, plus the same network-exposure security work from §6.2, plus new trust semantics) — it should be its own spec if/when there's real demand for cross-machine Blender driving, not bundled into this one.

**Recommendation:** Phase 1 (§5) stays same-machine only. If cross-machine is needed next, reach for §6.2's plain MCP-over-LAN/VPN first — it costs nothing new to build. Treat a muxbus-based remote-MCP tunnel as explicitly out of scope for this spec and worth its own proposal later.

---

## 7. Open questions (need a product decision before Phase 1 lands)

1. **Bridge ownership**: fork/vendor an existing OSS Blender MCP server, or write a minimal AgentMux-maintained one? Vendoring is faster but inherits that project's security posture and update cadence; maintaining our own is slower but lets us enforce the macro-tools-only default from day one. The addon side is now live-verified (§3.3); **the `uvx blender-mcp` server package's exact version pin is still open** — needs its own hands-on pass, separate from the addon-socket verification already done.
2. **Sandboxing default**: given the Foundation's own "no guardrails, use a VM" warning (§3.2), should AgentMux *require* an explicit acknowledgment/warning dialog the first time a user binds the Blender MCP server to an agent — similar in spirit to the existing `FORBIDDEN` mutation guards, but user-facing?
3. ~~**Catalog UX scope**~~ — **Resolved 2026-08-10 (§0.1): ship the flat `McpPreloadEntry` shape, not §4.7's `AppConnector` schema, for this first connector.** Two real precedents (Ableton, TouchDesigner) already validated that the flat shape is sufficient; building the heavier schema now would be speculative. Revisit if/when a second connector actually needs what the flat shape can't provide.
4. **Tool-allowlist enforcement point**: since the MCP Server primitive has no allowlist field at all (§4.1), does the macro-tools-only default get enforced solely inside the bridge process (fastest, Phase 1), or should this be the forcing function that finally un-defers the **Policy** primitive (`docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` §3.4/§9.6)? Recommend: bridge-level for Phase 1, revisit if a second app-connector wants the same allowlist shape.
5. **`compuse` sequencing**: should this effort formally request `docs/specs/computer-use-pane.md` be prioritized (as the fallback modality, §4.4), or should Phase 1 ship Blender-MCP-only and treat `compuse` as fully independent, whenever-it-lands work? (Narrowed 2026-09-25: only the input-synthesis half is still missing — capture already ships as `CaptureWindow`.)
6. **Cross-machine demand**: is same-machine-only acceptable for an initial ship (§6 recommendation), or is "Blender on a render workstation, agent on a laptop" a real near-term use case that should pull the §6.2 remote-MCP-over-LAN config work into Phase 1 scope?
7. ~~**Geometry-node read-write risk**~~ — **Moot as of 2026-08-10 (§0.1): the Geometry Nodes pane (§4.8) was rejected outright**, so no read-write path exists to risk-review. Left struck through rather than deleted for the record.

## 8. Risks

- **Security**: raw-`bpy`-exec tools are a real code-execution surface inside a process with full filesystem/network access. Mitigation: curated macro tools by default + explicit opt-in gate (§5 Phase 1).
- **Thread-safety bugs in the add-on**: any bridge/add-on code AgentMux ships or vendors must marshal through `bpy.app.timers`; skipping this crashes Blender (§3.1).
- **Scope creep into window embedding**: §4.4's fragility argument should be treated as a hard "not now," not a soft preference — it's easy to underestimate the OS-level cost. §4.8's analysis (kept for record, not being built — §0.1) confirmed this applies at least as strongly to embedding a single editor area as to the whole window.
- **Silent stalls read as agent failure**: per §3.3, a minimized/backgrounded Blender window can stall command delivery for reasons invisible to both the agent and the user (no error, just no response) — though the 2026-08-10 verification session found this to be occasional, not routine. Without the connection-state surfacing called out in §5 Phase 1, users will file this as "the agent is broken" rather than "Blender needs to stay visible."
- **User expectation mismatch**: "integrated app driving" sounds like it should feel like the Browser pane (click it, it's there) or the Shell tool (fire-and-forget). It's neither — it requires a human to manually pre-launch and keep visible a separate application before any of this works. That expectation gap needs explicit product messaging, not just a Skill's fine print.
- **Network exposure creep**: the moment anyone points a Blender MCP Server row at a LAN URL instead of a local stdio command (§6.2), the "no guardrails" code-exec risk (§3.2) becomes network-reachable. This needs to be a loud warning in the setup UX, not an assumption users will infer.

## 9. References

- [Blender.org: MCP Server (official)](https://www.blender.org/lab/mcp-server/)
- [djeada/blender-mcp-server](https://github.com/djeada/blender-mcp-server)
- [glonorce/Blender_mcp](https://github.com/glonorce/Blender_mcp)
- [PatrykIti/blender-ai-mcp](https://github.com/PatrykIti/blender-ai-mcp)
- [Eigent: Claude for Creative Work — Blender MCP Connector Guide 2026](https://www.eigent.ai/blog/claude-blender-mcp)
- [Zylos Research: Computer Use and GUI Agents in 2026](https://zylos.ai/research/2026-02-08-computer-use-gui-agents/)
- [Zylos Research: GUI AI Agents & Computer Use, State of the Art 2025-2026](https://zylos.ai/research/2026-01-09-gui-ai-agents-computer-use)
- [Microsoft: Where AI meets GUI — an overview of computer-using agents](https://medium.com/data-science-at-microsoft/where-ai-meets-gui-an-overview-of-computer-using-agents-3085d3bbe332)
- [API Agents vs. GUI Agents: Divergence and Convergence (arXiv 2503.11069)](https://arxiv.org/pdf/2503.11069)
- Internal: `docs/specs/archive/EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md`, `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`, `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`, `docs/specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md`, `docs/specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`, `docs/specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`, `docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`, `docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md`, `docs/specs/SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md`, `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`, `docs/specs/SPEC_AGENT_BROWSER_CONTROL_2026_04_17.md` (Option A: non-reparenting overlay-window precedent), `docs/specs/computer-use-pane.md` (unimplemented generic vision-driving fallback; its capture half shipped as `CaptureWindow`), `docs/specs/SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md`, `docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` (§6's basis for why muxbus doesn't fit as an MCP transport), `frontend/app/view/drone/` + `docs/specs/SPEC_DRONE_INLINE_NODE_PARAMS_2026_06_05.md` (node-canvas precedent for §4.8), `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md` and `docs/reports/REPORT_ARMORY_ARCHITECTURE_AND_NAMING_REVIEW_2026_07_23.md` (specs that cited this file before it existed in the repo).

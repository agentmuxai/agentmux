# Spec: `OpenPane` — a general-purpose, agent-facing "open any pane" MCP tool

**Status:** design, not implemented.
**Author:** AgentY
**Date:** 2026-08-21
**Related:** `agentmux-mcp/src/main.rs` (`OpenEditor`, `OpenMedia` — the direct
precedent this spec generalizes), `docs/specs/SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md`
(the last time this exact pattern — whitelist a view in `build_pane_meta` +
add an MCP tool — was done, for `media`), `docs/specs/SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md`,
`agentmux-srv/src/server/app_api/pane.rs` (`build_pane_meta`, the whitelist
this spec widens), `agentmux-srv/src/server/app_api/mod.rs` (`open_pane`,
the shared reducer-backed core both the WS RPC and HTTP route call),
`frontend/app/block/block-registry.ts` (`blockViewRegistry` — the
authoritative list of view types that actually render), `CLAUDE.md`'s
"Widgets" / "Not widgets" tables (the human-facing surface this tool mirrors
for agents).

---

## 0. Motivation

Live gap hit today: verifying ABF (Armory Bundle Format) features required
opening the Armory pane from an agent's own conversation. No agent-facing way
to do this exists — `OpenEditor` and `OpenMedia` are the only two "open a
pane" MCP tools, and both are hardcoded to one view each (`editor`, `media`).
Every other pane a human can open by clicking the hamburger menu or a widget
— Armory, Settings, Toolchain, Swarm, Warden, Drone, Sysinfo, Help, Browser,
Terminal — has no agent-facing equivalent at all.

This is bigger than "add one more `OpenArmory` tool." The operator's framing
during investigation: *"being able to open panes and control agentmux is a
first-class feature of agentmux"* — the ask is a general capability, not a
one-off. Backed up by what the research below confirms: the **backend is
already generic** (one RPC, one function deciding what a `view` string means)
— it's only the agent-facing MCP layer that's been built one hardcoded tool
at a time. Continuing that pattern for every future view (Armory alone has
6 internal sections) doesn't scale and actively works against the "first
class" framing.

## 1. Research — the actual current mechanism (verified against live code, not assumed)

### 1.1 Correction to an earlier premise

Investigating this, it was initially assumed the hamburger menu (where
"Armory" lives for humans) is a native OS/Electron menu unreachable by
`UIClick`/`UIQuery` (which only see the web DOM). **That's wrong, checked
directly:** `frontend/app/window/hamburger-menu.tsx:117-121` is a plain
SolidJS-rendered flyout — `onClick: () => fireAndForget(() =>
openOrFocusPaneByView("armory"))`. It's reachable by web-DOM automation tools
after all. This doesn't change the motivation (an agent still has no
first-class *tool* to call), but the real gap is "no agent-facing verb for
this," not "the menu is unreachable."

### 1.2 There are two pane-creation paths today, not one

**(a) Low-level, unvalidated — `object.CreateBlock`** (WebSocket RPC only,
no HTTP route, not usable by an MCP tool without adding one):
- Used by the hamburger menu (`openOrFocusPaneByView`,
  `frontend/app/store/block-component-registry.ts:71-89`) and the widget bar
  (`frontend/app/window/action-widgets-config.ts:220-222`, blockdefs sourced
  from `agentmux-srv/src/config/widgets.json`).
- Backend: `agentmux-srv/src/server/service/object.rs:44-148`, the
  `"CreateBlock"` case of the `"object"` WSH service. Takes an arbitrary
  `BlockDef` — **no view-string whitelist, no per-view field validation** —
  and dispatches straight through the reducer.

**(b) High-level, validated — `pane.open`** (WS RPC *and* HTTP route; this is
the one `OpenEditor`/`OpenMedia` already use and the one this spec builds
on):
- Command constant: `agentmux-srv/src/backend/rpc_types/commands.rs:337`
  (`COMMAND_PANE_OPEN = "pane.open"`).
- Request shape: `CommandPaneOpenData` (`agentmux-srv/src/backend/rpc_types/block.rs:404-468`)
  — `view, file, url, cwd, title, tab_id, split_direction,
  split_reference_block_id, focus, tree_expanded, floating, meta,
  skip_placement, reuse_editor_pane`.
- HTTP route: `POST /api/v1/pane/open` (`agentmux-srv/src/server/mod.rs:425`,
  handler `handle_pane_open` at `mod.rs:928-950`) — a thin wrapper over the
  shared core.
- Shared core: `open_pane(state, cmd)` (`agentmux-srv/src/server/app_api/mod.rs:67`),
  whose own doc comment already says it's "shared by the WebSocket RPC
  handler … and the HTTP route … (`agentmux-mcp`'s `OpenEditor` tool)" — i.e.
  this spec's new tool is exactly the kind of caller this function already
  anticipates.
- **The gap**: `build_pane_meta` (`pane.rs:208-265`) — the function that
  turns a bare `view` string into real block meta — only has arms for
  `editor`, `term`, `browser`, `sysinfo`, `help`, `media`. Every other real,
  rendering view (`armory`, `settings`, `toolchain`, `memory`, `warden`,
  `drone`, `swarm`, `agent`, `identity`, `cpuplot`, `launcher`) hits the
  `other =>` arm and 400s with `INVALID_VIEW` — **unless** the caller
  supplies `cmd.meta` directly, which `open_pane` uses as-is
  (`mod.rs:75-78`), bypassing `build_pane_meta` entirely. This is exactly how
  the widget bar's unvalidated path (§1.2a) effectively works today for
  those views, just through a different RPC.

This is the concrete inconsistency behind the "architecture rethink"
question: two different pane-creation paths, one validated (narrow
whitelist), one not (whatever the frontend hands it), doing the same job.

### 1.3 Authoritative view-type list

`frontend/app/block/block-registry.ts:25-47` (`blockViewRegistry`), the
ground truth for what actually renders:

```
term, cpuplot, sysinfo, help, launcher, agent, swarm, editor, browser,
memory, media, identity, drone, warden, toolchain, armory, settings
```

Plus a legacy alias (`block.tsx:53`): `view: "forge"` renders as `agent`.

Per-view required extra meta, confirmed against `agentmux-srv/src/config/widgets.json`
and each view's own model:
- `editor` — none required (optional `editor:tree_expanded`).
- `media` — `media:path` (a file).
- `browser` — `url`.
- `term` — none required (widget sets `controller: "shell"`, not user-supplied).
- `armory`, `settings`, `toolchain`, `sysinfo`, `help`, `swarm`, `drone`,
  `warden`, `memory` — **none required**, `view` alone is sufficient.
- `agent` — has its own much richer, already-agent-facing flow
  (`agent_open.rs`) with its own dedicated `agent.open` command; out of scope
  here (see Non-goals).

**Armory's internal sections are not separate `view` strings.** They're a
second block-meta key, `armory:section`, patched *after* the Armory pane
already exists (`frontend/app/view/armory/armory-view.tsx:33-37`, a
`SetMetaCommand` RPC). Valid values —
`ArmorySection` (`frontend/app/view/armory/armory-model.ts:14`):
`"accounts" | "memory" | "skills" | "mcp" | "bundles" | "native_memory"`.
(Note: `"skills"` plural — matches the earlier finding that ABF's
programmatic-access view registers under `memory`, distinct from Armory's
`"memory"` *section* label, which is Armory Native Memory, not ABF. See §4
open question — this naming collision needs to be resolved carefully in the
tool's own argument description so a caller doesn't confuse "ABF" with
"Armory's memory section.")

### 1.4 The `OpenEditor`/`OpenMedia` template

Both live in `agentmux-mcp/src/main.rs`: schema consts (`OPEN_EDITOR_TOOL`
at `main.rs:323-337`, `OPEN_MEDIA_TOOL` at `main.rs:339-352`) plus handler
arms (`main.rs:1091-1170` and `main.rs:1172-1241`). Each: reads args, checks
`AGENTMUX_LOCAL_URL`/`AGENTMUX_AUTH_KEY`/`AGENTMUX_BLOCKID` env, builds a
`PaneOpenRequest`, POSTs to `{local_url}/api/v1/pane/open` with an
`X-AuthKey` header, parses `{block_id}` back, returns a one-line confirmation
string. This is the exact shape `OpenPane`'s handler copies.

Two independent places a tool must be registered, confirmed by
`SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md`'s own post-review
correction (do not repeat that near-miss):
1. The production `"tools/list"` response array, `main.rs:513`.
2. The test-only `defs` validity/count array, `main.rs:1719-1747` (bump
   `assert_eq!(defs.len(), N, ...)`).

### 1.5 Tests

No MCP-tool-layer tests exist for `NewTab`/`OpenEditor`/`OpenMedia` (thin
HTTP wrappers, nothing to unit test). Real coverage is server-side:
`agentmux-srv/src/server/app_api/mod.rs`'s `pane_open_reducer_tests` module
(`docked_pane_open_block_is_in_reducer_and_tears_off`,
`skip_placement_creates_block_without_touching_the_layout_tree`) —
`#[tokio::test]`, `test_state()` in-memory `AppState`, hand-built
`CommandPaneOpenData { .. }`, `open_pane(&state, cmd).await`, assert against
`state.srv_state.blocks`. **No test today exercises `build_pane_meta`'s
`INVALID_VIEW` rejection or a not-yet-whitelisted view** — this spec's
backend change adds the first such coverage, in the same module/idiom.

---

## 2. Design decision — one generic tool, not one tool per view

Explicitly settling the "separate commands per pane vs. one command with a
pane argument" question raised during design:

**One generic `OpenPane` tool**, taking `view` as its primary argument. Not
a new hardcoded tool per view.

Reasoning:
1. The backend already treats this uniformly in intent (one `pane.open`
   command, one `build_pane_meta` dispatch) — building N MCP tools on top of
   one generic mechanism is an asymmetry that gets worse as the view list
   grows (already 17 entries, and Armory alone fans out into 6 sections).
2. Every future view becomes agent-usable the moment it's added to
   `build_pane_meta`'s whitelist, with no follow-up MCP-tool PR — this is
   what makes it a "first-class, general" capability rather than a series of
   one-offs.
3. It closes the §1.2 inconsistency as a side effect: widening
   `build_pane_meta` to cover the full `blockViewRegistry` set means the
   *validated* path (`pane.open`) becomes capable of everything the
   *unvalidated* path (`object.CreateBlock`) already does today, rather than
   adding a third, MCP-tool-specific bypass (raw `meta` passthrough) that
   would just paper over the gap instead of closing it.

**`OpenEditor`/`OpenMedia` are kept as-is** — not retired, not required to
change. They're established, presumably-in-use tools with genuinely distinct
ergonomics (`collapse_tree` for Editor; file-type handling for Media)
that read better as dedicated, self-documenting tools than as
`OpenPane(view: "editor", file: ...)`. `OpenPane` is additive: the general
entry point for everything else, and a template any future dedicated tool
can still choose to peel off from if a view earns bespoke ergonomics later
(non-goal to unify them into shared Rust code right now — see §5).

---

## 3. Design

### 3.1 Backend: widen `build_pane_meta`'s whitelist

`agentmux-srv/src/server/app_api/pane.rs:211-258`. Add arms for every
no-extra-arg view confirmed in §1.3:

```rust
"armory" | "settings" | "toolchain" | "memory" | "warden" | "drone"
    | "swarm" | "identity" | "cpuplot" | "launcher" => {
    meta.insert("view".to_string(), json!(cmd.view.as_str()));
}
```

(`agent` deliberately excluded — it has its own dedicated, much richer
`agent.open` flow; routing it through this generic arm would bypass that
flow's real setup logic. See Non-goals §5.)

For `armory` specifically, accept an optional `section` field on
`CommandPaneOpenData` (new field, or reuse the existing generic `meta`
passthrough for it) and, when present, additionally insert
`"armory:section": json!(section)` into the built meta — validated against
the real `ArmorySection` enum (`"accounts" | "memory" | "skills" | "mcp" |
"bundles" | "native_memory"`) server-side, erroring the same way an
unrecognized `view` does now, rather than trusting the caller's string
blindly.

Update the `INVALID_VIEW` error message (`pane.rs:255`) to list the full
expanded set so a typo'd view string still gets an accurate hint.

**Open question (not resolved by research, needs a real check before/during
implementation):** does the Armory pane's frontend read `armory:section`
from a block's *initial* meta (set at creation) the same way it reads a
later `SetMeta` patch? `armory-view.tsx:33-37`'s existing usage is
patch-after-create, not create-with-section already set. If the Armory
`ViewModel`'s section signal only initializes from a live subscription and
not from initial mount meta, this needs a small frontend read-path fix
alongside the backend change, or the tool needs to issue a follow-up
`SetMeta` call itself after `pane.open` returns (matching the two-step
dance a human's UI already does) rather than assuming a one-shot create
works. Verify empirically (open an Armory pane via a hand-crafted
`pane.open` call with `meta: {"view":"armory","armory:section":"bundles"}`
and observe) before committing to the one-call design.

### 3.2 New MCP tool: `OpenPane`

`agentmux-mcp/src/main.rs`, mirroring `OPEN_EDITOR_TOOL`/`OPEN_MEDIA_TOOL`:

```rust
const OPEN_PANE_TOOL: &str = r#"{
  "name": "OpenPane",
  "description": "Open any AgentMux pane by view type next to this conversation — Armory (Bundles/MCP Servers/Skills/Accounts/Native Memory), Settings, Toolchain, Swarm, Warden, Drone, Sysinfo, Help, Browser, Terminal. For editor/media files specifically, prefer OpenEditor/OpenMedia instead. Fire-and-forget: returns once the pane is opened.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "view":     { "type": "string", "enum": ["armory","settings","toolchain","memory","warden","drone","swarm","identity","cpuplot","launcher","sysinfo","help","browser","term"], "description": "Which pane to open. 'memory' is ABF (Armory Bundle Format), distinct from Armory's own 'native_memory' section — see 'section'." },
      "section":  { "type": "string", "enum": ["accounts","memory","skills","mcp","bundles","native_memory"], "description": "Only valid when view='armory' — jump straight to this Armory sub-tab (e.g. 'bundles' for ABF, 'mcp' for MCP Servers)." },
      "url":      { "type": "string", "description": "Required when view='browser' — the URL to load." },
      "title":    { "type": "string", "description": "Optional tab/pane title." },
      "split":    { "type": "string", "enum": ["right", "left", "down", "up"], "description": "Where to place the new pane relative to this agent pane (default: right). Ignored when floating is true." },
      "floating": { "type": "boolean", "description": "Open in a floating window instead of a docked split. Default: false." }
    },
    "required": ["view"]
  }
}"#;
```

Handler (`"OpenPane" => { ... }`): same shape as `OpenEditor`'s — env
checks, build a `PaneOpenRequest { view, url: (browser only), title, focus:
Some(true), split_direction, split_reference_block_id, floating, meta: (if
section present, Some({"armory:section": section})), file: None,
tree_expanded: None, reuse_editor_pane: None }`, POST to
`{local_url}/api/v1/pane/open`, return `"Opened {view} pane (block
{block_id})"` (or `"... section {section}"` when set).

Register in both places per §1.4 — production `tools/list` array
(`main.rs:513`) and the test `defs` array + bumped count
(`main.rs:1719-1747`).

### 3.3 Tests

Backend: extend `pane_open_reducer_tests`
(`agentmux-srv/src/server/app_api/mod.rs`) with:
- One test per newly-whitelisted no-arg view (or table-driven over the list)
  confirming `open_pane` now succeeds instead of `INVALID_VIEW`.
- A `view: "armory", meta: Some({"armory:section": "bundles"})` test,
  resolving §3.1's open question — assert the created block's meta actually
  carries `armory:section` (whether that's sufficient for the frontend to
  render the right tab is a separate, UI-level check, but this pins the
  backend contract).
- A still-rejected case (an unknown view string) to lock in the widened-but-
  still-finite whitelist and the updated error message.

MCP layer: no existing precedent for tool-handler unit tests (§1.5) — follow
that precedent (none) unless this changes for `OpenEditor`/`OpenMedia` too.

---

## 4. Open questions

1. **§3.1's `armory:section` initial-meta question** — needs a real check,
   not an assumption, before implementation is considered done.
2. **Should `browser`'s `url` requirement be enforced in the new whitelist
   arm** (mirroring `media`'s `MISSING_ARG` pattern in the `OpenMedia` spec)
   **or left to the frontend to show its own empty-state?** Leans toward
   requiring it server-side, matching `media`'s existing precedent exactly
   rather than introducing a new laxer convention.
3. **Naming clash**: `memory` (top-level ABF view) vs. Armory's own
   `"memory"` *section* value (Armory Native Memory, a different feature).
   The tool schema's description above calls this out explicitly, but is
   that enough, or does this warrant renaming one of the two identifiers at
   the source (a much larger, separate change, and explicitly out of scope
   here — flagged only so it isn't lost)?
4. **Should `agent` ever be included?** Deliberately excluded in §3.1 because
   `agent.open` already exists as its own rich, dedicated flow — but if a
   future need arises for "open a *view* of an already-running agent's pane
   without going through the launch flow," that might belong here instead.
   Not needed for this spec's motivating case (Armory/ABF).

## 5. Non-goals

- Refactoring `OpenEditor`/`OpenMedia` to share Rust implementation code
  with `OpenPane` internally. Worth doing eventually (all three ultimately
  build a `PaneOpenRequest` and POST it) but not required to ship this, and
  risks destabilizing two already-working tools for a pure DRY win.
- Routing `view: "agent"` through this tool (see Open question 4).
- Closing the §1.2 inconsistency at the `object.CreateBlock` layer itself
  (the widget bar / hamburger menu's own unvalidated path). This spec only
  widens the *validated* path to match it in capability; it doesn't touch
  or restrict the existing unvalidated one.
- A generic "close pane" / "list open panes" / "focus pane" agent-facing
  tool set. `OpenPane` is scoped to opening; a fuller pane-lifecycle control
  surface (matching the "control agentmux" framing more completely) is a
  natural follow-up but a separately-scoped one.

## 6. Files (anticipated — this spec does not implement)

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/server/app_api/pane.rs:211-258` | `build_pane_meta` — widen the whitelist (§3.1), the real gap |
| `agentmux-srv/src/server/app_api/mod.rs` (`pane_open_reducer_tests`) | New tests per §3.3 |
| `agentmux-mcp/src/main.rs:323-352` | `OPEN_EDITOR_TOOL`/`OPEN_MEDIA_TOOL` — pattern `OPEN_PANE_TOOL` copies |
| `agentmux-mcp/src/main.rs:1091-1241` | `"OpenEditor"`/`"OpenMedia"` handler arms — pattern the new `"OpenPane"` arm copies |
| `agentmux-mcp/src/main.rs:513` | Production `tools/list` registration — add `open_pane` |
| `agentmux-mcp/src/main.rs:1719-1747` | Test `defs` array + count assertion — add `OPEN_PANE_TOOL`, bump count |
| `frontend/app/view/armory/armory-model.ts:14` | `ArmorySection` — the enum `section` validates against |
| `frontend/app/view/armory/armory-view.tsx:33-37` | Existing `SetMeta` pattern for `armory:section` — reference for §3.1's open question |
| `frontend/app/block/block-registry.ts:25-47` | `blockViewRegistry` — authoritative view-type list this spec's whitelist widening targets |

<!-- Captured from GitHub issue #261 — agentmuxai/agentmux -->

## Overview

Add a first-class **Computer Use pane** (`compuse`) to AgentMux, giving agents the ability to see and control the desktop — screenshots, mouse, keyboard, scroll — using the Anthropic `computer_20251124` tool API. Modeled after Claude Code's new built-in `computer-use` MCP server (shipped v2.1.85, ~2026-03-26).

---

## Background

On March 26, 2026 Anthropic shipped `computer-use` as a built-in MCP server in Claude Code. It lets Claude take screenshots, move the mouse, click, type, scroll, and switch displays on a real desktop. AgentMux should own this loop natively — the agent pane drives the tool loop, the CEF backend executes actions, and the pane shows a live annotated view.

---

## The Anthropic Computer Use API

**Tool definition:**
```json
{
  "type": "computer_20251124",
  "name": "computer",
  "display_width_px": 1920,
  "display_height_px": 1080,
  "enable_zoom": true
}
```
Beta header required: `computer-use-2025-11-24` (Opus 4.6, Sonnet 4.6, Opus 4.5).

**Full action surface:** `screenshot`, `left_click`, `right_click`, `double_click`, `triple_click`, `mouse_move`, `left_click_drag`, `left_mouse_down`, `left_mouse_up`, `scroll`, `type`, `key`, `hold_key`, `wait`, `zoom`

**Agent loop:**
```
Pane sends prompt + tools to Claude API
  ↓
Claude returns tool_use: { action: "screenshot" }
  ↓
AgentMux executes action → captures screenshot
  ↓
AgentMux returns tool_result with base64 image
  ↓
Claude reasons → next action
  ↓
Loop until task done
```

**AgentMux is the execution layer** — Claude is purely the reasoning engine.

**Coordinate scaling:** Screenshots are downsampled to max 1568px long edge before sending to Claude. Coordinates returned by Claude must be scaled back to actual screen resolution before executing clicks.

---

## Architecture (CEF host)

```
┌─────────────────────────────────────────────────────┐
│  AgentMux Frontend (SolidJS in CEF webview)         │
│  ┌──────────────────────────────────────────────┐   │
│  │  CompUsePane                                  │   │
│  │  ├── ScreenView (annotated screenshot)        │   │
│  │  ├── ActionLog (click/type/scroll history)    │   │
│  │  ├── PromptBar                                │   │
│  │  └── AppApprovalDialog                        │   │
│  └──────────────────────────────────────────────┘   │
└────────────────────────┬────────────────────────────┘
                         │ HTTP POST IPC (cef-api.ts → agentmuxsrv-rs)
┌────────────────────────▼────────────────────────────┐
│  agentmuxsrv-rs  (Rust sidecar)                      │
│  ├── AnthropicClient (computer_20251124 tool loop)   │
│  ├── ScreenCapture  (xcap crate)                     │
│  ├── InputDriver    (enigo crate)                    │
│  ├── AppApprovalStore (session-scoped)               │
│  └── CoordinateScaler                               │
└─────────────────────────────────────────────────────┘
              spawned by agentmux-cef (CEF host)
```

### Rust crates

```toml
xcap = "0.2"           # screen capture — ScreenCaptureKit (macOS), DXGI (Windows)
enigo = "0.2"          # mouse + keyboard simulation, cross-platform
accessibility = "0.1"  # macOS AXUIElement — structural UI queries (Phase 2)
uiautomation = "0.3"   # Windows UIA — structural UI queries (Phase 2)
```

### New IPC commands (agentmuxsrv-rs)

```
POST /api/compuse/start    { pane_id, prompt }       → { session_id }
POST /api/compuse/approve  { session_id, app_name }  → {}
POST /api/compuse/cancel   { session_id }            → {}
GET  /api/compuse/screenshot?session_id=…            → { image_b64, width, height }
```

### Push events → frontend (via CEF bridge / CustomEvent)

```
compuse:action      { session_id, action_type, coordinate?, text? }
compuse:screenshot  { session_id, image_b64, width, height }
compuse:approval    { session_id, app_name, tier }
compuse:done        { session_id, summary }
compuse:error       { session_id, message }
```

---

## UI

```
┌─────────────────────────────────────────────┐
│  [●] Screenshot  [▶ Running]  [✕ Cancel]   │  ← toolbar
├─────────────────────────────────────────────┤
│                                             │
│   [annotated screenshot with action dot]   │  ← ScreenView
│                                             │
├─────────────────────────────────────────────┤
│  ✓ screenshot                               │
│  ✓ left_click (1240, 340)                  │  ← ActionLog
│  ✓ type "search query"                     │
│  ⟳ screenshot                              │
├─────────────────────────────────────────────┤
│  [  Task prompt...                    ] [▶] │  ← PromptBar
└─────────────────────────────────────────────┘
```

Action annotations overlaid on screenshot: click → pulsing dot, type → keyboard icon, scroll → arrow.

### widgets.json entry

```json
{
  "defwidget@compuse": {
    "view": "compuse",
    "display": { "label": "computer use", "order": 5, "icon": "desktop", "color": "#8b5cf6", "visible": true }
  }
}
```

---

## Security Model (mirrors Claude Code)

- **Session-scoped app approval** — user must approve each app before the agent can control it; approvals cleared on pane close
- **App tier warnings** — Terminal/shell apps flagged as "equivalent to shell access"; Finder as "can read or write any file"; System Settings as "can change system settings"
- **Never auto-approve** Terminal, Finder, System Settings
- **Machine-wide mutex** — one active computer-use session at a time (`Mutex<Option<SessionId>>` in agentmuxsrv-rs)
- **No self-screenshot** — AgentMux window excluded from captures (prevents prompt injection from its own UI)
- **Credential redact toggle** — option to black out password fields before sending to Claude

---

## Platform Support

| Feature | macOS | Windows | Linux |
|---|---|---|---|
| Screenshot | `xcap` (ScreenCaptureKit) ✓ | `xcap` (DXGI) ✓ | `xcap` (X11/PipeWire) ⚠ |
| Mouse simulation | `enigo` ✓ | `enigo` ✓ | `enigo` (X11) ⚠ |
| Keyboard simulation | `enigo` ✓ | `enigo` ✓ | `enigo` (X11) ⚠ |
| Structural UI queries | `accessibility` (AXUIElement) Ph2 | `uiautomation` (UIA) Ph2 | AT-SPI2 (future) |
| Permissions | TCC (Accessibility + Screen Recording) | None required | distro-dependent |

macOS: `agentmux-cef` needs `NSAccessibilityUsageDescription` + `NSScreenCaptureUsageDescription` in `Info.plist`. Portable ZIP distribution — no App Store sandbox constraint.

---

## Phased Plan

### Phase 1 — Windows + macOS pixel-based MVP
- [ ] `xcap` screen capture in agentmuxsrv-rs
- [ ] `enigo` mouse + keyboard in agentmuxsrv-rs
- [ ] Anthropic `computer_20251124` tool loop
- [ ] Coordinate scaling (downsample → execute → scale back)
- [ ] Session-scoped `AppApprovalStore`
- [ ] Machine-wide session mutex
- [ ] CompUsePane frontend: ScreenView + ActionLog + PromptBar
- [ ] AppApprovalDialog with tier warnings
- [ ] macOS permission onboarding dialog (TCC deep link)
- [ ] `defwidget@compuse` in widgets.json

### Phase 2 — Structural UI queries
- [ ] `accessibility` crate — AXUIElement on macOS (find button by label, not pixel)
- [ ] `uiautomation` crate — Windows UIA element tree
- [ ] Multi-display support
- [ ] App-tier warning system in UI

### Phase 3 — Power features
- [ ] View-only mode (stream live desktop, no agent control)
- [ ] Record + replay (store action sequences as reusable macros)
- [ ] AgentBus integration — one pane triggers another pane's computer-use session
- [ ] Drone integration — computer-use sessions as schedulable Drone tasks
- [ ] Remote mode — control another machine over AgentBus
- [ ] Wayland support (Linux)

---

## Open Questions

- Should the model be configurable per-pane (Opus 4.6 for complex tasks, Sonnet 4.6 for speed/cost)?
- Do we want to optionally proxy to Claude Code's built-in `computer-use` MCP server when installed?
- UX for mid-task clarification questions from the agent?

---

## References

- [Claude Code computer use docs](https://code.claude.com/docs/en/computer-use)
- [Anthropic computer use tool API](https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool)
- Full spec: `docs/specs/computer-use-pane.md`

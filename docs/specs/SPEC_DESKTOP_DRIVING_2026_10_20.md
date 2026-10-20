# SPEC: Desktop driving: an agent looks at and controls a native app, with the user's consent for each app

**Status:** proposed — nothing built. Verified against `main` 2026-10-20 (v0.59.35): there is no OS-level input injection in `crates/` (the `CGEvent*` hits in `crates/cef` only read button state or tap events for window dragging), and `xcap` is used only by `CaptureWindow`.
**Date:** 2026-10-20
**Author:** Manoz@Area54, at the operator's request
**Trigger (operator, 2026-10-20):** "write spec to file and lets build it", after a review of issue #261 and of how Claude Cowork, Perplexity and OpenAI's CUA do computer use.
**Supersedes the design in:** [`computer-use-pane.md`](computer-use-pane.md) (issue #261, a `compuse` pane that runs its own Anthropic tool loop). The agent pane already runs the loop; what is missing is the tool.
**Builds on:** `CaptureWindow` and its tiers (`crates/mcp/src/window_capture.rs`), the browser pane's approve-in-the-pane flow (`crates/srv/src/server/browser_actions.rs`, `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md`), the `UI` tool family (`SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md`), and `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`.

---

## 1. Summary

Agents can see any OS window (`UI window`) and drive AgentMux's own UI (`UI click`), a browser pane they opened (`Browser`), and a terminal (`Terminal`). They cannot click or type in a native desktop app that has no other way in: a game, a legacy GUI, a design tool without a scripting API, an installer.

This spec adds one tool, `Desktop`, for that gap. It is the last rung of a ladder, not the first: an agent uses a connector, an app's own API, `Browser`, or `Terminal` when one exists, and `Desktop` only when none does. Every use is gated by the person at the machine:

- **off by default**, switched on in settings and per agent;
- **a grant per app**: the first action on an app asks the user, in the agent's pane, and the grant expires;
- **a blocklist**: password managers, web browsers (use `Browser`), AgentMux itself, and anything the user adds;
- **visible, stoppable**: a "Driving <app>" badge with a Stop button, and a stop that cuts the next action off;
- **audited**: every action is written to an append-only log.

Phase 1 is Windows only. macOS and Linux (X11) follow; Wayland is not supported because it has no sanctioned way to inject input.

## 2. Why now, and what the others do

How the products in this space are built (public documentation):

| | Acts on | Sees | Gate |
|---|---|---|---|
| Claude Cowork | the user's real desktop | screenshots | permission per app; trading and crypto apps blocked by default; user can stop it or blocklist an app |
| Perplexity Comet | the browser, via an extension | the DOM and accessibility tree | not published |
| OpenAI Operator (CUA) | a virtual browser on OpenAI's servers | raw pixels | the user takes over, and enters payment details themselves |

Two things carry over. **Structured routes first:** Cowork tries connectors, then the browser, then the screen; Comet never uses pixels. AgentMux already has the structured routes (§3). **Consent per app, with a way to stop:** Cowork's model, which this spec follows.

Where AgentMux differs from Cowork: the grant is held by AgentMux (an agent is a process the user launched and may not be watching), and every action is audited in a log the agent cannot edit.

## 3. The ladder, and what `Desktop` must not replace

1. **A connector or an app's own API** (MCP servers, `bpy` for Blender, etc.).
2. **`Browser`**: element references, secret-field guards, approval of committing clicks. A browser window is blocked from `Desktop` by default so an agent cannot go around those guards.
3. **`Terminal`**: keys and output for a shell.
4. **`UI`**: AgentMux's own UI.
5. **`Desktop`**: pixels and the accessibility tree of a foreign native app. This spec.

The tool description tells the agent this order. An agent that reaches for `Desktop` with a browser window gets a refusal that names `Browser`.

## 4. Goals and non-goals

**Goals**
- An agent can open a native app's window, read it, click, type, scroll, and press keys, with the user's consent for that app.
- The user can see that it is happening and stop it at any time.
- Every action is attributable (agent, app, action) in a durable log.
- Phase 1 works end to end on Windows with Notepad and Calculator.

**Non-goals**
- Not an OS sandbox. `Desktop` is a consent, visibility and audit layer for the sanctioned tool. Confining what an agent's own shell can do is a separate subject (container agents, `SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md`; issue #2939).
- No driving AgentMux itself (use `UI`), other users' sessions, other AgentMux instances, elevated windows, or the secure desktop (UAC, lock screen).
- No background (non-foreground) driving, no automatic multi-step planning inside the tool, no recording or replay. The agent pane runs the loop.
- No Wayland.

## 5. Design

### 5.1 The tool

One tool, `Desktop`, with an `action`, like the other MuxAPI tools. It is listed for an agent only when `desktop:enabled` is true and the agent has it enabled (§5.7).

| Action | Arguments | Does |
|---|---|---|
| `apps` | none | Lists windows the agent could control: id, title, app name, tier, and whether granted or blocked (with the reason). |
| `look` | `window` | A screenshot of one window, like `CaptureWindow`, scaled to at most 1568 px on the long edge. Records the window's geometry for later clicks. |
| `click` | `window`, `x`, `y`, `button` (left, right, middle), `count` (1–3) | A click. |
| `scroll` | `window`, `x`, `y`, `dx`, `dy` | A wheel scroll at a point. |
| `type` | `window`, `text` (at most 500 characters) | Types text into the focused control. |
| `key` | `window`, `keys` (for example `ctrl+s`) | Presses a key or chord. |
| `wait` | `ms` (at most 5000) | Waits. It touches no app, so it does not ask srv. |
| `release` | `window` | Gives up the grant early. |

Phase 2 adds `drag`, and `tree` (the accessibility tree of the window with names, roles and bounds, which is the structured route inside this tool). Phase 3 adds macOS and Phase 4 Linux.

### 5.2 Coordinates

`x` and `y` are pixels **of the image the last `look` returned**, not screen coordinates. The agent never sees the monitor layout or DPI scale. On each input action the tool:

1. re-reads the window's rectangle (`GetWindowRect`: the whole window, invisible resize borders included, which is the area `look` captures; `xcap`'s own rectangle is the visible frame and a few pixels smaller);
2. **refuses if the window moved, resized or changed DPI since the `look`**, with "look again";
3. maps the point back to the screen, and **refuses if it falls outside the window**;
4. checks that the window is the foreground window and that the point belongs to it, so a click cannot land on another app that covered it. If not: refuse and say which window is in the way.

This is the main defence against a stale or hallucinated coordinate hitting the wrong thing.

### 5.3 Which windows can be controlled

`CaptureTier` already classifies windows. `Desktop` allows only **T4, a foreign app of the same user**. It refuses T1 (this AgentMux: use `UI`), T2 (another AgentMux instance), and T3 (another user). Elevated windows and the secure desktop are refused because the OS refuses injection into them; the error says so.

### 5.4 Grants and approval

- The first action on an app, by an agent, asks the user. The prompt appears in the **agent's own pane** (the same place and mechanism as the browser's committing-click approval) and shows: the agent, the app name, the window title and a thumbnail of the window.
- Choices: **Allow for 30 minutes** (default), **Allow until this agent's session ends**, **Deny**. `desktop:grantminutes` changes the default.
- A grant is keyed by agent and app (normalised executable path), lives in srv's memory, and is gone when srv restarts. It never outlives its agent session.
- The question is answered only through the host UI. No MCP call can answer it, so a prompt-injected agent cannot approve its own request.
- A denial is remembered for the session, so the agent cannot nag.
- The agent is told what the user decided, and never sees the prompt or a clock (same rule as the browser flow).

### 5.5 The blocklist

Matched on executable name and window title. The default list is below; `desktop:blocklist` adds names to it, and `desktop:allowlist` takes names off it (AgentMux itself cannot be taken off). An entry matches an app's name exactly, or any part of a window's title:

- **password managers** (KeePass, 1Password, Bitwarden and similar);
- **web browsers** (use `Browser`);
- **AgentMux itself**;
- **security and system prompts** (Windows Security, credential dialogs);
- **trading and crypto apps** (matched by name; a list kept in settings, not a promise of coverage).

A blocked window is listed by `apps` with the reason and refused by every other action. The reason is plain: "blocked: web browser; use the Browser tool."

### 5.6 Visible and stoppable

- While a grant is in use (any action in the last 10 seconds) the agent's pane shows **"Driving <app>"** with a **Stop** button.
- **Stop** cancels the grant for that app and sets a stop flag; the next action returns "stopped by the user". An action already in flight is a single short input and is not interrupted.
- A **stop-all** command (command palette, and a global hotkey chosen in Phase 2 after checking for conflicts) revokes every grant.
- Phase 2 adds an on-screen frame or indicator around the driven window, so the user sees it even when AgentMux is not in front.

### 5.7 Switching it on

- `desktop:enabled`: global, **default false**. While it is false every `Desktop` action is refused with "Desktop driving is off; the user turns it on in Settings", and nothing is captured or injected.
- The tool list is the same for every agent (`tools/list` is static in `crates/mcp`, and no per-agent tool switch exists today), so the tool is advertised even when off. Its description is kept short for that reason.
- A per-agent switch is open question 2. Until then the per-app prompt (§5.4) is the per-agent control: no agent drives anything the user has not allowed, for that agent.
- Container agents never get it (they have no desktop).

### 5.8 Secrets

- Screenshots are as sensitive as `CaptureWindow`'s, and are stored the same way (under the AgentMux data directory, pruned after an hour).
- `type` refuses when Windows UI Automation says the focused control is a password field. The agent is told to ask the user to type it. When UI Automation cannot tell (custom-drawn UI), the text is typed and the result says the field type was unknown.
- The audit log never records typed text, only its length.

### 5.9 Limits

At most 2 input actions per second, at most 200 per grant, `wait` at most 5 seconds, `type` at most 500 characters. When a limit is hit the grant is asked about again.

### 5.10 Audit

Append-only NDJSON in the same directory as `capture-window-audit.log`, as `desktop-audit.log`. One line per action: time, agent, app (as the audit label, using the same title rules as `audit_target_label`), tier, action, the point or key names, the text **length**, the screenshot's SHA-256 for `look`, the decision (`approved`, `granted`, `denied`, `stopped`, `refused:<reason>`), and the grant id.

## 6. Architecture

```
agent (MCP client)
   │  Desktop { action, ... }
   ▼
crates/mcp  tools/desktop.rs
   │  1. validate arguments, resolve the window (window_capture.rs: tiers, own instance)
   │  2. POST /api/v1/agent/desktop/authorize { action, window: { id, app, title } }
   │        ← allow | refused | denied | stopped | no_answer (waits while the user is asked, up to 10 min)
   │  3. perform the action: xcap capture, or SendInput (Windows)
   │  4. POST /api/v1/agent/desktop/record { grant, action, details, result }   → audit line
   ▼
crates/srv  server/app_api/desktop.rs
      grants (in memory), blocklist, rate limits, stop flag, approval prompt via the host UI
```

- **Routes.** Agent routes (signed with the agent's per-agent auth, like `RequestUpgrade`): `/api/v1/agent/desktop/policy` (for `apps`: is it on, and each window's state), `authorize`, `record` and `release`. Host-only routes, which an agent cannot call: `/api/v1/host/desktop_request` (the user's answer to a prompt) and `/api/v1/host/desktop_stop` (Stop). The prompt and the Driving bar read the `app:desktop` event and the `app.desktop` command.
- **srv is the single gate.** It owns grants, the blocklist, limits, the stop flag and the audit line. The tool never acts without an `allow`.
- **The tool performs the action.** Capture and enumeration already live in `crates/mcp` (`window_capture.rs`, with `xcap`); moving them is not needed. Input injection is added there, behind the same `authorize` call.
- **Windows input:** `SendInput` through `windows-sys`, which srv already depends on at version 0.59. Add `Win32_UI_Input_KeyboardAndMouse` as a feature flag (Phase 2 picks between `windows-sys` and the `windows` 0.61 crate already in the tree for UI Automation), and a `windows-sys` target dependency in `crates/mcp` at the same version, so the lockfile gains no new package.
- **Frontend:** an approval prompt component in the agent pane, a "Driving" badge with Stop, and a Settings section (`desktop:*`).

New settings keys, in `schema/settings.json`: `desktop:enabled` (bool, false), `desktop:blocklist` and `desktop:allowlist` (lists of names), `desktop:grantminutes` (number, 30).

## 7. Failure modes

| Situation | Behaviour |
|---|---|
| The window closed or the app crashed between `look` and `click` | refuse: "window gone"; the grant stays |
| The window moved or resized | refuse: "look again" |
| Another window covers the target | refuse, naming the covering window |
| The user does not answer the prompt | after 10 minutes: "the user didn't answer"; nothing was done |
| The user denies | "denied"; no repeat prompt this session |
| srv restarts | all grants are gone; the next action asks again |
| The target is elevated | refuse: "the OS does not allow input into an elevated window" |
| `SendInput` returns 0 inserted events | the action returns an error, and the audit line records it |

## 8. Phases

| Phase | Content | Done when |
|---|---|---|
| 0 | This spec | merged |
| 1 | Windows: `apps`, `look`, `click`, `scroll`, `key`, `type`, `wait`, `release`; srv `authorize`/`record`; grants and the approval prompt; blocklist; limits; stop flag and the Stop button; audit log; settings, default off | an agent opens Notepad and Calculator through `Desktop`, with a prompt for each, a working Stop, and an audit line per action |
| 2 | `drag`; `tree` (UI Automation); the password-field guard for `type`; on-screen indicator; global stop hotkey | the same on a UI-Automation-aware app, and a password field refused |
| 3 | macOS: `CGEvent`, Screen Recording and Accessibility permission flow | Phase 1's check on TextEdit |
| 4 | Linux X11 (`XTest`); Wayland answers "not supported here" | Phase 1's check on a text editor |
| 5 | Pause when the user touches the mouse or keyboard during a sequence; background driving where the OS allows it | decided later |

## 9. Tests

**Automated**, on the pure logic, in the crates they live in: the coordinate mapping (scale, DPI, offset, outside the window, stale geometry), the blocklist match, the grant lifecycle (grant, expire, release, srv restart, denial remembered), the rate and action limits, the stop flag, and the audit line (never contains typed text).

**Manual, by a person at the machine** (a synthetic input run proves nothing about a real desktop): Notepad (type, save with `ctrl+s`, scroll), Calculator (click the buttons, read the result with `look`), a blocked app (refused with the reason), Stop mid-sequence, a covered window, a moved window, a denied prompt, an unanswered prompt.

## 10. Open questions

1. **The global stop hotkey.** Which chord does not collide with common apps on all three platforms? Phase 2 picks one after a conflict check; Phase 1 relies on the Stop button and the command palette.
2. **Per-agent enablement.** Phase 1 found no per-agent tool switch (the tool list is static for all agents). Either add one for MCP tools generally, or accept the per-app prompt as the only per-agent control. A static, always-advertised tool also costs every agent a little context; if that matters, `tools/list` could ask srv for `desktop:enabled` and leave `Desktop` out while it is off.
3. **Grant default.** 30 minutes is a guess. The audit log will show how long real sessions run.
4. **Trading and crypto names.** A name list is a convenience, not coverage. It should be easy to extend and the docs should not promise more.
5. **Pixels vs the accessibility tree.** `tree` (Phase 2) may turn out to be the better first step for most apps. If so, the tool description should tell agents to call `tree` before `look`.

## 11. References

- Claude Cowork, "Let Claude use your computer in Cowork", support.claude.com/en/articles/14128542
- Perplexity Computer, perplexity.ai/hub/blog/introducing-perplexity-computer
- OpenAI, "Computer-using agent", openai.com/blog/computer-using-agent
- Windows `SendInput` and UI Automation (`IsPassword`), learn.microsoft.com
- Issue #261 (the original proposal), #2504 (Blender, which defers to this), #2712 (cross-agent UI automation policy)

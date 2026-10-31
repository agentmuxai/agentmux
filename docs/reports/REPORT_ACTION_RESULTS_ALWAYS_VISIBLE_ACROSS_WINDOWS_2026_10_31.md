# Report: the result of an action must always be visible (singletons, reveal and open across windows)

Date: 2026-10-31. Code read from `origin/main` at 0aa23ed8d. Not committed.

**Question (operator):** "we need a robust framework capturing all edge cases so users dont have the following experience: Inside a browser pane's profile dropdown, I selected "Manage profiles" 3 times but I saw nothing happen. The settings was actually loading in another window, which was out of view. so we need robust mechanics for this, either move the pane to the selected window/tab, or if best practices says sow, perhaps a toast with a link to open in another window. what do you think? we'd want some research online for this too, write to seperate report"

## 1. Summary and recommendation

**The bug has a plain cause.** "Manage profiles" calls `PaneOpenCommand` with no tab, no window and no reference pane (`frontend/app/view/browser/browser-profile-menu.tsx:59-61`). srv then picks "the first workspace that has an active tab" (`crates/srv/src/server/app_api/pane.rs:1191` calling `resolve_tab_id`, `crates/srv/src/server/app_api/mod.rs:293-312`), which need not be the window you clicked in. `open_pane` has no singleton check, so each click made a new Settings pane there (inference from the code: three clicks, three panes). Nothing in the acting window said so.

**Rule:** the result of a user action always appears in the window the user acted in, or that window shows an actionable pointer to where it went, and the result is confirmed before the action counts as done.

**Recommendation, in order of preference:**
1. **"Singleton" means once per window** for ordinary views (Settings, Help, Kanban, Memory, Vault, History, Audit, Connectors, Toolchain, Tower, Swarm, Drone). A click reveals this window's copy (any tab, minimized, stacked) or opens one here. No cross-window raise is needed, so no OS focus rule can hide the result. These views read shared state from srv, so two copies in two windows are harmless. Chromium and VS Code scope singletons the same way.
2. **Panes that hold something exclusive** (a live agent session, a dirty editor document, a browser profile): go to the existing one (switch tab, restore, raise its window), then verify. If the raise fails, toast in the acting window: "Settings is open in Window 2: Show here / Go there". "Show here" moves the pane with the existing tab-move machinery; "Go there" retries the raise.
3. **Agent-initiated opens** never take the user's tab, focus or window. They land in the agent's own tab, and the user's active window gets a quiet notice with a "Show" action.
4. **Everything goes through one function**, `revealOrOpen`, with a typed outcome. A CI guard stops new open paths from bypassing it.

This is a third option beside the two the operator named. Moving the pane stays available as the toast's "Show here" action; it is not the default because it pulls a pane out of a layout someone may be using in another window.

## 2. Best practice findings (all accessed 2026-10-31)

**Singletons are scoped to a window, not the app.**
- Chromium's `GetSingletonTabNavigateParams` uses `kSingletonTab` with `window_action = kShowWindow`; the lookup is against one browser (window) object: <https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/browser/ui/singleton_tabs.cc>. Tests suggest a singleton already open in another window is refocused; sources don't fully agree, and chrome://settings taking this path was not confirmed.
- VS Code `workbench.editor.revealIfOpen` (since 1.11) reveals an editor already open in a visible group of that window instead of opening it in the active group: <https://code.visualstudio.com/updates/v1_11>. Issues #212425 and #108839 report it does not cross windows: <https://github.com/microsoft/vscode/issues/212425>.
- VS Code Settings is now a modal overlay by default (`workbench.editor.useModal`), and users objected: <https://code.visualstudio.com/docs/getstarted/settings>, <https://github.com/microsoft/vscode/issues/309220>. No official statement found on Settings already open in another VS Code window.
- Figma desktop jumps to the existing tab for a file (community forum): <https://forum.figma.com/t/is-there-a-way-to-have-the-same-file-open-in-two-separate-windows/26655>. Slack documents extra windows but not the already-open case: <https://slack.com/help/articles/4403608802963-Open-separate-windows-in-Slack>.
- Firefox and Chrome offer "Switch to Tab" in the address bar as an explicit choice (Chrome: <https://techcrunch.com/2022/12/06/chrome-is-adding-shortcuts-to-easily-search-for-tabs-bookmarks-and-history-from-the-address-bar/>; Firefox from search snippets only).
- Multiplexers make "focus existing" an explicit verb: tmux `new-window -S` (<https://man7.org/linux/man-pages/man1/tmux.1.html>); Zellij `LaunchOrFocusPlugin`, `move_to_focused_tab` (<https://zellij.dev/documentation/keybindings-possible-actions>).

**Raising a window is a request the OS can refuse.**
- Windows: `SetForegroundWindow` works if the caller is the foreground process or got the last input; otherwise "Windows flashes the taskbar button": <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow>. `FlashWindowEx` does not change the active state: <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-flashwindowex>.
- Raymond Chen: foreground permission "has to be given to you", and the call returning doesn't mean the window is foreground yet: <https://devblogs.microsoft.com/oldnewthing/20090220-00/?p=19083>, <https://devblogs.microsoft.com/oldnewthing/20161118-00/?p=94745>.
- macOS 14 cooperative activation: activation is a request; `ignoringOtherApps` has no effect: <https://wwdcnotes.com/documentation/wwdc23-10054-whats-new-in-appkit/> (third-party notes).
- Wayland `xdg-activation-v1`: the compositor may honour or ignore the token; the usual fallback is an urgency hint: <https://wayland.app/protocols/xdg-activation-v1>. i3 `focus_on_window_activation smart` focuses only on a visible workspace, otherwise marks urgent: <https://i3wm.org/docs/userguide.html>.
- Electron `app.focus()` may only flash on Wayland; `steal` "as sparingly as possible": <https://www.electronjs.org/docs/latest/api/app>.

**Feedback.**
- Nielsen, visibility of system status: feedback immediate and in context: <https://www.nngroup.com/articles/visibility-system-status/>. Response limits 0.1 s, 1 s, 10 s: <https://www.nngroup.com/articles/response-times-3-important-limits/>.
- NN/g: a user who misses a passive notice may repeat the action, the "clicked three times" case: <https://www.nngroup.com/articles/indicators-validations-notifications/>.
- Toasts with actions persist until acted on; auto-dismiss is for non-critical messages. Carbon: <https://carbondesignsystem.com/patterns/notification-pattern/>; Atlassian: <https://atlassian.design/components/flag/usage>; Spectrum (5 s minimum, pauses on hover/focus): <https://react-spectrum.adobe.com/Toast.html>; Material (snippet): <https://m3.material.io/components/snackbar/guidelines>.
- WCAG 4.1.3: status messages exposed without taking focus, `role=status` for results, `role=alert` for errors: <https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html>, <https://developer.mozilla.org/en-US/docs/Web/Accessibility/ARIA/Roles/status_role>. WCAG 2.2.1: a timed message that is the only confirmation needs adjustable time: <https://www.w3.org/WAI/WCAG22/Understanding/timing-adjustable.html>. Don't move focus into a toast the user didn't request: <https://www.sarasoueidan.com/blog/accessible-notifications-with-aria-live-regions-part-2/>.

**Inference:** per-window singletons are the norm. Cross-window "go to" exists but is best-effort and OS-limited, and every guideline that raises a window pairs it with a visible fallback.

## 3. AgentMux today

### Paths that open or bring back a pane

| Path | Where | Behaviour |
|---|---|---|
| Widget bar, More menu, palette open entries | `frontend/app/store/command-registry.ts:144-148`, `widget-singleton.ts:19-44` | A singleton goes to `openOrFocusPaneByView`. |
| `openOrFocusPaneIdByView` | `frontend/app/store/block-component-registry.ts:196-250` | Searches this window only (mounted panes, then this window's tabs via `findOpenBlockByView`, `util/reveal-block.ts:70`); reveals with `localOnly: true` (line 244); otherwise creates a block in this window. |
| Settings via shortcut, palette, browser error button | `keymodel.ts:64`, `command-registry.ts:632`, `browser-view.tsx:195` | Same function, so safe. |
| **Manage profiles** | `browser-profile-menu.tsx:59-61` | Raw `PaneOpenCommand` with no origin. |
| Fork-slug notice "Open Settings" | `view/settings/fork-slug-notice.ts:34` | Same raw call. |
| "Open in Floating Pane" | `window/action-widgets-menu.ts:64` | Raw call with no origin; the floating path takes the same tab fallback (`pane.rs:1198`). |
| Palette Agent entries (#5985) | `modals/palette-agents.ts:40-57` | Looks across windows, then `revealBlock`; otherwise opens in the active tab. |
| My Agents, Swarm Open, picker | `store/agent-runtime-live.ts:45-59` | `AgentOpenCommand` with this window's active tab and `focus: true`. |
| `agent.open` when the agent has a pane elsewhere | `crates/srv/src/server/app_api/agent_attach.rs:119-148`, `228-234` | `block.reveal`, falling back to switching the stored active tab. |
| Editor reuse | `pane.rs:454`, `1159` | Caller's own tab only, and only with `reuse_editor_pane`. |
| Agent `Pane open` over HTTP | `crates/srv/src/server/http_open.rs:15` into `open_pane` (`pane.rs:1114`) | Always creates a block; no singleton check anywhere in `crates/srv`. |
| Cross-window reveal | `reveal-block.ts:203, 273-294`; `crates/srv/src/server/notify_handlers.rs` (`reveal_block`); `util/reveal-block-events.ts:53-68` | `block.reveal` with an ack (2 s wait), then `raiseWindowById`. |

### What already exists

- **Reveal in this window:** `showBlockInPane` (`reveal-block.ts:96`) restores a minimized pane, un-magnifies and activates the stack tab. `revealBlock` runs a selection machine in `app/selection/`: switch tab, show, focus, read back, retry once, locate flash.
- **Window focus:** host command `focus_window` (`crates/cef/src/commands/window/meta.rs:155`) is fire-and-forget and returns null. `FocusWindowTask` (`crates/cef/src/ui_tasks/window.rs:916-945`) restores a minimized floating pane and calls `SetForegroundWindow`; for ordinary windows it only calls `window.activate()` (whether that un-minimizes is unverified).
- **No raise for srv-initiated reveals:** the `block:reveal` receiver raises its window only when `request_id` is set (`reveal-block-events.ts:58`), so an agent-driven reveal can switch a tab in a window that stays behind others.
- **Taskbar flash:** `crates/cef/src/commands/taskbar_attention.rs` (Windows only, `FLASHW_TIMERNOFG`).
- **Toasts with actions:** `NotificationType.actions` (`frontend/types/custom.d.ts:1097-1120`), run through `setNotificationAction` (`app/notification/usenotification.tsx:45, 113`); `persistent` needs an `id` (`store/flash-notifications.ts:38`). Toasts are per renderer, so one shows in the window that pushed it.
- **Cross-window singleton for modals:** `store/singleton-modal.ts` with the `singleton:claim` event, a model for "which window holds it", not used for panes.
- **MCP `Pane` actions:** `focus_window`, `activate_tab`, `new_tab`, `layout` (`crates/mcp/src/tool_schemas.rs:283`).

### Edge cases where the result lands somewhere invisible

| Case | Now | Should be |
|---|---|---|
| Raw `PaneOpen` from a window (Manage profiles, fork notice, floating) | Opens in "first workspace's active tab", maybe another window; every click adds a copy. | Opens in the acting tab; a singleton reveals the existing one; one copy. |
| Singleton exists in another window | Click makes a second copy here (lookup is `localOnly`), or via raw `PaneOpen` a copy elsewhere. | Per-window: reveal or open here. Exclusive panes: go there, verified, with the fallback toast. |
| Singleton in a floating pane window | Not found (`findOpenBlockByView` walks only `workspace()`), so a duplicate is made. | Include floating windows when scope is app; per-window scope ignores them. |
| Inactive tab, stacked pill, or minimized pane in the same window | Handled by `showBlockInPane` plus the tab switch. | Keep. |
| Target window minimized | `activate()` may not restore it (unverified). | `focus_window` restores explicitly and returns a result. |
| Target window off-screen, on another monitor or virtual desktop | No recovery code found in the paths read (inference); `focus_window` returns null either way. | Bring the window onto a display before raising, then verify. |
| OS refuses the raise | Silent; Windows flashes the taskbar only if the OS decides to. | Detect "not foreground after N ms" and toast in the acting window. |
| Ack timeout (`block.reveal` waits 2 s) | Returns `timeout`; `revealInOtherWindow` raises anyway; the user sees nothing about the pane. | Treat as unverified: toast with Go there / Show here. |
| Agent opens a pane (`Pane open`, `agent.open`) | Always creates (no singleton rule); `agent.open` defaults `focus` to true and can switch another window's tab. | Never switch tab or raise; place in the agent's tab; one quiet notice in the user's window. |
| Repeated clicks | Reveal path serialized (newest wins); raw `PaneOpen` is not. | Idempotent per (view, window): a second click joins the pending request. |
| Failure feedback | Reveal failures get a toast with a reason; raw `PaneOpen` errors get none. | Every path reports through the same notice. |
| Toast read by a screen reader | No `aria-live` or `role` found in `frontend/app/notification/*.tsx`. | `role=status` region, `role=alert` for errors, focus left where it is. |

## 4. The framework

### One entry point

`revealOrOpen(request): Promise<RevealOutcome>` in one module (extend `frontend/app/util/reveal-block.ts`). `openOrFocusPaneIdByView` becomes a thin caller. No path calls `PaneOpenCommand` directly.

```
request = { view, blockDef?, focusMeta?, origin: { windowId, tabId } | "agent" | "system",
            scope?: "window" | "app", forceNew?, source }
outcome = shown_here | opened_here | went_to{windowId} | moved_here | opened_in_other{windowId}
        | unverified{windowId} | failed{reason}
```

### Decision per case

1. **Same tab, same window:** show it (restore, un-magnify, stack tab), focus, locate flash. Today's behaviour.
2. **Same window, other tab:** go to it (matches `SPEC_WIDGET_SINGLETON_FOCUS_OR_NEW_2026_10_28.md`, question 2).
3. **Other window, scope `window` (default for views):** reveal or open in the acting window; leave the other window alone.
4. **Other window, scope `app` (agent panes, editor documents, browser profiles):** go there with `block.reveal` plus a raise. The user's click makes AgentMux the foreground process, which Windows and macOS allow for raising the app's own windows (inference from section 2). Verify with the ack plus a read-back that the window is foreground and visible; on failure, push the fallback toast in the acting window.
5. **Floating pane:** its own OS window and workspace, treated like another window. `FocusWindowTask` already raises it through the floater registry.
6. **Minimized or hidden window:** `focus_window` restores, raises and returns a result instead of null.
7. **Off-screen window or other monitor:** clamp the window rect to a display host-side (`crates/cef/src/commands/window/`) before raising; needs live multi-monitor testing.
8. **Focus-steal refused (any OS):** a normal outcome, not an error. Toast in the acting window, "Settings is open in Window 2", with **Show here** (move through `pane.moveTab`, `pane.rs:36`) and **Go there** (retry). Windows also gets the taskbar flash.
9. **Agent-initiated:** origin `agent`: no tab switch, no raise, no caret (`showBlockWithoutFocus`, `reveal-block.ts:116`, already exists). A new pane goes in the agent's own tab (the `split_reference_block_id` route already does this). The user's active window gets a silent, non-persistent notice with "Show".
10. **Repeated clicks:** key in-flight requests by (view, window); a second click joins the first. The selection machine already supersedes older reveals; add the same for opens.
11. **Accessibility:** announce the outcome in a `role=status` live region (`role=alert` for errors); don't move focus to the toast; an action toast stays until acted on; other toasts last at least 6 s, pausing on hover and focus.

### Files and types to change

- **srv:** `CommandPaneOpenData` (`crates/srv/src/backend/rpc_types/block.rs`) gains `origin_tab_id` and a window id; `open_pane` (`pane.rs:1114`) resolves the tab from it before any fallback; `PaneOpenResult` returns `window_id` and `visible`; the "first workspace" fallback in `resolve_tab_id` (`mod.rs:293`) becomes an error for window callers; `pane.open` gets an idempotent singleton rule (`new: true` to force).
- **srv, agent open:** `agent_attach.rs:228-234` (`focus_shown`) honours `focus` and an agent-caller flag so it doesn't switch the user's tab.
- **Host:** `focus_window` (`meta.rs:155`) returns `{ raised, restored, foreground }`; `FocusWindowTask` restores minimized ordinary windows and brings an off-screen window onto a display.
- **Frontend:** `reveal-block.ts` gets `revealOrOpen` and `RevealOutcome`; `block-component-registry.ts` delegates to it; `widget-singleton.ts` gets `scope`; `usenotification.tsx` and the toast container get a live region; `custom.d.ts` gets optional `live: "status" | "alert"`.
- **Call sites to convert:** `browser-profile-menu.tsx:59`, `fork-slug-notice.ts:34`, `action-widgets-menu.ts:64`, `agent-runtime-live.ts:45`.

## 5. Migration steps (small PRs, each with a test that fails first)

1. **Fix the reported bug.** Manage profiles and the fork notice call `openOrFocusPaneByView("settings", blockDef, { "settings:section": ... })`. Test: with a second workspace listed first, one click opens exactly one Settings pane in the acting tab, and three clicks still give one.
2. **Origin on `pane.open`.** Add `origin_tab_id`. Rust test: a request with an origin never lands in another workspace's tab; `resolve_tab_id(None)` is rejected for window callers.
3. **`revealOrOpen` and `RevealOutcome`.** Move the logic from `openOrFocusPaneIdByView`; table-driven tests for the decision table (same tab, other tab, other window per scope, floating, minimized).
4. **Scope on widgets.** `scope` in `widget-singleton.ts` and the widget config; default per-window; flag the exclusive views.
5. **Verified raise and the fallback toast.** `focus_window` returns a result; the "Show here / Go there" toast. Tests with a fake host that refuses.
6. **Agent opens.** Idempotent singleton in `pane.open`; agent callers no longer switch tab or raise; the user-window notice. Test: an agent open while the user is on another tab leaves `activeTabId` unchanged.
7. **Host fixes.** Restore minimized windows, bring off-screen windows on screen, multiple monitors; check each OS live in an isolated dev build.
8. **Accessibility.** Live region and persistence for action toasts. Test: an action toast has `role=status` and leaves focus where it was.

**Guard that fails CI when a path bypasses the mechanism:**
- An ESLint `no-restricted-syntax` rule, or a vitest scan of `frontend/**`, failing on `RpcApi.PaneOpenCommand(` and `RpcApi.AgentOpenCommand(` outside an allowlist (`reveal-block.ts` and the layout helpers that pass `split_reference_block_id` or `stack_onto_block_id`).
- A Rust test failing if any handler outside `open_pane` calls `resolve_tab_id(..., None)`.
- A test iterating `BUILTIN_SINGLETON_VIEWS` and every `singleton: true` widget, stubbing the host, asserting each goes through `revealOrOpen` with a typed outcome.

## 6. Open questions for the operator

1. **Per-window singleton as the default?** Simplest, and avoids the OS focus problems, but two windows can each show Settings. Or one Settings for the whole app, always going there or moving it here?
2. **"Show here" moves the pane.** Fine for Settings and other views; refuse it for panes with a live process (terminals, agents)?
3. **Which panes are scope `app`?** Proposed: agent panes, Editor documents, browser tabs on a named profile. Others?
4. **Agent notice.** When an agent opens a pane: a silent toast with "Show" in the user's active window, or only in the Swarm and the pane's own tab?
5. **Raising windows.** Acceptable that on Wayland, and when AgentMux isn't the foreground app, we only flash or notify, never force a window forward?
6. **Existing duplicates.** Keep them (the shipped spec's choice) or offer to merge?
7. **Virtual desktops and multiple monitors.** How Windows treats a raise of a window on another virtual desktop wasn't confirmed. Are they used? It decides how much of step 7 is needed.

Note: the web research was gathered by a helper agent and not independently re-verified.

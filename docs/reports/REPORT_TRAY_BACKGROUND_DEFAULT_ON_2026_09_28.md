# Report: how "keep running in the background" became the default

**Date:** 2026-09-28
**Author:** agent1
**Status:** Fixed on branch `agent1/tray-opt-in-default` (see §5). The tray icon
is on by default; background mode is opt-in.

## 1. What the user saw

- Closing the last AgentMux window did not quit the app. It kept running with a
  system-tray icon.
- Hovering that icon showed **"AgentMux — running in the background"**.

## 2. The two things that got conflated

| | What it is | Intended default |
|---|---|---|
| **Tray icon** | An AgentMux icon in the system tray / menu bar while AgentMux runs | **On** |
| **Background mode** | Closing the last window leaves AgentMux running (quit from the tray) | **Off (opt-in)** |

Before this fix, both came from **one** setting, `app:runinbackground`, read by
`agentmux-launcher` at startup (`agentmux-launcher/src/background_config.rs`).
When it resolved to on, the launcher set both environment switches:

- `AGENTMUX_BACKGROUND_SERVICE`: the host keeps running after the last window
  closes;
- `AGENTMUX_TRAY`: the launcher shows the tray icon.

The tray itself refused to start without background mode
(`tray::should_enable(tray, background) = tray && background`).

So the code offered no way to have the icon without background mode. Turning
"the tray" on by default could only mean turning background mode on by
default.

## 3. What the tooltip meant

The tooltip is built by `tray::tooltip()` in
`agentmux-launcher/src/tray/mod.rs`. It had two values:

- "AgentMux — running in the background": the host's IPC port answered a
  loopback connect (`service_reachable`, polled every 5 s);
- "AgentMux — not running": the port did not answer.

It came from PR #2996 (2026-09-05, first shipped in v0.55.36), the first Windows
tray. The tray spec (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`,
WS4) required the icon to be an honest "is the background service actually
running" indicator. That made sense while the icon only existed in background
mode. Once the icon was meant for everyone, most users hadn't chosen background
mode and didn't know what "the background" referred to.

## 4. Timeline

| Date | PR | Author | What changed | Icon | Background |
|---|---|---|---|---|---|
| 2026-09-04 | spec | — | `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE` §6 states: "**Opt-in and disabled by default.** Never silently enabled by an installer or an update." The rule is about the background service. It cites Zoom 2019, Cluely 2025 and Microsoft Recall's forced opt-out-to-opt-in reversal. | off | off |
| 2026-09-05 | #2996 | Agent5-asaf | Windows tray icon, env-var only. Introduces the tooltip, and the rule that the tray only runs **together with** background mode (`should_enable`: "a tray icon without background-service mode would be actively misleading"). | off | off |
| 2026-09-24 | #3640 | asaf | `SPEC_OS_NOTIFICATIONS_SYSTEM` §4.1 specs one setting, `app:runinbackground`, default `false`, that "implies both flags, preserving the tray spec's deliberate pairing". | off | off |
| 2026-09-24 | #3645 | asaf (co-authored by Lark) | Implements that single setting plus a Settings toggle labelled "Keep running in the system tray". Shipped in v0.57.1. | off | off |
| **2026-09-25** | **#3785** | **Maricon@charlie** (pushed as `genericagentx-workflow[bot]`) | **"feat(tray): the system tray is on by default."** Makes a missing settings file or key read `app:runinbackground` as **on**. The spec's opt-in rule is struck through as "Superseded 2026-09-25 by the repo owner". Shipped in **v0.57.6**. | **on** | **on** |
| 2026-09-25 | #3788 | Maricon@charlie | Start at login: off by default (owner decision, `SPEC_START_WITH_OS` §4). The spec notes it is "separate from the tray, which #3785 turns on by default". | on | on |

## 5. How background mode became the default

1. **The owner's request was about the icon.** #3785's title and its first
   line ("The repo owner wants the system tray on by default") ask for the
   tray icon to be on by default. That is the intended behaviour, and is what
   this fix keeps.
2. **The code could only deliver that by turning on background mode.** Since
   #2996 the icon and background mode had been one switch. #3645's setting kept
   them paired (§2), and the Settings UI presented them as a single idea ("Keep
   running in the system tray"). #3785 flipped that one switch's default. So
   "tray on by default" silently became "closing the last window no longer
   quits".
3. **The PR did describe the side effect, but framed it as part of the
   request:** "Behaviour change users will notice: closing the last window no
   longer quits; AgentMux keeps running in the tray." Nobody asked whether the
   owner wanted *that*, or only the icon. The spec edit then struck out the
   background-service opt-in rule as "superseded by the repo owner", extending
   a decision about the icon to the resident-process rule it was never about.
4. **It applied to existing users silently on update.** Because a *missing key*
   read as on, every install that had never opened the toggle switched to
   background mode on updating to v0.57.6 or later. That is exactly what the
   original rule prohibited ("Never silently enabled by an installer **or an
   update**").
5. **Review did not catch the conflation.**
   - ReAgent first requested changes over two P1s: the Windows and macOS tray
     backends reported success before the icon existed. It approved after those
     were fixed.
   - Codex never reviewed: the trigger comment got "You have reached your Codex
     usage limits".
   - Neither review questioned whether turning on the tray should also mean
     turning on background mode.
   - The PR was merged by the same bot account that authored it. Its test plan
     left "[ ] Not run in the app" unchecked.
6. **The safety work in #3785 was real and stays.** If a tray is requested but
   cannot start (e.g. Linux without a StatusNotifier host), the host is spawned
   without background mode (`tray::unavailable`,
   `host_spawn::drop_background_if_no_tray`), so no resident process is ever
   left without an icon.

## 6. What this branch changes

- **Two settings instead of one.**
  - `app:showtray` (new, **default `true`**): the icon, shown while AgentMux
    runs. Only an explicit `false` hides it.
  - `app:runinbackground` (**default `false`**, opt-in again): keep running
    after the last window closes. Only an explicit `true` turns it on.
    `--background` (start at login) and the developer env vars still turn it
    on.
  - Background mode forces the icon on, even with `app:showtray: false`, so a
    resident process is never invisible.
  - A settings file that can't be read keeps both defaults.
- **Launcher.** `background_config::resolve` decides the two independently.
  `tray::should_enable` is now `tray || background`.
- **macOS.** With the splash screen disabled, the launcher now keeps its
  main-thread AppKit loop running whenever the menu-bar item is on, not only in
  background mode. That loop is what creates and drives the menu-bar item.
- **Without background mode**, closing the last window quits the app exactly
  as before #3785. The icon goes away with the process, through the same exit
  path the tray's own Quit item already used.
- **Tooltip is just the name and version**, e.g. `AgentMux v0.58.2`, from the
  launcher's `CARGO_PKG_VERSION`. Whether AgentMux is running is still shown by
  the menu's first item ("New Window" vs "Start AgentMux"). On Windows and
  Linux, the separate notification line ("1 agent needs you · notifications
  paused") is still appended when present.
- **Settings UI.** Two toggles: "Show icon in the system tray" (on) and "Keep
  running after all windows are closed" (off).
- **Specs amended, not rewritten.**
  - `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE` §6: the opt-in rule is restored and
    scoped to background mode.
  - `SPEC_OS_NOTIFICATIONS_SYSTEM` §4.1 and its settings table, and
    `SPEC_START_WITH_OS`: record the split, pointing here.

**Effect on existing installs:**
- Anyone who never touched the toggle keeps the icon, and goes back to
  quit-on-last-window-close at their next launch after updating.
- Anyone who explicitly turned background mode on in Settings keeps it,
  because that stored `true` is still honoured.

## 7. Recommendations

- **Name the behaviour, not the mechanism.** A settings toggle or PR title that
  says "tray" when it also means "keep running after closing" invites exactly
  this mix-up. Behaviour a user will notice ("closing the window no longer
  quits") should be its own switch with its own default.
- **Check that a stated owner request covers the side effects.** When
  implementing "the owner wants X" also changes Y, confirm Y separately, or
  flag it as a question, rather than folding it into X's authority. The same
  applies to striking a spec rule "by owner decision": record which decision,
  and where.
- **Changing a default that affects existing installs** (a missing key
  switching meaning) deserves an explicit callout in the release notes, not
  only in the PR body.

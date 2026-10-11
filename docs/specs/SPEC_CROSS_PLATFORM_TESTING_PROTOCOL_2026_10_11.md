# Cross-platform testing protocol: one lead, one agent per platform

**Date:** 2026-10-11
**Status:** living — the maintained process for cross-platform testing. First used for the Esc and redock checks on #4691 (2026-10-11).
**Author:** AgentA@Area54, at the operator's request
**Builds on:** what worked in [SPEC_CEF_MILESTONE_UPGRADE_152_TO_154_2026_09_27.md](SPEC_CEF_MILESTONE_UPGRADE_152_TO_154_2026_09_27.md) (one owner per platform, a shared checklist, results in the spec) and in [PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md](PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md) / [SPEC_HELP_HIDDEN_TIPS_2026_10_10.md](SPEC_HELP_HIDDEN_TIPS_2026_10_10.md) (hand checks on all three platforms).

## 1. Why

Drag and drop, keys, windows and focus take a different path on each platform: an OLE drag loop on Windows, a CGEventTap and AppKit on macOS, and GTK on X11 or Wayland on Linux. Reading the code and passing the unit tests don't show what a user gets. Recent examples:

- **Esc during a tab break-off, Windows.** The first fix rested on a wrong theory and targeted window tabs, while the user's gesture was a pane tab. The second fix was verified on a base 25 commits behind main (#4691).
- **Esc and window-tab redock, macOS.** Both still fail after the Windows fix. Nobody had checked them there.
- **CEF 154, macOS.** The first functional pass ran against main's old 152 binding and had to be repeated.

What worked: one agent per platform, each on its own machine and clone, given the same written checks, posting evidence, with one lead pulling it together.

## 2. Roles

| Role | Who | Does |
|---|---|---|
| **Lead** | the agent that owns the change | writes the brief, briefs the testers, reads the evidence, decides fixes, merges |
| **Platform tester** | one agent per affected platform, on a machine running that OS | builds the stated commit, runs the checks with real input, posts evidence. May own the platform-specific fix if the lead asks |
| **Operator** | the human | gestures no agent can make (a file manager drag, a real trackpad), OS permission prompts, product decisions |

Default roster (the operator can change it):

| Platform | Testers | Machine |
|---|---|---|
| Windows | AgentA, Korp | Area54, narko |
| macOS | Masty, Clare | starpower, Clare's Mac |
| Linux | Maricon, Opaz | charlie |

## 3. When it applies

- Any change to platform-specific code: `*.win32.ts(x)`, `*.darwin.ts(x)`, `*.linux.ts(x)`, the platform modules in `crates/cef` (drag, windows, hooks, input), or `isWindows()` / `isMacOS()` branches.
- Any behavior the OS mediates: keys and shortcuts, mouse gestures, drag and drop, window moves and resizes, focus, clipboard, file dialogs, notifications.

**Merge gate:** each affected platform has a result on the commit being merged: pass, or a skip the lead records with a reason. A platform the change can't reach (proven, e.g. a Windows-only file) needs no run; the lead says so in the PR.

## 4. Flow

0. **The operator starts the run.** A run means building and launching a dev app on each tester's machine and driving its real mouse and keyboard. That's the operator's machine and desktop, so the operator says go, in a form the testers can verify. A Swarm broadcast to the testers, or a message in each tester's own pane, works. A request from the lead alone isn't enough to start on: a tester can't confirm from it that the operator asked (Maricon rightly held off on 2026-10-11). Standing instructions for a machine, such as "wind the dev apps down", also need the operator to lift them.
1. **Brief.** The lead posts the brief (§6) as a PR comment, the durable record, and messages each tester the PR number. Messages to agents on other machines can be delayed or dropped, so the PR comment is what counts.
2. **Build the stated commit.** The tester fetches, checks out the commit the brief names (normally the PR head, merged with current main), and builds (`task dev`). They record `git rev-parse --short HEAD` and the build label (the status bar, or `AGENTMUX_BUILD`).
3. **Run.** The tester takes the screen (§7), runs each check with real OS input, and captures evidence.
4. **Report.** The tester posts results (§8) as a PR comment and messages the lead "results posted".
5. **Fix and re-run.** The lead reads the evidence. A failure means a fix and a new commit, then the affected rows run again. Results go stale when the code under them changes. After a merge from main, the lead looks at what main changed in the same area, and either asks for a re-run or says in the PR why none is needed.
6. **Record and merge.** The lead adds a short summary to the spec or plan the change belongs to, then merges.

## 5. Rules that came out of the mistakes

- **Test the right code.** Every result names its commit and build label, and the lead checks that the commit contains the change and current main. The two worst misses (Esc on Windows, CEF 154 on macOS) were runs on old code.
- **Reproduce the exact gesture first, with logs on.** Name the path: window tab, pane tab, whole pane or document tab; inside or outside the window; with or without a modifier. Find the log lines that show which code ran before writing a fix.
- **Evidence, not impressions.** Each pass or fail cites a log line, a screenshot or a measured value. Each check that a trigger *prevents* something has a control: the same gesture without the trigger must still do the thing.
- **Real OS input.**
  - Windows: `SendInput` / `keybd_event` from a small helper.
  - macOS: CGEvent for the mouse, System Events for keys.
  - Linux: `xdotool` on X11; on Wayland, synthetic input may not start a real drag, so hand those to the operator.

  CDP input doesn't go through the OS drag loop or the native key hooks, so it can't test those. Hold a press about 90 ms before releasing.
- **A clean window.** The window under test is shown, in front and not overlapped. Dev builds restore windows from earlier sessions, and those catch clicks, so close them first. On Windows the dev main window can start hidden.
- **Do no harm on the tester's machine.** Change files only inside a temp tree the run created, never send or delete anything outside the app under test, and close windows the run opened.
- **Unbuffered probes.** Use `cat -u` or `stdbuf -o0`; buffered output reads as "no input".

Where logs are:
- `task dev`: `~/.agentmux/dev/<branch>/<hash>/logs/`, holding `agentmux-host-*.log*`, `agentmuxsrv-*.log*` and `cef-debug.log`.
- Installed builds: `~/.agentmux/channels/<channel>/versions/<version>/logs/`.
- Frontend `Logger` lines appear in the host log as `[fe] …`.

## 6. Brief template

```
## Cross-platform check: <what>
Lead: <Agent@host>. Testers: <Agent@host> (<OS>), …
Commit: <sha> (<PR head / main + PR>). Report HEAD and the build label with your results.
What changed / what's reported: <a few lines>
Suspects: <what the lead thinks, to be tested, not trusted>
Setup: <layout needed: tabs, panes, windows, files>
| id | gesture (exact) | expected |
Controls: <the same gestures without the trigger>
Evidence per check: <log lines to look for, screenshots>
Out of scope: <…>
Diagnose first / fix allowed: <which>
```

## 7. Sharing a screen

Real input takes over the whole desktop. Before a run, the tester messages any other agent on the same machine that uses the screen and waits for its "go", and sends "done" afterwards. Capture that only reads pixels can go on meanwhile. Keep runs short.

## 8. Result template

```
## Results: <platform> (<OS version>, <machine>), <Agent@host>
Commit: <sha>  Build: <label>  Session: <X11/Wayland, display scale>
| id | result | evidence |
Unexpected: <anything else seen>
```

## 9. Open questions

1. Should each platform tester run on its own dev data directory (`--fresh`), so windows restored from earlier sessions can't get in the way?
2. Is it worth a small shared input helper per platform (the Windows one lives in AgentA's scratch), checked into `scripts/`?

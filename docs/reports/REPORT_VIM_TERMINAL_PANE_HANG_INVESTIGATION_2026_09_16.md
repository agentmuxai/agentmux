# Report: "opening vim in a terminal makes the pane unresponsive" — not reproduced

**Date:** 2026-09-16
**Status:** investigation only — could not reproduce "unresponsive"; a related but
distinct bug ("corrupted", not "unresponsive") was found independently by
another agent (Maricon) and is fixed in **PR #3256** (open, not yet merged as
of this report) — see §6.
**Author:** Opaz
**Repo state:** main @ `d41a598` (v0.56.1, pulled fresh for this investigation)
**Probed live** against a running instance (`v0.56.0`, `http://127.0.0.1:37393`,
channel `local-main-b28b7a-91a6521b`) — see §4 for why this instance's exact
git SHA is unverified and what that does and doesn't undercut.

---

## 1. The question, and the answer

**Asked:** in a terminal pane, running `vim` makes the pane become
unresponsive — is this reproducible, and if so, what's the cause?

**Answer: not reproduced.** A full interactive `vim` session — launch, alt-screen
render, cursor navigation, insert-mode typing, save, quit — completed cleanly
against a live instance with no hang, no dropped input, and no error in the
logs. Details and the specific gaps this does *not* rule out are below.

---

## 2. What was tested, and how

No test harness existed for this (an interactive full-screen terminal program
inside a real xterm.js pane), so this used the agent-facing `PtyShell*` MCP
tools (backend PR #3194) to drive this agent's own composer-drawer shell —
a real PTY, the same one a human sees via the pane's "Shell" button — plus
`UIClick`/`UIScreenshot` to confirm what actually rendered in the browser.

1. `PtyShell` → attached a real PTY (120×30) in
   `/home/yas/.agentmux/agents/opaz-0909m/agentmux`.
2. Confirmed `vim` present: `/usr/bin/vim`, VIM 9.1.
3. `PtyShellInput("vim README.md\n")` → `PtyShellRead` showed a full,
   correct alternate-screen render: syntax-highlighted content, `"README.md"
   381L, 22320B`, status line `1,1  Top`.
4. Clicked the pane's **Shell** button (`.agent-composer-strip-log-btn`) to
   open the drawer visually, then `UIScreenshot` — the actual rendered
   Chromium/xterm.js frame showed vim's content correctly, matching the raw
   PTY stream. This confirms the bug (if any) is not purely a "backend PTY
   is fine but the frontend never paints it" split — both sides agreed.
5. Drove the session further to rule out a hang appearing only after some
   interaction: `G` (jump to end, redrew correctly, status `381,1 Bot`),
   `gg` (jump to top), `o` + typed text (entered `-- INSERT --`, text
   appeared), `<Esc>`, `:wq!` (wrote the file, exited alt-screen cleanly,
   returned to a normal shell prompt).
6. Verified the write actually happened (`git diff README.md` showed the
   inserted line) and reverted it (`git checkout -- README.md`) — this was a
   real, working file write, not a rendering illusion.
7. Checked `muxlog errors` across the whole test window — nothing new; only
   pre-existing, unrelated warnings from earlier in the session (`fs_watch`
   degraded-retry noise, a stale `identity_id` warning already present before
   this test started).

**Full session log** (raw PTY bytes) is in the transcript of this
investigation; the key excerpts (alt-screen entry `\x1b[?1049h`, status line,
insert mode, `"README.md" written`, alt-screen exit `\x1b[?1049l`) are all
present and well-formed.

---

## 3. Why this result is stronger than it might look

`PtyShell*`'s write path is not a separate, unverified code path from what a
human's keystrokes take. `handle_pty_shell_input`
(`agentmux-srv/src/server/mod.rs:1771`) calls:

```rust
blockcontroller::send_input(&req.shell_id, blockcontroller::BlockInputUnion::data(req.text.into_bytes()), None)
```

— the **exact same function** the WebSocket `blockinput` handler
(`agentmux-srv/src/server/websocket.rs:711`, the real human-keystroke path
per `frontend/app/view/term/termViewModel.ts:412`'s own comment) calls. The
only difference is transport (HTTP POST vs. WS frame) and a lock-ownership
check that doesn't apply to a standalone Terminal pane at all. So this test
exercised the actual PTY-write code a keystroke uses, not a look-alike.

Separately, `AgentShellSubblock.tsx` (the composer-drawer shell tested here)
and `frontend/app/view/term/term.tsx` (the standalone **Terminal** widget,
`defwidget@terminal`) both instantiate the same `TermViewModel`
(`frontend/app/view/term/termViewModel.ts`) — the xterm.js wrapper, its
config, and its alt-screen/resize handling are shared code, not
per-surface reimplementations. A hang specific to vim's alternate-screen
mode would very likely reproduce here too if it existed in that shared layer.

---

## 4. What this does NOT rule out (real gaps, not hedging)

- **Genuine browser keyboard events were not tested.** `PtyShellInput` writes
  bytes directly; it does not go through xterm.js's own DOM `keydown` →
  byte-sequence encoding in the frontend. If the bug is specifically in how
  a real keypress gets encoded (e.g. a specific key combo, once vim puts the
  terminal in application-cursor-key mode), this test would not catch it.
  No tool available in this environment can synthesize a real keyboard event
  scoped to an arbitrary pane (`UIClick` only clicks).
- **The standalone Terminal widget itself was not directly opened and
  driven.** The "More" widgets dropdown (`.action-widget-more-btn` →
  `.action-widget-more-dropdown`) would not populate via `UIClick` in this
  headless-ish environment (`DiscoverWindows` also returned no top-level
  windows here, so a full-window screenshot to debug the dropdown wasn't
  possible either) — see §3 for why the shared-`TermViewModel` architecture
  makes this a moderate-confidence inference rather than a directly observed
  result.
- **Exact commit of the live instance is unverified.** `main` was pulled
  fresh into the workspace clone for this investigation (now at `d41a598`,
  v0.56.1), but the already-running instance this test was probed against
  (`v0.56.0`, extracted at `~/.local/share/agentmux/extracted/0.56.0`) logs
  no git SHA at startup, so it's unconfirmed whether it already contains
  every very-recent terminal-path change (e.g. `fix(term): take the
  per-keystroke SQLite read off the input path`, #3249, merged today).
  Rebuilding and restarting that instance was avoided deliberately — it is
  the live srv serving this very session, and restarting it mid-session is
  the kind of disruptive action that should be confirmed with the user
  first, not done as a side effect of a bug investigation.
- **Not tested:** resizing the pane/window while vim is running (dragging,
  docking, or a debounced `usePtyWidth.ts` resize event landing mid-redraw),
  a `.vimrc` with plugins that emit unusual escape sequences, Windows/macOS
  (this VM is Linux only — the project's own docs note Windows gets the most
  testing and Linux/macOS lag behind), and any interaction with the very
  recent close-on-exit shell-pane feature
  (`SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md`).

---

## 5. Recommendation

Since this could not be reproduced with the tools available in this
environment, the most useful next step is narrowing the repro rather than
guessing at a fix:

1. **Confirm exactly which pane type**: the standalone Terminal widget
   (hamburger → More → Terminal) or an agent's own shell drawer (the "Shell"
   button under a composer)? They share rendering code (§3) but are opened
   through different UI paths.
2. **Platform** — Windows, macOS, or Linux? Given the project's own
   "Windows gets the most testing" note, a Linux/macOS-specific PTY/console
   difference is plausible and this VM is Linux-only.
3. **Does resizing the pane or window while vim is open trigger it**,
   or does it happen immediately on launch regardless of size changes?
4. **Any custom `.vimrc`/plugins**, or does a bare `vim -u NONE file` also
   hang?
5. If reproducible with an answer to the above, the next investigation
   should use an environment where a real browser `keydown` can be
   synthesized into the specific pane (Playwright/`_electron`, or a human
   in the loop) — the gap this report could not close with `PtyShell*`
   alone.

---

## 6. Update, same day: a real, related bug was found — by exactly the
gap this report flagged in §4

Another agent (Maricon) reported that **this agent's own composer-drawer
shell pane visibly corrupted (garbled vim screen) during the live session
above** — not a hang, a garbled render — and traced it to a genuine gap:
`blockinput` (the real-keystroke WS path, `agentmux-srv/src/server/websocket.rs:711`)
had **no server-side lease check at all**, unlike `controllerinput`. So a
human keystroke reaching the same pane while this investigation's
`PtyShellInput` writes were also landing mid-vim-redraw could interleave
into the same PTY and corrupt an in-flight alternate-screen escape
sequence. Fixed in **PR #3256** (`maricon/blockinput-agent-lock-check`,
open, not yet merged): adds the same `agent_lock::is_locked` check
`controllerinput` already had to `blockinput`, with a
red→green test (`blockinput_is_dropped_while_an_agent_lock_is_active`)
and the full `websocket`/`blockcontroller` suites passing (342 tests).

**This is exactly the §4 gap ("genuine browser keyboard events were not
tested... if a real human keystroke interleaves with this test's own
writes, this would not catch it") turning out to be load-bearing** — the
corruption required *concurrent* human + agent input into the same PTY,
which is precisely the scenario this report's `PtyShell*`-only methodology
structurally could not produce or observe by itself.

**Verified after being alerted, before taking any action on the peer's
report:** `PtyShellStatus` showed the shell (`d52fcc46-...`) still running;
both a fresh `UIScreenshot` and a full `PtyShellRead` of the raw buffer
showed a **clean, idle shell prompt — not corrupted** at the time of
checking. No swap files, no dirty `README.md`. Most likely explanation:
whatever garbling occurred during the concurrent-input window was already
undone by this investigation's own `:wq!` sequence, which exits vim
cleanly and emits real terminal-mode-reset sequences (`\x1b[?1049l` and
siblings) as part of a normal alternate-screen exit — so no recovery
action (redraw, force-quit, RIS reset, or `PtyShellStop`) was needed or
applied. Reported back to Maricon rather than running the suggested
recovery steps against a pane that, on inspection, was not actually in the
broken state described.

**Net effect on §1's conclusion:** unchanged for the *specific* symptom
asked about ("unresponsive") — corruption and unresponsiveness are
different failure modes, and PR #3256's own description is explicit that
this is "not a hang, byte interleaving mid-escape-sequence." But it is
strong evidence that the *general risk class* — concurrent writers into
one PTY — is real and was previously unguarded on the human-keystroke
side, and worth keeping in mind if "unresponsive" reports keep recurring:
a sufficiently mangled escape sequence from the same race could plausibly
produce something closer to a stuck-looking pane (e.g. cursor visibility
toggled off mid-sequence, `?25l` sent without its matching `?25h`) rather
than a cleanly "garbled but still updating" screen, depending on exactly
where the interleave lands. Worth re-asking anyone who reports
"unresponsive" specifically whether an agent might have been driving the
same pane via `PtyShellInput` at the same time.

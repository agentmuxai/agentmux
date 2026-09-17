# Report: vim locks the pane in 0.56.3 but not 0.56.2 — the code delta is not the cause

**Date:** 2026-09-16
**Status:** investigation complete; root cause NOT confirmed. The leading
hypothesis is environmental (concurrent-instance GPU contention), not a code
regression. Three cheap decisive tests are proposed in §7.
**Author:** Opaz
**Repo state:** `main` @ `cc8897c` (pulled fresh for this investigation)
**Method:** static analysis + read-only inspection of both running instances'
logs, settings and bundled runtimes. **Neither live instance was driven or
modified.**

---

## 1. The question

The repo owner runs two builds side by side on the same Linux VM:

- **0.56.2 — WORKS.** Open a Terminal pane, run `vim`: normal.
- **0.56.3 — BROKEN.** Open a Terminal pane, run `vim`: the pane loses
  keyboard responsiveness and locks.

Why?

## 2. Answer in one paragraph

**The 0.56.2 → 0.56.3 code delta does not contain a plausible cause, and two
independent deep audits failed to find one.** Every environmental variable I
could compare between the two builds is identical — same `libcef.so`, same
settings, same dependencies, same terminal renderer, same GPU init path. The
one thing that is *not* the same is that **0.56.3 is the second concurrently
running CEF instance on a VM with no real GPU**, and `vim` is by far the
heaviest renderer workload a terminal pane can produce. That is currently the
best-supported explanation, but it is a hypothesis, **not** a confirmed root
cause: I could not reproduce the freeze myself (see §6).

## 3. What the two builds actually are

Established from the running processes and the build artifacts on `~/Desktop`:

| | Working | Broken |
|---|---|---|
| Version | 0.56.2 | 0.56.3 |
| Commit | `11fe85f` (the `chore: release v0.56.2` commit, exactly) | `cc8897c` |
| Artifact | `AgentMux_0.56.2+g11fe85f5.20260916T174603.205596_amd64.AppImage` | `AgentMux_0.56.3+gcc8897c.dirty.20260917T032512.341118_amd64.AppImage` |
| Channel | `verify-decrqm-fix` (long-lived) | `local-main-b28b7a-bdd6bdf0` (fresh) |
| Uptime at time of report | ~6h40m | ~5 min |

The delta is exactly ten commits (`git log 11fe85f..cc8897c`): the eight that
make up v0.56.3, plus `c8c04c7` (docs) and `cc8897c` (the Wave→Mux rename).

**Note the broken build is `.dirty`.** That stamp is benign here: the only
modified tracked file is `package-lock.json`, and its entire diff is npm
renormalizing `"dev": true` into `"devOptional": true` on optional
platform-specific esbuild binaries. No package was added, removed or
version-changed. The artifact is effectively a clean `cc8897c`.

## 4. Ruled out, with the evidence

### 4.1 The four recent terminal fixes — ruled out by version membership
`#3253` (exit collapses the drawer), `#3256` (the `blockinput` agent-lock gap),
`#3257` (re-attach on sub-block respawn) and `#3261` (the flush barrier) were
the obvious suspects, since they are recent and all touch the terminal. They
are all **already in the working 0.56.2** — verified individually with
`git merge-base --is-ancestor <sha> 11fe85f`, all four YES. They cannot explain
a regression that appears only in 0.56.3.

### 4.2 The eight v0.56.3 commits — audited, clean
The range `11fe85f..bef81da` touches 32 files and **not one is a terminal,
keyboard, focus, PTY or app-shell file.** Grepping every added line in the range
for `keydown|keyup|addEventListener|.focus(|preventDefault|stopPropagation|activeElement|tabindex|capture`
returns exactly one handler: an `animationend` listener with `{ once: true }`
bound to a single `<li>` in the agent picker. Specifically ruled out:

- **`7815709` / `solid-transition-group`** — the DOM-reparenting risk is real in
  principle (reparenting a terminal's node drops xterm.js's hidden textarea
  focus), but `<TransitionGroup>` wraps only the `<For>` over agent-picker rows
  in `MyAgentsList.tsx`. No terminal DOM is inside it. The new
  `position: absolute; z-index: 4; pointer-events: none` in
  `_recent-sessions.scss` is scoped under `.agent-recent-sessions-row.agent-row-exit-active`.
- **`deebd72` (Open in New Window / Floating Pane)** — `window_create.rs` and
  `wcore::single_leaf_tree` only execute inside `handle_create_window`'s
  fresh-workspace branch when a `seed_view` arg is present. None of it runs when
  you open a Terminal pane in an existing window.
- **`65958f8` (muxsh Phase 2b)** — `muxsh.mjs` has zero added lines matching
  `stdin|tty|setRawMode|SIGWINCH`. It is an on-demand shell function; the rc
  files sourced at shell start are **byte-identical** across both builds.
- **`SPEC_PANE_OPEN_FOCUS_ROUTING_2026_09_16.md`** ("routing typing into a pane
  the moment it opens") looked highly relevant — but it landed in `78be065`,
  which is **already in the working 0.56.2**.

### 4.3 `cc8897c`, the 375-file Wave→Mux rename — audited, provably rename-only
This was the leading suspect: it is the **only** commit in the delta that
touches `view/term/`, `keyutil.ts`, `keymodel*.ts` or `websocket.rs` at all, and
a rename verified only by `cargo check` + `tsc` can still break runtime string
contracts. It was checked by reconstructing both trees and diffing all 256
changed files under a case-preserving rename-invariant normalization, so any
non-rename change survives as a textual diff.

**Six files differ beyond the rename, and all six are inert:** five are comment
text only, and one is `frontend/types/gotypes.d.ts` deleting dead `WaveAI*`
types (a `.d.ts` emits no JavaScript). The two genuine wire-value changes both
resolve as safe:

- `"get_wave_init_opts"` → `"get_mux_init_opts"` (`agentmux-cef/src/ipc.rs`) —
  renamed on both sides, and has **no caller at all**, in either tree.
  `docs/retro/audit-vestigial-types-2026-04-28.md` already documents it as dead.
- `arg_names: vec![... "waveObj" ...]` → `"muxObj"` (`backend/service.rs`) —
  `get_method_meta` is referenced only by its own unit tests; RPC args are
  positional.

Also checked and clean: no `.scss`/`.css` file is touched and no `className`
line changes (so no CSS/`data-*` desync); no Rust serde field renames (which
would silently change JSON wire names); no half-done rename (all 119 `wave*`
identifiers swept against the whole tree — the only survivor is `initHostWave`
in a comment).

### 4.4 Environment — identical on every axis I could measure
- **CEF/Chromium:** `libcef.so` is **287,218,840 bytes in both** extracted
  runtimes, and the source CEF build has not been rebuilt since 2026-09-15.
  Same Chromium 152.
- **Settings:** both channels' `config/settings.json` are all-defaults. In
  particular `term:disablewebgl` is unset (false) in **both**, and
  `window:disablehardwareacceleration` unset in both. The only difference is
  `"window:theme": "dracula"` in the 0.56.2 channel.
- **Dependencies:** identical except `solid-transition-group` (animation).
  **xterm.js is unchanged.**
- **Terminal renderer:** both logged `loaded webgl renderer!` — neither fell
  back to the DOM renderer.
- **GPU init:** both log the same software-rasterization pattern (`vaInitialize
  failed: unknown libva error`, `'--ozone-platform=wayland' is not supported`,
  `Cannot create bo with format=RGBA`, Vulkan `VulkanPreferredFeatures`
  warnings). No asymmetry.

### 4.5 Two "silently drop the keystroke" mechanisms — both checked, neither engaged
These deserved specific attention because their failure mode is *exactly* the
reported symptom:

- **`agent_lock`** (`blockcontroller/agent_lock.rs`). Since `#3256`, a held
  lease makes the server **drop human keystrokes** on `blockinput`. But the
  lease is a 4s window, evaluated (and pruned) at read time, per-block, and
  fails open across a restart — so it cannot wedge a pane for more than ~4s
  after the last agent write, and nothing leases a plain Terminal pane anyway.
- **`TermWrap.handleTermData`** (`termwrap.ts:513`) opens with
  `if (!this.loaded) return;` — every keystroke is discarded until the initial
  scrollback fetch completes. If that fetch hung, input would die exactly as
  described. It did not: both instances logged a clean
  `terminal loaded cachefile:0 main:0 bytes` (0.56.2: 79ms/112ms; 0.56.3:
  90ms/93ms), and there are no fetch errors in either log.

## 5. The only asymmetries found in the live logs

Real, but weak — and most likely consequences rather than causes:

| Signature | 0.56.2 | 0.56.3 |
|---|---|---|
| `[focus:focusNode] unable to focus node, cannot find it in tree` | 0 | 1 |
| `Cannot apply eventbus layout action DeleteNode, could not find leaf node` | 0 | 2 |
| `ShikiError: Language 'rust' is not included in this bundle` | 0 | 280 |
| `error setting RT info Error: unknown command: setrtinfo` | 9 | 2 |

- The **focus and layout-tree errors** name terminal blocks
  (`bba89910…`/node `9b88c933…`) — but the log context shows they fire
  immediately before `object.DeleteBlock`, i.e. while the pane is being
  *closed*. That is consistent with the owner closing a pane that had already
  locked, so it does not establish causation. Worth keeping: a frontend layout
  tree that disagrees with the backend's is the right *shape* of bug for
  "keystrokes have nowhere to go".
- **Shiki** is almost certainly a red herring: it fires when a Rust code block
  is rendered, and only the 0.56.3 session had an agent emitting Rust. It
  reflects different *content*, not a broken bundle.
- **`setrtinfo`** appears in **both** (more often in the working build) —
  pre-existing, not a regression.

## 6. What this report could NOT do, and why

**The freeze was never reproduced or directly observed.** Earlier in this same
session I damaged the repo owner's live workspace by driving a running instance
with `PtyShell*`/`UIClick` for exactly this kind of test. The standing
instruction now is that all testing happens on an isolated `task dev` instance,
never a live one — so this investigation was deliberately confined to static
analysis and read-only log/config/binary inspection. The gap that leaves is
real and is the reason §7 exists: I can rule things out, but the confirming
experiment has to be run against an instance it is safe to break.

## 7. Recommended next tests, cheapest and least disruptive first

1. **Turn off the WebGL renderer in the 0.56.3 channel and retest vim.**
   Uncomment `"term:disablewebgl": true` in
   `~/.agentmux/channels/local-main-b28b7a-bdd6bdf0/config/settings.json`
   (the file applies on save). If vim stops locking, this is GPU/WebGL
   contention under software rasterization, not a code regression — and the
   real fix is a renderer fallback heuristic for GPU-less environments, not a
   revert. **Do this one first: it is non-destructive and disturbs nothing.**
2. **Run 0.56.3 with 0.56.2 closed.** If vim behaves once it is the only
   instance, concurrent-instance contention is confirmed. ⚠️ This closes the
   instance the agent author of this report is running inside.
3. **Only if 1 and 2 both fail to explain it, bisect the code after all.**
   Build an AppImage at `bef81da` (v0.56.3 *without* the rename). vim working
   there would convict `cc8897c` despite §4.3; vim still locking would convict
   the v0.56.3 range despite §4.2 — and either outcome contradicts one of the
   two audits, which is itself the useful signal.

## 8. Standing caveat on this class of report

Two independent audits each cleared their own half of the delta and pointed at
the other half. That pattern usually means the premise shared by both — "the
cause is in the code delta" — is the thing that is wrong. §4.4 supports that
reading: nothing about the two builds differs except which one started second.
This report should not be cited as "0.56.3 has a terminal regression" until one
of §7's tests actually confirms a cause.

---

## 9. Update: reproduced nothing — vim works in BOTH an isolated dev instance and the isolated packaged AppImage

§6 said the confirming experiment needed an instance it was safe to break. That
experiment has now been run, and it changes the picture.

### 9.1 The missing instrument, found in-repo
CEF exposes a DevTools debug port (9223 dev / 9222 release) and
`tools/tests/bench-agent-keystroke.mjs` already drives it with
`Input.dispatchKeyEvent`. That is a **genuine browser key event** — the exact
capability whose absence made the earlier vim report (§4 of
`REPORT_VIM_TERMINAL_PANE_HANG_INVESTIGATION_2026_09_16.md`) inconclusive.
A probe built on it lives at `tools/tests/vim-freeze-probe.mjs`.

Ground truth is deliberately **the file vim writes**, not the screen: the WebGL
renderer paints to a canvas that cannot be read back as text, so asserting on
the rendered frame would be asserting on nothing.

### 9.2 The first verdict was FALSE, and the control is what caught it
The probe's first run printed "FREEZE REPRODUCED". **It was wrong.** A control
run — a plain `echo CTRL_x > file`, no vim, no alternate screen — *also*
failed, which proves a broken harness rather than a broken terminal. Two real
defects in the probe, both worth recording because both produce a confident
false positive:

1. **`el.focus()` is not focus.** It sets DOM focus, but does not drive the
   app's own focused-pane routing, so keystrokes arrived at
   `.xterm-helper-textarea` and were then discarded. Fixed by clicking the pane
   with a real `Input.dispatchMouseEvent`, the way a human does.
2. **xterm.js does not take printable characters from `keydown`.** It encodes
   the special keys itself (Enter, Escape, arrows, chords) and takes ordinary
   text from the textarea's `input` event. So `keydown`+`char` per letter
   delivered real events that xterm correctly ignored. Confirmed directly
   against the live PTY scrollback, which showed a bare prompt followed by
   `exit=failure;status=1` — Enter arriving with an empty command line. Fixed
   by sending text via `Input.insertText` and keeping real keydown for the
   special keys.

A `--control` mode is now part of the probe, and **its passing is what licenses
believing any negative result.** Without it this report would have claimed a
reproduction that never happened.

### 9.3 Results, with the control passing in both environments

| Environment | Build | Control | vim |
|---|---|---|---|
| `task dev`, isolated dev data dir | 0.56.3 source @ `cc8897c` | PASS | **PASS — no freeze** |
| Packaged AppImage, isolated `opaz-vimtest` channel | `AgentMux_0.56.3+gcc8897c…AppImage` — the exact artifact reported as broken | PASS | **PASS — no freeze** |

In both, real synthetic keystrokes launched `vim`, entered insert mode, typed a
sentinel, pressed Escape, ran `:wq`, and the file on disk contained the
sentinel. Every keystroke reached the PTY.

### 9.4 What that means
**The 0.56.3 code and the 0.56.3 artifact both behave correctly in isolation.**
Combined with §4 (no plausible cause in the code delta) and §4.4 (identical
CEF, settings, dependencies and renderer), the cause is not *what was built* —
it is something about the environment the failing instance runs in. The
surviving difference is unchanged from §2: the failing instance is the second
concurrent CEF instance on a VM with software rasterization, with agents
actively producing output, and vim is the heaviest repaint load a pane can
generate.

This is consistent with the repo's own current position: `2c1704b` (merged
after this build was cut) adds `tools/pane-load.mjs` and states plainly that
**"the cross-pane typing lag is still undiagnosed"**, noting that its `paint`
mode — bulk escape sequences, real parser and renderer work per byte — is the
load shape that provokes it. That is precisely vim's profile. The symptom here
may well be that open issue rather than a 0.56.3 regression.

### 9.5 Honest remaining gap
The cross-pane load test was **not** completed. `/api/v1/pane/open` placed its
panes in a tab the window did not display (worth a look on its own), so a
second *visible* pane to flood while typing in the first was never established,
and `/api/v1/ptyshell/create` requires an `agent_block_id`. So "vim under
concurrent heavy output from another pane" remains untested — and it is the
single most likely remaining candidate. That is the next experiment, not a
conclusion.

---

## 10. The dev-vs-portable question, and a separate real finding

### 10.1 The dev/portable split alone does NOT explain it
The reported pattern is "vim is fine under `task dev`, but fails on a
portable". §9.3 tested **both**, each with a passing control, and vim passed in
both — including the packaged AppImage, the exact artifact reported as broken.
So "packaged" is not by itself sufficient to produce the failure. Something
about the *environment* the failing instance runs in is still required.

### 10.2 What genuinely differs between `task dev` and a portable
Grounded in the build scripts, not folklore:

| | `task dev` | portable / AppImage |
|---|---|---|
| Frontend | Vite dev server, **unminified**, HMR, separate origin (`localhost:<vite>`) | bundled, **minified** assets from the srv's own endpoint |
| Source maps | present | tarball build strips them (196 files, per its own log) |
| `libcef.so` | the CEF out dir directly — **unstripped, 1571 MB** | **stripped to ~287 MB** at bundle time |
| Env | `AGENTMUX_DEV=1` (writes `authkey.dev`, dev badge) | not set by default |
| CDP port | 9223 | 9222 |
| Layout | `dist/cef-dev`, flat | extracted AppDir + `LD_LIBRARY_PATH` |

The two with real behaviour-changing potential are **minification** and the
**stripped CEF**. Neither was falsified here — they were simply not sufficient
on their own, since the packaged build passed.

### 10.3 NEW: shell integration is a machine-global singleton, shared across
### every instance AND every version

`bootstrap.rs:1045` deploys shell integration to
`get_home_dir().join(".agentmux")` — i.e. **`~/.agentmux/shell/`**, *not* the
instance's own data dir or channel. Every instance on the machine, of every
version, portable or dev, writes and reads the same files.

Redeployment is gated on `~/.agentmux/shell/.version`, whose marker is a
version string plus a hash of the script contents (`version_marker()`).
Observed live: the marker is `0.56.3-94ef97c98e76de45` and every script's
mtime is **20:37** — the moment the 0.56.3 instance started.

The consequence, observed on this machine right now: **the 0.56.2 instance is
still running, and its shells now source 0.56.3's shell-integration scripts.**
Because the marker embeds the version, two concurrently-running versions will
each redeploy on startup, overwriting the other's scripts — the last one to
start wins, for everyone.

This is a genuine isolation defect and worth fixing on its own merits,
independent of the vim question: per-build channel isolation (I1–I6) is
carefully observed for data dirs, and then silently abandoned for the shell
integration every terminal in every instance sources at startup.

Whether it can produce the vim symptom is NOT established. The deployed
`bash/.bashrc` only defines on-demand functions (`muxlog`, `muxspect`,
`muxopen`, `muxsh`) and appends `_agentmux_si_prompt_command` — which emits
OSC 7 and agent env **at prompt time**, i.e. after vim exits rather than during
its alternate-screen session. So the obvious mid-vim corruption path is not
there. It is recorded here as a real defect found while investigating, and as
the best current explanation for why the failure correlates with *running two
versions at once* rather than with dev-vs-portable.

### 10.4 The honest state
No, this is not understood yet. The repo's own newest commit says as much.
What has been added here is: the code delta is cleared (§4), the artifact is
cleared in isolation (§9), and the remaining variables are narrowed to
**concurrent load** and **cross-instance coupling** — of which §10.3 is now one
concrete, verified instance.

---

## 11. PROVEN: shell integration breaks instance isolation (I6), demonstrated live

§10.3 recorded the shared-path defect from code reading. It has now been
**demonstrated empirically**, using only throwaway instances in their own
channels.

### 11.1 The experiment
`~/.agentmux/shell/.version` holds the deployment marker (`<version>-<hash>`).
Watching it while starting an instance in a **completely separate channel**:

| Time | `.version` | What happened |
|---|---|---|
| 20:37:20 | `0.56.3-94ef97c98e76de45` | the 0.56.3 instance booted |
| 23:49:19 | `0.56.2-a8a36d037c528879` | a 0.56.2 instance booted in channel `opaz-iso562` — **a different channel entirely** |
| 23:49:46 | `0.56.3-94ef97c98e76de45` | **one terminal opened** in a 0.56.3 instance flipped it straight back |

27 seconds between the two flips. Nothing here is a race condition or a
corner case: it is the designed behaviour of a version-keyed marker over an
unkeyed shared path.

### 11.2 Why it is worse than "written once at startup"
`deploy_scripts` is called from **two** places:
- `bootstrap.rs:1045`, once per srv start, and
- `blockcontroller/shell/lifecycle.rs:555`, **on every interactive shell spawn**.

So the rewrite is not a boot-time event that settles; it happens every time
anyone opens a terminal in any instance of any version. Two concurrently-running
versions therefore rewrite these files past each other indefinitely. And because
`std::fs::write` truncates before writing, there is a real (if narrow) window in
which a shell starting in instance A sources a `.bashrc` that instance B is
mid-way through replacing.

The shared surface is not only the rcfiles: `muxlog.mjs`, `muxspect.mjs`,
`muxopen.mjs` and `muxsh.mjs` all live in the same global directory. Those talk
to a backend over its API, so a 0.56.2 instance can end up running 0.56.3's
`muxlog.mjs` against a 0.56.2 srv — a straightforward protocol-skew vector.

### 11.3 Why it is like this (and what a fix must preserve)
Not an oversight. `lifecycle.rs:549-553` states the reason:

> Deploy shell integration scripts to `~/.agentmux/` … instead of
> AGENTMUX_DATA_HOME. MSIX packages virtualise writes to `%LocalAppData%`, so
> files written by the packaged backend aren't visible to child processes
> (pwsh, bash, etc.) spawned via ConPTY. The home dir is never virtualised, so
> the scripts are always reachable at their literal path.

That constraint is real and any fix must keep it. So the fix is **not** "move it
back under the data dir" — that would reintroduce the MSIX bug. The fix is to
keep the non-virtualised home base and *key the subdirectory*, exactly as I5
already requires of every named OS object:

    ~/.agentmux/shell/<channel>-<version>/{bash,zsh,pwsh,fish}/…

Both call sites already take the base as a parameter (`deploy_scripts(&base)`
and `get_shell_startup(shell_type, &base)`), so they stay consistent by
construction as long as the same keyed base is passed to both. A stale-directory
sweep would keep it from growing without bound.

### 11.4 Status
Proven defect, violating **I6** ("instances of different `(channel, version)`
never share a data/logs/cef-cache directory") in substance if not in the literal
list of directories named there. **Not** shown to cause the vim symptom — the
deployed `.bashrc` only defines on-demand functions and a prompt-time OSC 7 hook
— and it should be fixed on its own merits rather than as a vim fix.

# Report: Blockers building and running the UI-manual screenshot tooling

**Date:** 2026-09-19
**Status:** retro — a live-as-it-happened record. Attempts 1/3's root
cause is fixed by `docs/specs/SPEC_DEV_INSTANCE_ISOLATION_DIAGNOSTICS_2026_09_19.md`;
the rest (Attempts 2, 4-8) are recorded findings, not yet acted on.
**Context:** Building `scripts/ui-screenshots/` (see
`docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md`) needed a clean,
isolated AgentMux instance — no real conversations/accounts — to capture
manual screenshots against. Getting a second instance running turned into
most of the actual time spent on this task (§Attempts 1-5). Once capturing
against the live primary instance instead (the fallback — see §Outcome),
navigating its UI to reach each shot surfaced a second, distinct class of
blocker (§Attempts 6-8) — general enough to matter well beyond this one
tool, and directly relevant to the separate mouse/screen-control spec this
same session produced. Recorded live, as it happened, so the underlying
gaps are fixable rather than re-discovered next time.

**Platform scope — read before assuming any of this reproduces elsewhere:**
this entire session ran on Linux (Wayland, GNOME) only — nothing below was
tested on macOS or Windows. Attempts 1-4 and 6-8 are rooted in
platform-agnostic code (Rust logic with no OS-specific branch for the
behavior in question, or pure JS/CDP logic) and so are *reasoned to likely
reproduce* cross-platform — but that's an inference from reading the code,
not empirical confirmation. Attempt 5 (GPU device contention) is the one
exception: the *investigation method* was Linux-DRI-specific
(`/dev/dri/renderD128`, `fuser`) even though the underlying problem class
(too many concurrent GPU-accelerated processes on one machine) plausibly
has a Windows/macOS equivalent through entirely different mechanisms
(DXGI/Metal contention) that this session never looked at.

---

## Attempt 1: `AGENTMUX_CHANNEL` env override on the already-built binary

Reasoning: `dist/cef-dev/` already had a fully-built binary from an earlier
`task dev` run this session. Rather than rebuild, launch it again with
`AGENTMUX_CHANNEL=dev-screenshot-demo` set manually.

```
LD_LIBRARY_PATH=. AGENTMUX_DEV=1 AGENTMUX_CHANNEL=dev-screenshot-demo ./agentmux-launcher --url=http://localhost:5309
```

**Result:** refused — `AgentMux dev instance already running (channel:
dev-main-c8fd090d2b4adf1d). Use task dev:local to launch a second isolated
session.` The reported "channel" (`dev-main-c8fd090d2b4adf1d`) shows the env
override was ignored for the actual isolation key — it's still `dev-main`,
plus a hash that turned out (see Attempt 3) to be derived from the clone
path + branch, not from `AGENTMUX_CHANNEL` at all. **The env var isn't
wired into whatever `second_instance.rs` actually keys on for a `task
dev`-style launch** — only `task dev:local`'s version-bump mechanism
changes that key. This is confusing because the same env var name **does**
work as the compile-time isolation key for `task package` builds (per
`CLAUDE.md`'s per-build-channel docs) — the two build modes look like they
should behave the same way here and don't.

**Fixable:** either make the runtime `AGENTMUX_CHANNEL` override actually
take effect for a `task dev`-style launch too (consistent with the
compile-time `task package` behavior), or have the refusal message name the
real key (clone path + branch) instead of implying `AGENTMUX_CHANNEL` alone
is the lever.

**Platform scope:** `second_instance.rs`'s key-resolution logic isn't
gated on `cfg(windows)`/`cfg(unix)` anywhere near this path — reasoned to
likely reproduce identically on macOS/Windows, not verified there.

## Attempt 2: `task dev:local` (the tool's own suggested fix)

**Result:** `scripts/dev-local.sh: line 79: bump: command not found` — the
script calls a bare `bump` (the `@a5af/bump-cli` package), but it isn't on
`PATH` in this shell (`npm run`/`npx` scripts get `node_modules/.bin` on
`PATH` automatically; a plain login shell does not). `npx bump --version`
resolves to a **different, unrelated npm package also named `bump`**
(v0.2.1, not `@a5af/bump-cli`), which is actively dangerous — if
`dev-local.sh`'s own `bump "$BUMP_TYPE"` call had silently resolved to that
wrong package via some future `npx` fallback, it could have run an
unexpected tool in a version-bump-shaped hole with no warning.

**Fixable:** `dev-local.sh` should invoke `node_modules/.bin/bump` directly
(or `npx --no-install @a5af/bump-cli`, pinning the exact package) instead of
a bare `bump`, so it fails loudly on a missing dependency instead of
silently resolving to a same-named-but-wrong package if one happens to be
npx-cacheable.

**Platform scope:** the script is bash, so this exact failure is
Unix-shell-specific (`scripts/dev-local.sh` — check whether a Windows
equivalent exists and has the analogous bare-`bump` issue; not checked this
session).

## Attempt 3: Manual version bump (package.json + Cargo.toml), then `task dev`

Worked around Attempt 2's tooling gap by hand-editing both version fields to
`0.56.7-screenshot-demo`, then running `task dev` directly. This **did**
trigger a real rebuild (~7 minutes for `agentmux-srv` alone) and did
register under a distinct launcher-level socket
(`ee625e0d53bd24b2.sock`, vs. the primary instance's
`9d8597e7af935398.sock`) — so the launcher-level isolation genuinely is
version-keyed, contrary to what Attempt 1 suggested.

**But the underlying CEF process still refused to start** — `CEF early
exit (process singleton or similar) — exiting cleanly exit_code=24` /
`Opening in existing browser session`. The log's own `data_dir` line gave
it away: `/home/yas/.agentmux/dev/main/c8fd090d2b4adf1d/cef-cache` — **the
exact same path the primary instance uses.** The *launcher's* IPC socket is
version-keyed; the *data directory* (and therefore Chromium's own
`SingletonLock` inside `cef-cache`) is not — it's keyed on clone path +
git branch only (`~/.agentmux/dev/<branch>/<hash>/`, matching `CLAUDE.md`'s
own "dev data dir is keyed on the git branch" note, which I'd read earlier
in the session and then second-guessed based on the Attempt-1 error
message's phrasing).

**This is the actual root confusion worth fixing:** two different
isolation keys (launcher socket = version-based; CEF profile dir =
branch-based) are both live in the SAME `task dev` codepath, and a launch
can partially succeed (own socket) while still failing (shared Chromium
profile) with an error message (`CEF early exit ... process singleton`)
that doesn't say *why* — no data-dir path, no pointer to the conflicting
process, no mention that a different branch would fix it.

**Fixable:**
- Either key both surfaces (launcher socket AND CEF profile dir) off the
  same value, or make the mismatch legible: the `CEF early exit (process
  singleton...)` log line should print the data_dir it collided on and
  which PID/channel currently holds its `SingletonLock`, mirroring the
  quality of the launcher-socket refusal message (which at least names the
  channel and socket path).
- The refusal message a user/agent actually needs — "two dev instances
  from the same branch cannot run side by side; check out a different
  branch or worktree" — never appears anywhere in this whole chain.

**Platform scope:** the launcher-socket-vs-CEF-profile-dir split is core
Rust logic (`agentmux-common`/`agentmux-launcher`), not behind an OS
`cfg` gate for this specific behavior — reasoned to likely reproduce on
macOS/Windows, not verified there. Chromium's own `SingletonLock`
mechanism is itself cross-platform (same concept on all three OSes), which
supports the inference.

## Attempt 4: git worktree on a new branch (worked, isolation-wise)

`git worktree add ../agentmux-screenshot-demo -b screenshot-demo main` — a
genuinely different branch name, so the branch-keyed data dir differs.
Confirmed this is the right lever per Attempts 1-3's findings.

**Cache-sharing did not work as hoped.** Symlinked both `target/` and
`node_modules/` from the worktree to the main checkout's directories,
expecting cargo to recognize identical source (same commit) and skip
recompilation entirely. It did not skip — `agentmux-srv` compiled again
from scratch. Not yet root-caused in this session (ran out of time
budget for the actual screenshot task), but the likely culprit is that
cargo's build fingerprinting includes the **absolute source path**
(`agentmux-screenshot-demo/...` vs `agentmux/...`), which differs between
the two worktrees even though the file *contents* are identical — cargo
treats a different `CARGO_MANIFEST_DIR` as a genuinely different build
unless `[profile].strip-paths`-style path normalization or a shared
`CARGO_TARGET_DIR` set *consistently across both invocations from the
start* is in place. Symlinking `target/` after the main checkout had
already built into its own (path-fingerprinted) target dir was too late to
help.

**Fixable, for next time:** if isolated demo/test instances from a fresh
worktree become a recurring need (not just this one-off), set
`CARGO_TARGET_DIR` to a shared, absolute, worktree-independent path (e.g.
`~/.cargo-target-shared/agentmux`) **from the very first build**, in both
the primary checkout and any worktree — never let either build into its
own path-fingerprinted `target/` first. This is a `.cargo/config.toml` or
env-var change to make once, not a per-worktree workaround.

**Platform scope:** Cargo's build-fingerprinting behavior (including
`CARGO_MANIFEST_DIR`/absolute-path sensitivity) is part of Cargo itself,
not the OS — reasoned to reproduce identically on macOS/Windows. `git
worktree` is equally cross-platform.

## Attempt 5: worktree + shared-cache build actually launched — then hung on GPU contention

After Attempt 4's worktree+symlink build (which did NOT skip recompilation
as hoped — see above), the resulting binary launched with genuinely
distinct isolation (`data_dir=/home/yas/.agentmux/dev/screenshot-demo/...`,
a real, different hash from the primary instance's `dev/main/...`). No
socket collision, no CEF `SingletonLock` collision — the worktree branch
approach is confirmed correct for isolation.

**But the process never finished creating its window.** Log progress
stopped cold after `window:transparent=false` / `resolved GPU tier for
ANGLE selection tier="hw-gl"` — no window-creation confirmation, no CDP
remote-debugging port ever actually bound (`ss -tlnp` showed only the IPC
port, 44727; the logged "CEF remote-debugging port: 35085" was apparently
just the *intended* port, printed before the bind was attempted, not
confirmation of an actual bind). Process sat idle (`Sl` state, ~0% CPU) for
over a minute with no further log output — not slow, genuinely stalled.

Root cause (fairly confident, not exhaustively proven): GPU device
contention. `fuser /dev/dri/renderD128` showed **16 processes** already
holding the render node — the primary dev instance's main window plus its
window-pool pre-warm processes, a real Chrome browser also running on this
box, and now this new instance competing for the same one. Both instances
log the identical non-fatal GPU warnings (`vaInitialize failed`,
`'--ozone-platform=wayland' is not compatible with Vulkan`,
`Cannot create bo with format=RGBA_8888`) — the primary instance survives
these and renders fine; this one didn't get far enough to even try.

**Fixable / worth investigating separately from this task:**
- If a **third+ concurrent CEF instance is a real, supported use case**
  (multiple agents each running their own `task dev` on a shared dev box —
  plausible given this exact box had at least 3 other agents' processes
  visible during this session), GPU-resource exhaustion silently hanging
  window creation with no error, no timeout, and no diagnostic log line is
  a real gap. A "GPU acquisition timed out after Ns, falling back to
  software rendering" log line (or an outright `--disable-gpu` fallback
  flag `task dev` could pass when resource pressure is detected) would
  turn a silent multi-minute hang into a fast, recoverable path.
- Not investigated: whether `agentmux-cef --disable-gpu` (or an equivalent
  flag) would have unstuck this specific launch — ran out of task time
  budget to test it. Worth trying first, next time this recurs.

---

The isolated instance never came up (Attempts 1-5). Falling back to
capturing against the already-running **primary** instance instead
(restricted, per the original scoping discussion, to chrome/widget shots
that shouldn't show real conversation content) surfaced a second, distinct
blocker class — not about instance isolation at all, about **driving the
UI reliably once you have a target**. Recorded here because it's the more
generally useful finding of the two, and because it directly informed the
separate `docs/specs/SPEC_AGENT_MOUSE_SCREEN_CONTROL_2026_09_19.md` written
the same session.

## Attempt 6: fuzzy text-matching click landed on the wrong element, with a real side effect

`capture.mjs`'s `clickText()` helper finds the first leaf element whose
text matches (exact, falling back to substring) inside a given container,
then clicks its nearest clickable ancestor. Reaching for "Terminal" (not a
pinned widget — it lives in the top bar's "More" dropdown, not the pinned
`.action-widgets` row) went through several iterations: first tried
`.action-widgets` directly (correctly failed — nothing there matches
"Terminal"), then tried opening "More" first — and that attempt, whatever
element it actually landed on, did **not** open a terminal. It **opened
Parlo's own live agent pane instead**, briefly surfacing Parlo's real,
in-progress conversation transcript (including a "Claude Code login has
expired" prompt) on screen in the shared primary window.

No screenshot of that state was saved — the run errored (on a *later* step,
verifying `.view-term` never appeared) before `capture.mjs` ever got to
`Page.captureScreenshot` for that shot. But the exposure itself already
happened, live, in a window other people can see — the save failing is not
what prevented it.

**Root cause:** `clickText`'s substring fallback has no confirmation step.
It computes a click target from a text match and fires the click
immediately — there is no "did this actually navigate to what I expected"
check before treating a click as done and moving on. A generic tool
(`capture.mjs`) reaching into a specific app's UI by guessed text labels,
with no structural/semantic anchor (no `data-testid`, no ARIA role
requirement, no post-click assertion) and no dry-run/preview step, is
exactly the failure shape: it doesn't fail loud when it clicks the wrong
thing, it just silently does something else instead.

**Platform scope:** none — pure JS/CDP logic in code this session wrote,
identical behavior on any OS `capture.mjs` runs on.

## Attempt 7: no way to tell "focus an existing pane" from "create a new one"

Clicking a pinned widget's top-bar button when that widget's pane is
*already open* correctly re-focuses the existing pane (confirmed: Swarm,
Sysinfo, and Armory shots all landed on the pre-existing panes, no
duplicates). But there is no way to know *in advance*, from the click
target alone, which behavior a given click will trigger — "focus existing"
and "create new" are the same gesture (click the widget's icon), and the
outcome depends entirely on invisible-to-the-caller UI state (is a pane
for this already open in the current tab). Reaching for `Settings` and
(eventually) something terminal-shaped, through a few retries, left the
primary window with **several new, uncloseable-by-me-safely panes** — the
tool has no concept of "clean up what I opened," and the operator (this
agent) had no reliable way to identify which of several near-identical
"Agent" picker panes were pre-existing versus freshly created by its own
clicks, so cleanup was correctly abandoned rather than guessed at (see
§Outcome).

**Fixable:** a generic input-driving tool needs either (a) a way to query
"would this action create something new or just focus something existing"
before acting — not available today for AgentMux's own UI even via the
DOM (no `data-amx-*` marker distinguishes "existing pane, focusing" from
"about to construct a new one") — or (b) its own action log recording
enough identity about what it opened (a resolved `block_id`, not just "I
clicked the Settings menu item") to reliably undo/close exactly what it
created and nothing else.

**Platform scope:** none — an AgentMux frontend/UI-design gap, not
OS-specific.

## Attempt 8: a "safe-looking" view turned out to contain real workspace data

Two shots (`agent-picker`, `armory-bundles`) were selected specifically
*because* they looked content-free by construction — a picker screen with
no live conversation, a bundle list. Both actually render real,
workspace-specific data by design: the agent picker's "MY AGENTS" section
lists every agent that's ever launched in this workspace by name (Parlo,
Opaz, Maricon), and the Armory Bundles tab lists per-agent bundle names the
same way. Neither is conversation content, but both are real,
workspace-identifying data — exactly the category the original "clean
staged instance" plan (§Outcome) existed to avoid, and the live-instance
fallback reintroduced by construction, not by a technical slip.

**This wasn't a bug to fix — it was a wrong assumption caught late.**
Worth recording anyway: "does this view show conversation content" is not
the same question as "does this view show workspace-identifying data," and
a capture tool's shot-selection judgment (mine, in this case) needs to ask
the second question explicitly, not infer it from the first.

**Platform scope:** not applicable — a content-design mistake, not a
technical/platform issue.

## Outcome for this task

After 5 attempts and roughly 30-40 minutes, gave up on standing up a
genuinely isolated instance **for this specific run**, on this specific
(already GPU-saturated, multi-agent-shared) machine, given no fixed budget
justified pushing further. Fell back to capturing the first 10 shots
against the **already-running primary dev instance**, restricted to
chrome/widgets that don't expose real conversation content (the alternate
option from the original scoping discussion) — not the originally-chosen
"clean staged instance" path. `scripts/ui-screenshots/` itself is unaffected
by this — it just points at whatever CDP port it's given; running it
against a properly isolated instance later (once GPU contention isn't a
factor, or `--disable-gpu` is confirmed to help) needs no tool changes.

## Net time cost

Roughly 30-40 minutes of an interactive session spent purely on instance
isolation, across 5 attempts, before any actual screenshot capture began —
for a repo that already has first-class per-channel isolation machinery
(`SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md`, the I1-I7 invariants in
`CLAUDE.md`) for the **packaged/release** build path. The `task dev` path's
isolation story is real but has a rougher edge than the packaged path's,
its own error messages don't yet point at the actual fix the way the
packaged path's do, and — orthogonally — this specific shared dev machine
didn't have GPU headroom for a third concurrent CEF instance regardless of
how cleanly isolated it was.

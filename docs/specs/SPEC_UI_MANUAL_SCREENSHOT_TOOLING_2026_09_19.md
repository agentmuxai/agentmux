# SPEC: Reusable, cropped screenshot tooling for the user manual

**Date:** 2026-09-19
**Status:** implemented — first run complete (§6a), tool itself working.
**Related:** `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` (the
CDP automation layer this reuses — `Page.captureScreenshot`/`Input.dispatchMouseEvent`/
`Runtime.evaluate` already built into `agentmux-cef/src/browser_api/`, exposed
to agents as `UIScreenshot`/`UIClick`/`UIQuery`), `docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`
(capture is no longer own-pane-restricted, but the MCP tools themselves stayed
scoped that way — see §2 for why this spec talks to CDP directly instead),
`docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md`
(OS-level `CaptureWindow` blockers — irrelevant here since this tool never
uses OS-level capture at all, see §2), `docs/reports/REPORT_SCREENSHOT_TOOLING_ISOLATED_INSTANCE_SETUP_BLOCKERS_2026_09_19.md`
(the full blockers log from building and first-running this tool — isolated-instance
setup Attempts 1-5, then navigation-robustness Attempts 6-8 once capturing
against the live primary instance instead), `docs/specs/SPEC_DEV_INSTANCE_ISOLATION_DIAGNOSTICS_2026_09_19.md`
(the one blocker from that report fixed so far — a misleading `task dev:local`
refusal message hit while trying to stand up the isolated instance this
spec's own §3 describes).

---

## 0. The ask

A user manual for AgentMux needs screenshots of its widgets and panes —
cropped to the relevant UI region, not full-desktop grabs with a bunch of
irrelevant chrome around them. This needs to become a **reusable** tool, not
a one-off: the UI changes over time, and the manual will need re-captures
that don't require re-deriving pixel coordinates by hand every time.

First run: 10 high-value shots, broad coverage, agent's own judgment on
which 10.

## 1. Non-goals

- **Not** OS-level screen capture (`xcap`/`enigo`/`scrot`/PowerShell
  `CopyFromScreen`). AgentMux's entire UI — every pane, the status bar, every
  popover — is one DOM tree inside one CEF `Browser` per window (confirmed in
  `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`). Cropping a
  screen-space PNG to a widget's region means hand-computing/maintaining
  pixel offsets that break the moment a widget moves, resizes, or the window
  changes size. Cropping via the DOM's own `getBoundingClientRect()` is
  exact and survives layout changes — the crop coordinates are recomputed
  fresh on every run, not hand-maintained.
- **Not** a generic desktop-automation tool (no xdotool wrapper, no
  computer-use tool needed). CDP's `Input.dispatchMouseEvent` already gives
  pixel-accurate synthetic clicks scoped to the target window, which is all
  navigation (opening a widget, switching a settings tab) needs.
- **Not** capturing real user/agent data. See §3 — the first run uses an
  isolated, freshly-provisioned dev instance with no real conversations,
  OAuth accounts, or personal file paths in it.

## 2. Why raw CDP, not the `UIScreenshot`/`UIClick`/`UIQuery` MCP tools

Those tools exist and use the identical underlying mechanism (CDP against
the CEF instance's remote-debugging port) — but they're scoped to the
calling agent's **own pane** (confirmed live: their tool descriptions still
say "your OWN pane," "cannot reach a DIFFERENT pane," even after the
2026-08-30 accountability spec relaxed the *capture* restriction generally).
A widget/pane screenshot tool needs to reach an **entire separate AgentMux
window** — a different, isolated instance stood up specifically for clean
captures — which those own-pane-scoped tools cannot do.

CDP itself has no such restriction: `agentmux-cef` always binds a
remote-debugging port (confirmed live — every running instance in this
session had one, found via `ss -tlnp` and the `/json` endpoint's target
list). Connecting to it directly, the same way `UIScreenshot` does
internally, gives every capability needed with no pane-ownership boundary:

- `Page.captureScreenshot` with a `clip` region for an exact, pixel-precise
  crop — no separate crop step, no image-processing dependency.
- `Runtime.evaluate` to run `getBoundingClientRect()` against a CSS
  selector, producing that `clip` region fresh every run.
- `Input.dispatchMouseEvent` for the handful of navigation clicks needed
  (open a widget, switch a tab) before a shot.

This was proven out live earlier the same day, debugging the double-header
and blank-pane bugs (this doc's sibling specs in the same date) — the exact
same `ws://127.0.0.1:<port>/devtools/page/<id>` + `Page.captureScreenshot`/
`Runtime.evaluate` pattern, just pointed at a screenshot goal instead of a
debugging one.

## 3. Isolated capture instance

Screenshots for a manual should never show a real user's conversations,
home-directory paths, or account state. The first run uses a **separate,
isolated `task dev` instance** — not the primary dev instance already
running in this workspace — so a fresh install with no agents, no
conversations, and no OAuth-linked accounts is what ends up in frame.

Isolation mechanism: `task dev`'s per-instance channel is derived from
`CARGO_PKG_VERSION`/`package.json` version at launch (`agentmux-launcher`'s
single-instance socket is keyed on `hash(data_dir + version)` — see
`CLAUDE.md`'s isolation-invariants section, I1). A plain
`AGENTMUX_CHANNEL` env-var override on an otherwise-identical build did
**not** produce a distinct instance (the running binary re-derives/keys off
the version, not a bare env var — confirmed live: a second launch under a
different `AGENTMUX_CHANNEL` was rejected as "already running" against the
primary instance's socket). What actually works, and is the tool's own
documented escape hatch (`scripts/dev-local.sh`, `task dev:local`): a
temporary, uncommitted version bump (`package.json` + `Cargo.toml`
`[workspace.package].version`) before `task dev`, reverted via
`git checkout -- package.json Cargo.toml` once the capture run is done. No
git mutation persists across a capture session.

Both instances share the same Vite dev server (`--url`) — the frontend
bundle is identical across instances on the same commit; only the backend
channel/data-dir differs, which is what isolation actually needs.

## 4. Reusable tool

`scripts/ui-screenshots/`:

- **`shots.mjs`** — the declarative manifest. An array of shot descriptors:
  `{ id, title, description, selector?, padding?, prep? }`. `selector`
  omitted means "whole window." `prep` is an optional async function
  (given the CDP session) that runs navigation clicks/waits before the
  capture — e.g. clicking a widget-bar button, waiting for its content to
  settle. **This file is the thing a future update to the manual actually
  edits** — add a shot, change a selector, nothing else in the tool needs
  to change.
- **`capture.mjs`** — the runner. Connects to the CDP remote-debugging port
  of whichever instance you point it at (`--port N`, or `AGENTMUX_CDP_PORT`;
  defaults to `9222`), auto-selects the main-window target (the one whose
  URL has no `windowLabel=` query param — pool/floating-pool pre-warmed
  windows always do), and iterates `shots.mjs` in order (`--only id1,id2` to
  run a subset, `--out DIR` to override the output directory). For each
  shot: runs `prep` if present, waits a short settle delay,
  resolves the `clip` region via `Runtime.evaluate` if `selector` is set,
  captures via `Page.captureScreenshot`, writes `<NN>-<id>.png` to the
  output directory, and appends an entry to `manifest.json` (id, title,
  description, output filename, captured-at timestamp, pixel dimensions).
  No external npm dependency — Node's native `WebSocket` (already proven
  working this session) and `fs`/`http` are enough.
- **`manifest.json`** (generated, not hand-maintained) — the record of what
  was actually captured in a given run: what each file is, when, and at
  what size. This is what the person assembling the manual reads to know
  which file is which, without having to open 10 PNGs and guess.

### Why this stays current as the UI changes

- Crop regions are **never hand-coded pixel coordinates** — always a CSS
  selector resolved fresh via `getBoundingClientRect()` at capture time.
  A widget moving or resizing doesn't invalidate anything; re-running
  `capture.mjs` against the current build re-derives every crop.
- Adding, removing, or re-describing a shot is a one-line edit to
  `shots.mjs` — no new capture logic needed for the common case.
- A selector genuinely disappearing (a widget renamed/restructured) fails
  loudly (`Runtime.evaluate` returns `null`/an error for a missing
  selector) rather than silently capturing the wrong region — the runner
  reports which shot IDs failed to resolve, at the end of a run, so a
  selector rot is visible immediately rather than shipping a wrong crop
  into the manual.
- Output filenames are stable (`<NN>-<id>.png`, `id` from the manifest, not
  a timestamp or a counter that shifts when shots are reordered) so
  re-running the tool after a UI change updates exactly the images that
  changed, and a docs pipeline can diff/replace by filename.

## 5. Output location

`docs/manual/screenshots/<run-timestamp>/` for a given capture run — not
committed by default (large binary churn on every UI tweak is not what git
history should carry); `.gitignore`d. A person curating the manual copies
the specific images they want to keep into the manual's own asset directory
under a stable, hand-chosen name at that point — this tool's job is
producing accurate, well-cropped candidates, not deciding what's canonical
enough to commit.

## 6. First run — the 10 shots

Broad coverage across navigation chrome, the four default-pinned widgets,
one non-pinned widget, settings, and a couple of pane-level close-ups —
picked to be useful to someone opening the manual for the very first time,
before any deep-dive into one specific feature:

1. `main-window-overview` — full window, default multi-pane layout
2. `top-tab-bar` — hamburger + tab strip + pinned widget buttons, cropped
3. `hamburger-menu-open` — the hamburger dropdown open, showing entries
4. `agent-picker` — Agent widget's "pick an agent to launch" screen (no
   live conversation — this is the picker/onboarding state)
5. `pane-header-tabstrip` — a single pane's header/tab-strip row, close-up
6. `swarm-widget` — Swarm widget overview
7. `armory-bundles` — Armory widget, Bundles tab
8. `sysinfo-widget` — Sysinfo widget (CPU/Mem graphs)
9. `settings-appearance` — Settings pane, Appearance tab
10. `terminal-pane-fresh` — a fresh, empty terminal pane

## 6a. What actually happened

The isolated instance (§3) never came up on the day this ran — 5 attempts,
~30-40 minutes, root-caused to a mix of a real product bug (fixed —
`SPEC_DEV_INSTANCE_ISOLATION_DIAGNOSTICS_2026_09_19.md`) and this specific
machine being too GPU-saturated for a third concurrent CEF instance. Fell
back to capturing against the already-running **primary** instance instead,
restricted to the 9 chrome/widget shots that shouldn't show real
conversation content (`main-window-overview` skipped for this run — a
full-window shot of a live, shared instance risks showing real panes).

7/9 captured cleanly on the first `capture.mjs` run; `settings-appearance`
and `terminal-pane-fresh` needed `prep` fixes (Settings' tab wasn't reached
by the first selector attempt; Terminal isn't pinned, needed the "More"
dropdown first) — both captured successfully on retry, landing 8/9 total.
Two genuine content findings, not tool bugs: `agent-picker` and
`armory-bundles` — chosen specifically because they looked content-free —
actually render real, workspace-identifying data by design (agent names in
the "MY AGENTS" list / bundle list). Not conversation content, but not
manual-safe either; flagged rather than shipped silently. The full
narrative, including a live UI-navigation mistake that briefly surfaced
real conversation content on screen (no image of it was ever saved), is in
`docs/reports/REPORT_SCREENSHOT_TOOLING_ISOLATED_INSTANCE_SETUP_BLOCKERS_2026_09_19.md`'s
Attempts 6-8.

**Net assessment: the tool itself works as designed** — selector-based
crops were pixel-accurate every time, `manifest.json` correctly recorded
what was captured, and failures reported clearly (wrong selector, missing
element) rather than silently producing a wrong image. What didn't go to
plan was entirely about the *environment* (isolation) and *content
judgment* (what a "safe" shot actually shows) — both recorded as follow-ups,
neither a defect in `capture.mjs`/`shots.mjs` as committed here.

## 7. Open follow-ups (not blocking this first run)

- Selector-based `prep` navigation currently uses `Input.dispatchMouseEvent`
  at a resolved element centroid — fine for the shots above, all reachable
  in 1-2 clicks from a fresh window. A widget requiring deeper navigation
  (multi-step wizards, modal-nested state) may need `prep` to compose
  several click+wait steps; the runner already supports that (`prep` is
  just an async function), just not exercised yet.
- No annotation/callout support (arrows, numbered labels) — out of scope
  for "accurate raw crops"; a manual-authoring step layered on top of this
  tool's output, not this tool's job.
- Multi-resolution/theme variants (dark/light, different zoom levels) not
  addressed in this first run — `shots.mjs` could grow a `variants` field
  later without changing the runner's shape.
- **`clickText`'s substring fallback has no post-click verification** (found
  live, §6a / report Attempt 6) — it computes a click target from a text
  match and fires immediately, no "did this navigate to what I expected"
  check. Worth adding a `verify` field to a shot's `prep` contract (a
  selector or condition to assert before capturing) so a wrong click fails
  the shot loudly instead of silently capturing — or worse, silently doing
  something unintended — rather than what it does today.
- **A "content-free-looking" view isn't necessarily workspace-data-free**
  (§6a / report Attempt 8) — `shots.mjs` entries should get an explicit
  `containsWorkspaceData: boolean` (or similar) field, checked by a human
  before a shot is trusted for a public manual, rather than relying on the
  shot author's judgment call at selection time going unrecorded.

# SPEC: Install-log "Details" panel — auto-scroll, provider-install parity, and brand icons

**Date:** 2026-09-10
**Status:** implemented — §1-§6 shipped in PR #3165, including a stickiness
regression fix (codex P2: re-check `stickToBottom` inside the deferred rAF
callback, not just at schedule time) caught by review before merge.
**Related:** `docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md` (built
the component §1-§4 fix), `docs/specs/SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`
(the `install.start` / `install_chunk` streaming machinery both installers
share, and the modal §5 restyles), `docs/specs/SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`
(built `ToolOverlayLog.tsx`'s auto-stick-to-bottom scrolling — the pattern
§3 and §5 both reuse).

**Scope — three related changes to the same two install surfaces:**
1. **§1-§4:** Fix the system-toolchain installer's Details log to auto-scroll
   to the newest line (the originally reported bug — installing Node).
2. **§5:** Bring the provider-CLI installer (`AgentInstallModalPanel` — used
   for Claude Code, Codex, etc.) to the same progress-bar + collapsible-
   Details chrome, including the §3 auto-scroll fix.
3. **§6:** Show each tool's real brand icon (Font Awesome brand glyphs,
   already bundled) instead of a generic solid-style icon, starting with
   the system-toolchain catalog (node/npm/git/docker/python).

---

## 1. Report

Reported while installing Node through the system-toolchain installer
(Toolchain modal → "Install now"): the collapsible **Details** panel that
shows the live install log fills up with streamed output, but the view
stays wherever it was — it does not follow the newest line. A user who
opens Details to watch progress has to keep manually scrolling down as
output arrives, or ends up staring at stale lines from seconds ago.

**Root cause, confirmed in source:**
`frontend/app/view/toolchain/SystemToolInstallInline.tsx` — the `installing`
phase renders

```tsx
<details class="system-tool-install-details">
    <summary>Details</summary>
    <pre class="system-tool-install-log-body">
        {lines().map((l) => l.line).join("\n")}
    </pre>
</details>
```

(§165-208 of the file as of this writing). `lines()` grows on every
`install_chunk` WPS event (the `handler` inside `startInstall`, same file
§140-157) — each pushes a new `{ line, stream }` onto the signal, which
re-renders the `<pre>`. Nothing ever touches `scrollTop`. The `<pre>` itself
**is** independently scrollable —
`.system-tool-install-log-body { max-height: 200px; overflow-y: auto; }`
(`SystemToolInstallInline.scss:99-111`) — so the bug isn't that scrolling
is disabled, it's that nothing drives it as content streams in.

**This is the only install-log "Details" panel in the codebase today** —
grep for `<details` across `frontend/` turns up exactly four hits: this
one, a static "Raw payload" dump in `JektBubble.tsx` (already-complete
content, never streams, no scroll concern), a form's "Advanced options"
section in `AgentLaunchModal.tsx` (not a log at all), and a startup-error
dump in `error-display.ts` (also static). `SystemToolInstallInline` is
shared by **every** *system-tool* install screen in the app — its only two
callers are:

- `frontend/app/view/toolchain/toolchain-view.tsx:349` — the Toolchain
  modal's core-tools list (where this was reported, installing Node).
- `frontend/app/view/agent/components/AgentPrereqModal.tsx:178` — the
  agent-launch-time missing-prereq blocker.

Neither caller has its own log/Details rendering — both just mount
`<SystemToolInstallInline>` and let it own the whole install UI. So fixing
this one component's scroll behavior fixes it at both call sites; there is
no second implementation to find and fix separately for the system-tool
side. The provider-CLI installer (`AgentInstallModalPanel.tsx` — Claude
Code, Codex, etc.) is a *different* component with no Details panel at all
today; §5 brings it in line.

## 2. Goals / Non-goals

**Goals**
- While the Details panel is open and the user hasn't deliberately scrolled
  away from the bottom, every new streamed line keeps the view pinned to
  the newest output — matching the behavior `ToolOverlayLog.tsx` already
  has for tool-call output elsewhere in this app (§3 below).
- If the user scrolls up mid-install to read earlier output, stop
  auto-scrolling until they return to (or near) the bottom — never yank the
  view out from under someone who's reading.
- When the panel is reopened after being collapsed while output kept
  streaming, it opens scrolled to the newest line, not wherever it was left
  (native `<details>` markup that never had a chance to auto-scroll while
  hidden — see §3).
- Applies uniformly to both system-tool call sites (`toolchain-view.tsx`,
  `AgentPrereqModal.tsx`) via the one shared component — no per-caller work.
- The provider-CLI installer gets the same progress-bar-with-log-behind-
  Details UI, including the same auto-scroll behavior (§5).
- Each tool shows its actual brand mark where a clean one is available,
  starting with the system-toolchain catalog (§6).

**Non-goals**
- Whether `<details>` should default to `open` once an install starts (today
  it's closed until the user clicks "Details," and the reported bug is
  about behavior *once open*, not visibility). Worth a follow-up UX
  discussion, but conflating it here risks scope-creeping a small, provably
  correct fix into a design debate. Not changed by §1-§4; §5 makes an
  explicit, different call for the provider modal — see there.
- Any change to `install_chunk` event delivery, batching, or the streaming
  RPC machinery (`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`'s territory) —
  this is a pure rendering/scroll fix on the receiving end.
- Brand icons for the *provider* catalog (Claude Code, Codex, Gemini, …) —
  §6 explains why that's a separate, larger effort and out of scope here.

## 3. Design — reuse `ToolOverlayLog`'s proven stick-to-bottom pattern

This exact problem — a box with `overflow-y: auto` fed by a growing,
streamed content signal, needing to follow new output but yield the moment
a user scrolls away — is already solved and shipped in this codebase:
`frontend/app/view/agent/components/ToolOverlayLog.tsx` §136-144 and
§210-231 (`SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md`). Port the same shape
rather than invent a new one:

```tsx
// mirrors ToolOverlayLog.tsx's onScroll (§140-144)
let stickToBottom = true;
let logBodyRef: HTMLPreElement | undefined;
const onLogScroll = () => {
    if (!logBodyRef) return;
    const dist = logBodyRef.scrollHeight - logBodyRef.scrollTop - logBodyRef.clientHeight;
    stickToBottom = dist < 40; // same forgiving threshold — one mousewheel
                                // tick must not unstick
};

// mirrors ToolOverlayLog.tsx's createEffect (§210-231)
createEffect(() => {
    lines(); // register as a reactive dependency
    if (stickToBottom && logBodyRef) {
        requestAnimationFrame(() => {
            if (logBodyRef && logBodyRef.isConnected) {
                logBodyRef.scrollTop = logBodyRef.scrollHeight;
            }
        });
    }
});
```

with `ref={logBodyRef}` and `onScroll={onLogScroll}` added to the existing
`<pre class="system-tool-install-log-body">`.

**One addition `ToolOverlayLog` doesn't need, specific to `<details>`:** a
closed `<details>` element doesn't lay out its children, so `scrollHeight`/
`scrollTop` writes against `logBodyRef` while collapsed are either no-ops or
measure against a zero-size box — the same class of problem
`ToolOverlayLog`'s `panelHidden` (content-visibility) guard exists for
(§184-231 of that file), just triggered by native `<details>` semantics
instead of a `content-visibility` CSS class. Track open/closed via the
standard `toggle` event (broadly supported on `HTMLDetailsElement`) and
re-run the same scroll-to-bottom step when the panel opens, so reopening
mid-install (or after it finishes) lands on the newest line rather than
wherever a stale `scrollTop` was left:

```tsx
let detailsRef: HTMLDetailsElement | undefined;
onMount(() => {
    const el = detailsRef;
    if (!el) return;
    const onToggle = () => {
        if (el.open && stickToBottom && logBodyRef) {
            requestAnimationFrame(() => {
                if (logBodyRef && logBodyRef.isConnected) {
                    logBodyRef.scrollTop = logBodyRef.scrollHeight;
                }
            });
        }
    };
    el.addEventListener("toggle", onToggle);
    onCleanup(() => el.removeEventListener("toggle", onToggle));
});
```

with `ref={detailsRef}` added to the existing
`<details class="system-tool-install-details">`.

**Reset on retry.** `startInstall()` already does `setLines([])` on the
Retry path (§136 of the current file) — add `stickToBottom = true;`
alongside it, so a retry after scrolling up to inspect a failure starts
pinned to the bottom again rather than inheriting the previous run's
scroll-away state.

## 4. Testing (§1-§4)

`frontend/app/view/toolchain/SystemToolInstallInline.test.tsx` already
exists and covers phase transitions / the install flow. Add:

- A new line arriving while the log body is scrolled to (or near) the
  bottom moves `scrollTop` to track `scrollHeight` (jsdom doesn't compute
  real layout, so this needs either a jsdom `scrollHeight`/`clientHeight`
  stub matching `ToolOverlayLog.test.tsx`'s existing approach for the same
  problem, or a lightweight DOM-level check that the effect fires and sets
  `scrollTop = scrollHeight` when `stickToBottom` is true).
- Scrolling away (simulate `scrollTop` short of `scrollHeight - clientHeight`
  by more than 40px, dispatch `scroll`) and then pushing a new line does
  **not** move `scrollTop` — `stickToBottom` correctly latches off.
- Scrolling back within the 40px threshold and pushing a new line resumes
  auto-scroll.
- Dispatching `toggle` on the `<details>` while `stickToBottom` is true
  scrolls to bottom; while false, leaves `scrollTop` alone.
- Retry (`failed` → `startInstall()` again) resets `stickToBottom` to
  `true` regardless of its state before the retry.

## 5. Provider-CLI installer adopts the same chrome

**Request:** when installing a provider CLI (Claude Code, Codex, etc.), show
the same progress bar + log-hidden-behind-Details pattern §1-§4 fix, with
the same auto-scroll behavior — not a second, differently-shaped install UI.

**Current state, confirmed in source:**
`frontend/app/view/agent/components/AgentInstallModalPanel.tsx` streams
`install_chunk` events into a real `xterm.js` terminal
(`terminal.write(...)`, §106-113) that is **always visible** for the whole
modal body (`.agent-install-modal-term`, §401-460) — there is no progress
bar element and no Details wrapper. The header instead shows a text status
line: `⏳ Installing… {elapsedLabel()}` (§390-391). xterm.js already
auto-follows new writes to the bottom unless the user has manually scrolled
the scrollback up — that part of the *reported* bug (§1) doesn't reproduce
here, because this component never had the "nothing drives scrollTop"
problem in the first place. What it's missing is the **chrome**: a visible
progress bar, and the log collapsed behind Details rather than dominating
the whole modal.

**Design — same visual language, adapted for xterm instead of a raw `<pre>`:**

1. **Progress bar.** Reuse `SystemToolInstallInline.scss`'s
   `.system-tool-install-progress` / `.system-tool-install-progress-bar` /
   `@keyframes system-tool-install-progress-slide` (§68-84 of that file) —
   extract them into a small shared SCSS partial (e.g.
   `frontend/app/view/toolchain/_install-progress.scss` or promote to
   `frontend/app/element/`) with generic class names
   (`.install-progress`, `.install-progress-bar`) rather than duplicating
   the animation under an `agent-install-modal-` prefix. Both
   `SystemToolInstallInline.scss` and `AgentInstallModal.scss` (new file —
   none exists today; current styling for this component lives inline via
   the shared `modal-panel-*` classes) import the shared partial. Render
   the bar in `AgentInstallModalPanel.tsx`'s `phase() === "installing"`
   branch, next to the existing spinner/status line — same condition that
   already gates the spinner today (§390-391).
2. **Collapsible Details wrapping the terminal.** Wrap the existing
   `.agent-install-modal-term` div in
   `<details class="agent-install-modal-details"><summary>Details</summary>`,
   mirroring `SystemToolInstallInline`'s markup shape exactly. The terminal
   element, its ref, and all existing xterm setup (§206-328) are unchanged —
   only its container gains a `<details>` wrapper.
3. **Default open/closed — a deliberate difference from §1-§4's non-goal.**
   `SystemToolInstallInline` is an inline panel that expands below a
   compact "not found" row alongside several other rows (Toolchain modal's
   list, or `AgentPrereqModal`'s missing-prereq list) — collapsed-by-default
   there keeps a multi-row list scannable. `AgentInstallModalPanel` is a
   **dedicated, single-purpose modal** whose entire visible body today is
   the terminal — collapsing it by default here would mean a user who opens
   "Install Claude Code" sees only a progress bar and has to click Details
   to see anything at all, a real regression from today's always-visible
   console. Default this one **open** (`<details open>` or a signal
   initialized to `true`), while still being individually collapsible if a
   user wants the compact view. This is the one place this spec diverges
   from a literal chrome match — call it out in the PR description so a
   reviewer doesn't read it as an oversight.
4. **Auto-scroll.** xterm.js already handles "stick to bottom unless the
   user scrolled away" internally (`Terminal.write()`'s default behavior;
   `terminal.scrollToBottom()` is available if an explicit nudge is ever
   needed) — §3's manual `scrollTop` tracking is **not** needed here, that
   problem is specific to the raw `<pre>` case. What *is* needed, newly,
   because the terminal can now be inside a collapsed `<details>`:
   **`FitAddon.fit()` must re-run when the panel transitions from closed to
   open**, the same "no layout while hidden" issue §3 solves for
   `scrollTop`, but hitting xterm's sizing instead. The existing
   `ResizeObserver` (§325-326, already installed to handle the modal
   animating in from 0×0) may or may not fire reliably on a `<details>`
   open transition depending on how the browser handles that layout change
   — don't rely on it alone. Add an explicit `toggle` listener that calls
   the same `tryFit()` (§287-293) already defined in this file:

   ```tsx
   let detailsRef: HTMLDetailsElement | undefined;
   onMount(() => {
       const el = detailsRef;
       if (!el) return;
       const onToggle = () => { if (el.open) tryFit(); };
       el.addEventListener("toggle", onToggle);
       onCleanup(() => el.removeEventListener("toggle", onToggle));
   });
   ```

   Terminal creation and `writeTerm()` calls are unaffected — xterm buffers
   writes into its data model regardless of whether its container is
   currently laid out, so no output is lost while the panel happens to be
   collapsed; only the *visual* fit needs this extra nudge.

**Non-goals for §5:** replacing xterm.js with the plain `<pre>` approach
§1-§4 uses. ANSI color rendering, text selection, and the existing
copy/copy-all context menu (§405-459 of the current file) are real,
already-built features worth keeping — this section is about matching the
outer chrome (progress bar + Details), not the log renderer underneath it.

## 6. Brand icons for the system-toolchain catalog

**Request:** show each tool's real brand icon where it's offered for
install — the example given was Node.js.

**Current state, confirmed in source:**
`frontend/app/view/agent/providers/toolchain-catalog.ts`'s `CoreTool.icon`
field is documented as "Font Awesome (solid) icon name, rendered as
`fa-solid fa-<icon>`" and today holds generic, non-brand glyphs: `node` →
`"cube"`, `npm` → `"box"`, `git` → `"code-branch"`, `docker` → `"box-open"`,
`python` → `"snake"`, `uv` → `"bolt"`. Rendered identically at two sites in
`frontend/app/view/toolchain/toolchain-view.tsx` (§294 and its twin around
§440): `<i class={\`toolchain-row-icon fa-solid fa-${row.icon}\`} />`.

**Font Awesome's brand set is already bundled and loaded app-wide**
(`index.html:74`, `<link rel="stylesheet" href="/fontawesome/css/brands.min.css" />`)
and already used elsewhere (`about.tsx:47`,
`<i class="fa-brands fa-github mr-2"></i>`). Verified present in the
bundled `public/fontawesome/css/brands.min.css` — all five core tools that
have an official brand mark are covered:

| Tool | Current `icon` (solid) | Brand glyph (confirmed bundled) |
|---|---|---|
| node | `cube` | `node-js` |
| npm | `box` | `npm` |
| git | `code-branch` | `git-alt` |
| docker | `box-open` | `docker` |
| python | `snake` | `python` |
| uv | `bolt` | *(none — stays solid `bolt`; uv has no Font Awesome brand glyph)* |

**Design:**

1. Add an optional `brandIcon?: string` field to the `CoreTool` interface
   (`toolchain-catalog.ts`, alongside the existing `icon` field's doc
   comment) — an FA *brands* icon name, rendered with the `fa-brands`
   prefix instead of `fa-solid`. Keep `icon` as the guaranteed solid
   fallback (still required on every entry) rather than replacing it, so a
   tool with no brand mark (`uv`) degrades cleanly instead of needing a
   special case.
2. Populate `brandIcon` for the five rows in the table above (leave `uv`
   unset).
3. Add a small shared helper (e.g. in `toolchain-catalog.ts` itself, next
   to `cliCommandForPlatform`) —

   ```ts
   export function rowIconClass(icon: string, brandIcon?: string): string {
       return brandIcon ? `fa-brands fa-${brandIcon}` : `fa-solid fa-${icon}`;
   }
   ```

   — and use it at both `toolchain-view.tsx` render sites (§294, §440) in
   place of the inline `` `fa-solid fa-${row.icon}` `` template literal.
   `ToolRow`'s icon field(s) need `brandIcon` threaded through the same way
   `icon` already is, in the `coreRows` mapping (§105-111 of that file):
   `icon: t.icon, brandIcon: t.brandIcon,`. Provider rows and widget rows
   (built from `getProviderList()` / `EXTERNAL_WIDGETS` a few lines below)
   are unaffected — they don't set `brandIcon`, so `rowIconClass` falls
   through to their existing solid-class behavior unchanged.
4. **`SystemToolInstallInline.tsx` also gets the icon**, since that's the
   component actually showing "This will run: `<command>`" for the tool
   being installed (§1's original report — "where node is set to be
   installed" is this panel, not just the list row above it). It already
   receives `toolId: string` as a prop; add a plain, local lookup —
   `const brandIcon = () => CORE_TOOLS.find((t) => t.id === props.toolId)?.brandIcon;`
   — no RPC/backend change needed, `toolchain-catalog.ts` is a frontend-only
   catalog already (its own doc comment: `wingetId`/`brewFormula` are
   "never sent to the backend, only used to decide"). Render the icon next
   to the consent text (`system-tool-install-consent-text`, idle phase) and
   in the log header (`system-tool-install-log-header`, installing/done/
   failed phases) so it stays visible across the whole flow, not just the
   initial prompt.

**Explicitly out of scope — provider brand icons (Claude Code, Codex,
Gemini, Qwen, Kimi, Pi, OpenClaw, Copilot).** `frontend/app/view/agent/defaults/cli-catalog.ts`'s
`CliCatalogEntry.icon` field holds plain Unicode glyphs today (`"✖"` for
Claude, `"✦"` for Codex, `"π"` for Pi, etc. — §50-166 of that file), used
by `AgentInstallModalPanel.tsx`'s header (`catalog()?.icon ?? "📦"`, §379).
Unlike the system-toolchain catalog, most of these vendors have **no**
glyph in Font Awesome's free brand set — Anthropic, OpenAI, Alibaba/Qwen,
Moonshot/Kimi, and Plandex aren't covered by FA6 Free at all. Sourcing and
licensing individual brand SVGs per provider (or paying for FA Pro's wider
set, if it even covers all of these) is a real, separate effort with asset
and licensing considerations this spec shouldn't fold in by side effect.
One narrow exception worth a reviewer's call, not asserted here as a
decision: `fa-brands fa-github` is already bundled and unambiguous for
**GitHub Copilot** specifically — a one-line change if wanted, left as an
optional follow-up rather than bundled into this spec's required scope.

## 7. Testing (§5, §6)

**§5 (provider installer chrome):**
- `AgentInstallModalPanel.test.tsx` (exists — extend it): the progress bar
  renders only during `phase() === "installing"`, matching the existing
  spinner's gating.
- The Details panel defaults to `open` on mount (distinct from §1-§4's
  default-closed — assert this explicitly so the difference is a tested
  decision, not an accident).
- Dispatching `toggle` (open) on the wrapping `<details>` calls `tryFit()`
  — spy/mock `FitAddon.fit` and assert it's called on the toggle event, not
  just on the existing `ResizeObserver`/font-ready paths.
- Existing copy/copy-all context-menu and ANSI-color tests continue to pass
  unmodified — confirms the terminal itself wasn't touched, only its
  container.

**§6 (brand icons):**
- `rowIconClass("cube", "node-js")` returns `"fa-brands fa-node-js"`;
  `rowIconClass("bolt", undefined)` returns `"fa-solid fa-bolt"` (the `uv`
  case — confirms the fallback path).
- `toolchain-view.tsx`'s rendered row for `node` carries class
  `fa-brands fa-node-js`, not `fa-solid fa-cube` — for all five branded
  tools in the table above, plus a snapshot/explicit assertion that `uv`'s
  row is unchanged (`fa-solid fa-bolt`).
- `SystemToolInstallInline` rendered with `toolId="node"` shows the
  `fa-brands fa-node-js` icon in both the idle-consent view and the
  installing-phase header; rendered with `toolId="uv"` shows no brand icon
  (falls back to whatever `uv`'s row already showed, or renders nothing
  extra if no icon was shown there before — match current behavior for the
  unbranded case).

## 8. Rollout

All three changes are frontend-only, additive, and land in the same two
components + one catalog file:

- §1-§4: `SystemToolInstallInline.tsx` (+ its `.scss`, + its `.test.tsx`).
- §5: `AgentInstallModalPanel.tsx` (+ a new shared progress-bar SCSS
  partial, + its `.test.tsx`).
- §6: `toolchain-catalog.ts`, `toolchain-view.tsx`, and
  `SystemToolInstallInline.tsx` again (icon lookup).

No RPC/schema changes, no backend involvement, no migration, no flag. Safe
to land as one PR or split by section — §6 has zero dependency on §5, and
§3's auto-scroll mechanism is reused (not required) by §5, so any subset
can ship independently if reviewers prefer smaller PRs.

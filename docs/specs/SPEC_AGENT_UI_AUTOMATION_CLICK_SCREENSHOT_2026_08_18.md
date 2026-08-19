# SPEC: Agent UI Automation (Click + Screenshot) as a First-Class Agent App API Capability

Status: Phase 1 implemented (branch `agenty-0629j/feat-agent-ui-automation`)
Date: 2026-08-18

**Implementation update (same day):** §6's scoping design as originally
written ("default every tool to the caller's own window/pane") turned out to
be stricter than intended once built — the muxbus sign-in chip that
motivated this whole spec (§0/§7) lives in the status bar, which is app-wide
chrome rendered as a *sibling* of the pane-content tree
(`frontend/app/workspace/workspace.tsx:85`), not a descendant of any pane's
`[data-blockid]` wrapper. Strict pane-subtree scoping made it structurally
unreachable — the exact motivating scenario couldn't work. Resolved
(repo-owner confirmed): `UIClick`/`UIQuery` allow an element if it's either
inside the caller's own pane OR not inside ANY pane's `[data-blockid]`
wrapper (shared chrome — not owned by any one agent, so reaching it isn't
the kind of leak reaching another agent's pane would be); still never allow
reaching a *different* pane. `UIScreenshot` deliberately keeps the original
strict pane-only rule — a screenshot's CDP `clip` is a single rectangle
that can't exclude another agent's pane sitting geometrically between the
caller's pane and a chrome element the way an element-level filter can, so
widening it the same way would leak pixels. See
`agentmux-cef/src/browser_api/scripts/query.js`'s `__amq_allowed_for` for
the actual implementation.

**Review findings (2026-08-19, PR #2662 — Codex + reagent, converging
independently on the same root cause for one of them):**
- **Fixed, P0:** `__amq_allowed_for` only checked whether a matched element
  was *inside* a different pane's `[data-blockid]` wrapper, never whether
  it *contained* other panes as descendants — `body`/`#root`/any shared
  layout container isn't itself inside any pane, so it was waved through as
  "shared chrome," and `UIQuery` would return its full `textContent`
  (every pane's content, including other agents'), while `UIClick` could
  dispatch a real click at its centroid, landing on a different pane's
  actual element. Fixed: an element with no `[data-blockid]` ancestor is
  only allowed if none of its own descendants belong to a different block
  either. Verified live: querying `body` now correctly returns zero
  matches instead of leaking all panes' content.
- **Fixed, P0 (host_ipc.Register):** the registration handshake
  unconditionally overwrote `state.host_ipc` with whatever the caller
  supplied, over the same shared `X-AuthKey` every agent's environment
  holds — any agent could permanently hijack the credential for the whole
  session, with no self-heal. Fixed: rejects any re-registration whose
  port/token differ from what's already stored (identical re-registration
  is still a harmless no-op).
- **Acknowledged, not fixed — genuine open gap (Codex P1):** `block_id` on
  `/api/v1/ui/*` is trusted from the request body. The MCP tool schema
  never exposes it as an agent-settable argument, so a *well-behaved*
  caller going through the sanctioned MCP path is scoped to its own pane —
  but an agent that bypasses the MCP wrapper and calls the HTTP route
  directly (it holds the same shared `X-AuthKey`) can supply a *different*
  pane's real `block_id`, and nothing here can tell it doesn't actually
  belong to the caller. This is not unique to UI automation — it's how
  every existing App-API MCP-bound route's identity-stamping already
  works (`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`'s S1) — but this
  spec's own §6 language ("holds by construction," "never
  agent-suppliable") overclaimed it as airtight, which review correctly
  called out. Fully closing it needs a per-agent credential distinguishing
  individual agents at the HTTP layer, which doesn't exist anywhere in the
  App-API surface today; that's a genuinely separate, larger piece of work
  than this PR, tracked as follow-up rather than silently left
  undocumented.

## 0. Origin

This spec was prompted by a concrete gap hit in-session: verifying a fix to
the muxbus sign-in "Cancel" button (branch
`agenty-0629j/fix-muxbus-signin-cancel`) required clicking a real button in
the running app and observing the visual result — the same thing a human
tester would do. No agent-facing tool could do that; the agent had to fall
back to automated tests + `tsc`/`cargo build` and ask the human to do the
actual click-through. The repo owner reframed this as a gap in a *core*
feature, not a one-off: agents should be first-class operators inside
AgentMux, able to drive its UI (click, read rendered state, screenshot) the
way any other App API capability (Shell, OpenMedia, FocusWindow, …) lets
them operate the rest of the app.

## 1. The load-bearing fact: this is mostly already built

AgentMux already ships a full CDP (Chrome DevTools Protocol) automation
layer: `agentmux-cef/src/browser_api/` (`cdp.rs`, `resolver.rs`,
`routes.rs`, `types.rs`, `scripts/query.js`), designed and specced in
`docs/specs/SPEC_BROWSER_DOM_API.md` / `docs/specs/PLAN_BROWSER_DOM_API.md`.
It is real, shipped, production code — not a draft — currently used to
replace flaky Win32 pixel-coordinate clicks in the `pane-focus-stress.ps1`
test harness. It already implements, over CDP's `Page.*`/`Input.*`/
`Runtime.*` domains against a specific CEF `Browser` target:

- `screenshot` — `Page.captureScreenshot(fromSurface: true)`, PNG/JPEG, returns base64.
- `click_element` — a **real synthetic mouse event** (`Input.dispatchMouseEvent`
  mousePressed+mouseReleased) at a DOM element's computed centroid, resolved
  via an injected `query.js` helper — not a scripted `.click()`, so focus /
  `:focus-visible` / pointer listeners behave like a human click.
- `query` — CSS selector → matching elements (tag/text/attrs/rect/focused).
- `focus_element`, `dispatch_key`, `eval`, `navigate`/`back`/`forward`/`reload`.

Two facts, both confirmed directly in code, make this capability much
closer to "agent-ready" than it looks at first glance:

1. **The remote debugging port is already on, unconditionally, in every
   build.** `agentmux-cef/src/lib.rs:854` sets
   `remote_debugging_port: debug_port as i32` on CEF's process-wide
   `Settings` with no dev/release gate at that call site (dev picks 9223,
   release 9222, falling back to an OS-assigned free port per instance to
   avoid collisions across parallel AgentMux instances). `docs/specs/SPEC_TEST_API_ACCESS.md`
   §4.3 rejected using CDP for a *different* purpose (fetching `auth_key`
   for test-harness bootstrapping) partly on the premise "DevTools is
   disabled on release by default (and should be)" — that premise is
   **stale**; `browser_api` has since shipped with the port open in both
   dev and release, in production, gating access via `ipc_token` instead of
   by disabling the port.
2. **The entire main AgentMux UI — every pane, the status bar, the popover
   this session's bug lived in — is itself just DOM inside ONE CEF
   `Browser` instance per top-level window**, not a native-widget UI with
   isolated web views. Confirmed in `agentmux-cef/src/app/mod.rs:970-1058`:
   `on_context_initialized` calls `browser_view_create(...)` exactly once
   per top-level window, loading the frontend URL; every pane/tab/widget is
   a SolidJS component in that one page's DOM tree. (The one exception —
   Browser-pane widgets, `view: "browser"` — embed a genuinely separate
   *sub*-browser for third-party web content, which is what `browser_api`
   was built to target.) CDP's `/json` endpoint enumerates **every**
   `Browser` in the process — main windows, pool windows, floating windows,
   and browser panes — as targets. `browser_api/resolver.rs`'s target
   resolution just happens to only ever have been *asked* to resolve
   `browser-pane-*` block ids by its current callers; nothing in the CDP
   layer itself restricts it to panes.

**Conclusion:** "click a button in AgentMux's own UI, take a screenshot of
it" is not a new capability to build from scratch. It's an existing, tested
capability whose target resolver needs widening (browser-panes-only →
any in-process `Browser`), plus a new agent-facing (MCP) wrapper. No new
CDP client, no new screenshot/click mechanism, no new dependency.

## 2. Prior art in this repo (read before building; reconcile, don't ignore)

- **`docs/specs/computer-use-pane.md`** — draft, unbuilt. Proposes a
  first-class "Computer Use pane" using Anthropic's `computer_20251124`
  tool contract: OS-level whole-desktop screenshot (`xcap`) + input
  injection (`enigo`), for driving *arbitrary third-party apps*.
  Explicitly **excludes AgentMux's own window from capture** ("no
  self-screenshot… prevents prompt injection from its own UI") — i.e. it
  solves the opposite problem from this spec. Its security model (one
  active session at a time via a machine-wide mutex, per-app approval
  gating, app-tier warnings) is good precedent to keep in mind if AgentMux
  ever adds real desktop-wide computer-use, but it's the wrong vehicle for
  "let an agent drive AgentMux's own UI" — that's a narrower, fully
  self-controlled problem (one known DOM tree, not an arbitrary unknown
  screen), and deserves a lower-risk mechanism. See §6 for why this spec
  deliberately stays inside the CDP/DOM boundary instead of reaching for
  `computer-use-pane.md`'s OS-level approach.
- **`docs/specs/SPEC_AGENT_BROWSER_CONTROL_2026_04_17.md`** — draft,
  unbuilt. Proposes agent-facing `browser_screenshot`/`browser_execute_js`
  MCP tools scoped to the *agent's own Browser pane*. Closest prior art in
  spirit (agent-facing, CDP-based) but doesn't cover the main-UI-window
  case (i.e. automating AgentMux's own chrome, not a pane's embedded web
  content), and no `browser_*` MCP tool exists today — confirmed zero
  matches for `browser_api`/`/agentmux/browser` in
  `agentmux-mcp/src/main.rs`.
- **`docs/specs/SPEC_TEST_API_ACCESS.md`** §4.3 — see §1 above re: the
  stale "disabled on release" premise. Its other objections (CDP is an
  "uncontrolled" wire protocol we don't own; frontend internals like
  `globalAtoms` aren't a stable API surface; per-call latency) are still
  worth carrying forward: this spec's MCP tools should be selector/DOM-based
  against *stable, purpose-built* hooks (data attributes / accessible
  names), not ad hoc `Runtime.evaluate` reaching into frontend internals —
  see §6.
- **`docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`** —
  independently recommends CDP as the right mechanism for read-only
  introspection of what the app is actually doing, explicitly out of scope
  for muxspect itself. Corroborates CDP as the house-favored "look from
  outside" mechanism.

## 3. Industry best practices (external research, 2026-08-18)

**(a) Anthropic's computer-use guidance** — run with least privilege
(ideally sandboxed/disposable), log the full action+screenshot sequence for
every step (audit + anomaly detection), map screen coordinates to the
actual rendered resolution carefully (resize bugs silently break click
accuracy), batch predictable multi-step actions to cut latency/cost, and
start with low-risk tasks given the tooling is still immature industry-wide.
([Best practices for computer and browser use with Claude](https://claude.com/blog/best-practices-for-computer-and-browser-use-with-claude))

**(b) Accessibility-tree / DOM-native grounding beats pixel coordinates.**
Structured, DOM-native automation cuts per-action latency from ~2-5s to
under ~500ms and token cost by an order of magnitude versus screenshot-driven
pixel-coordinate agents, and is far more robust to UI changes (a pixel-coordinate
script breaks the moment layout shifts by one pixel; a selector/role-based
action doesn't). Benchmarks show hybrid approaches (structured + vision) beat
either alone. The standard safety pattern layered on top: explicit permission
gating per new workflow/step, and a replayable audit trail of every
click/keystroke/screenshot.
([Accessibility is the first-class interface for AI agents](https://www.infoworld.com/article/4193332/accessibility-is-the-first-class-interface-for-ai-agents.html))

**(c) OpenAI's `computer-use-preview` `pending_safety_checks`.** Every
computer-use turn runs automated classifiers (malicious-instruction
detection on the screenshot, irrelevant-domain / sensitive-domain detection
on the current URL); a triggered check blocks the next action until the
*application* explicitly surfaces it for human confirmation and the caller
acknowledges it — and OpenAI is explicit that these checks are not a
substitute for the app's own sandboxing/validation, just a floor under it.
([Safety checks | OpenAI API](https://developers.openai.com/api/docs/guides/safety-checks))

**Why this matters for AgentMux specifically:** (b) is not a hypothetical
recommendation to adopt — AgentMux's existing `click_element` (§1) is
*already* the DOM-native, centroid-of-element approach industry practice
prefers over blind pixel coordinates, purely because it was built to fix a
flaky pixel-click test harness. Building this spec on top of `browser_api`
instead of `computer-use-pane.md`'s pixel/OS-level approach isn't just
lower-risk (§6) — it's already the higher-quality approach by the industry's
own stated preference, for free.

## 4. Recommended architecture — Phase 1

1. **Widen `browser_api/resolver.rs`'s target resolution** from
   "browser-pane block ids only" to any in-process CEF `Browser` — main
   window, pool window, floating window, or browser pane — addressed by
   whatever handle `FocusWindow`/`Layout` already use (`window_id` /
   `block_id`). No change to `cdp.rs` or the CDP call sites themselves;
   `screenshot`/`click_element`/`query`/`dispatch_key` are reused verbatim
   against the wider target set.
2. **Add MCP tools** (names illustrative — finalize during implementation):
   `UIScreenshot`, `UIClick`, `UIQuery`, `UIReadText`/`UIFocusInfo`. Follow
   the binding pattern from `specs/SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`:
   a curated subset (not 1:1 with every `browser_api` route), one shared
   App-API core function reused across transports where practical, identity/
   target stamped server-side from the trusted `AGENTMUX_BLOCKID` env (never
   agent-suppliable), sensitivity tagged explicitly per tool.
3. **Auth path:** `browser_api` currently lives on the CEF host's own IPC
   axum server, gated by `ipc_token` — a different auth scheme from
   `agentmux-srv`'s `X-AuthKey` that `agentmux-mcp` actually holds (per
   `SPEC_AGENT_APP_API_MCP_BINDINGS`, MCP tools reach `agentmux-srv`, not
   the CEF host directly). This spec needs either (a) `agentmux-srv` proxies
   the widened `browser_api` calls to the CEF host's IPC server internally
   (srv already knows how to reach the host it's paired with), keeping the
   `X-AuthKey`-only contract MCP tools already use, or (b) a documented,
   deliberate exception where these specific MCP tools also carry an
   IPC-token credential. Recommend (a) — it's a smaller trust-boundary
   change and keeps "agents only ever hold `AGENTMUX_LOCAL_URL`+`AGENTMUX_AUTH_KEY`"
   true without exception. Needs a design pass against the actual srv↔host
   IPC contract before implementation — flagged, not resolved, here.
4. **Screenshot response shape:** `agentmux-mcp/src/main.rs` has zero
   precedent for returning image bytes through an MCP tool's `content`
   array today (confirmed: no base64/`image/` handling anywhere in the
   file) — every tool replies with `{"type": "text", ...}`. Two options:
   - Follow the `OpenMedia` precedent: write the PNG to a temp file
     server-side, return the path (and optionally auto-open it in a Media
     pane via the existing `pane.open` core) rather than the bytes
     themselves.
   - Extend the MCP tool response to include actual image content, if the
     connected agent runtime can render it inline — **unconfirmed, see
     Open Questions (§8)**.
   Recommend defaulting to the file-based (OpenMedia-style) hand-off for
   Phase 1 since it needs no MCP protocol extension and has a shipped
   precedent to copy.

## 5. Scope

**In scope (Phase 1):**
- Widen `browser_api` CDP target resolution to any in-process AgentMux
  `Browser` (not just browser panes).
- New MCP tools for screenshot, selector/text-based click, and reading
  rendered state (`query`/`focus_info`), of the caller's **own** window by
  default.
- `agentmux-srv`-proxied auth path (§4.3) so MCP tools keep the existing
  `X-AuthKey`-only contract.
- Default-deny cross-agent / cross-window targeting (§6).
- Action + screenshot audit logging (§6), at least matching `Shell`'s
  existing `tracing::info!` bar.

**Out of scope (Phase 1 — explicitly deferred, separate future spec if
ever pursued):**
- OS-level whole-desktop screenshot/input injection
  (`computer-use-pane.md`'s `xcap`/`enigo` approach). Different risk
  profile (arbitrary third-party apps, real OS input queue, no DOM
  ground-truth) — not needed to solve "let an agent drive its own app,"
  and there is currently zero OS-level capture/injection code anywhere in
  the repo to build on (confirmed) — no reason to introduce that whole new
  dependency/attack surface for this problem.
- Cross-agent / cross-window targeting as a default-available capability
  (opt-in only, if ever built — see §6).
- iframe/subframe/worker targeting (already out of scope for `browser_api`
  itself, per `SPEC_BROWSER_DOM_API.md` §2.2/§11).
- Coordinate-based (non-selector) clicking — see Open Questions (§8).

## 6. Security model — the actual hard part

The CDP mechanics are solved (§1). The open problem this spec exists to
close is targeting/authorization, and there's a real gap to fix, not just
harden:

- **Existing window/pane-targeting tools (`FocusWindow`, `Layout`,
  `SetActiveTab`) are scoped to "within this AgentMux instance," not "within
  this agent's own windows."** Confirmed: `FocusWindow` defaults to the
  caller's own window via server-derived `AGENTMUX_BLOCKID` (good), but if
  an explicit `window_id` is supplied, the handler uses it with no
  ownership check against the caller — any agent can already focus (and by
  the same code shape, could rename/relayout) a *different* agent's window
  within the same instance. A Click/Screenshot tool inheriting this exact
  pattern would be strictly worse: it can *read* another pane's on-screen
  content (credentials in a form, private conversation text) and *act* on
  another agent's UI (e.g. clicking "Confirm" on someone else's destructive
  dialog) — not just steal focus.
- **The I1-I6 multi-instance isolation invariants don't cover this gap.**
  They govern OS-process isolation *between* AgentMux instances (pipes, job
  objects, PIDs); this capability's exposure is entirely *within* one
  instance, agent-to-agent — a different axis they were never meant to
  constrain. (Also worth noting for the record: if OS-level computer-use
  is ever built per `computer-use-pane.md`, I1-I6 wouldn't constrain that
  either — real screen/input APIs don't go through `wstore`-based
  addressing at all. One more reason to keep this spec inside the CDP/DOM
  boundary.)
- **Recommendation, combining the App-API sensitivity-tiering precedent
  (`SPEC_AGENT_APP_API_MCP_BINDINGS` §4/§7 — mutations gated behind an
  explicit per-agent capability flag, shipped after read-only tools) with
  OpenAI's `pending_safety_checks` posture (app-owned gating, not just a
  classifier):**
  1. Default every new tool to the caller's **own** window/pane
     (server-derived from `AGENTMUX_BLOCKID`, same `resolve_own` pattern
     `FocusWindow` already has) — cross-agent targeting, if it's ever
     wanted at all, should be a distinct, explicitly higher-sensitivity
     capability, not the default shape of the same tool.
  2. **No capability-flag mechanism actually exists in code today** — the
     App-API MCP bindings spec recommends one for identity mutations, but
     it's aspirational, not implemented (confirmed: no
     `capability_flag`/`CapabilityFlag` hits anywhere in
     `agentmux-srv`/`agentmux-common`). If cross-agent targeting is ever
     pursued, building that gate is a prerequisite, not a detail — flagged
     as a blocking open question (§8), not assumed solved.
  3. Treat "read arbitrary on-screen pixels" (screenshot) as sensitive by
     nature even scoped to the caller's own window, since a pane can
     render another agent's shared state, secrets typed into a form, etc.
     mid-capture. Log distinctly from routine tool calls — mirrors this
     repo's own jekt posture of "when in doubt, treat as sensitive."
  4. Audit log every `UIClick`/`UIScreenshot` call (actor, target,
     timestamp, and for click: what was clicked). `Shell` today only has
     `tracing::info!` lines, not a queryable audit trail — this spec
     shouldn't regress below that bar, and arguably should clear it given
     the higher sensitivity of "can act on and read the UI," not just
     "can run a shell command."
  5. Stay inside the CDP/DOM boundary (§5, out of scope) — this is both
     the industry-preferred lower-risk approach (§3b) and the natural
     boundary given AgentMux has zero OS-level capture/injection code to
     build on today.

## 7. Relationship to today's fix

The muxbus sign-in Cancel-button fix that prompted this spec
(`agenty-0629j/fix-muxbus-signin-cancel`) was verified without this
capability — automated Rust tests (`cargo test -p agentmux-srv --bin
agentmux-srv muxbus::pkce::`, 3/3 passing, including a new regression test
for the cancel path) plus a clean `tsc --noEmit` and `cargo build`. That's
solid coverage for "does the logic work," but not "does the button visually
render and behave correctly when a human clicks it" — the gap this spec is
about. Once Phase 1 ships, the same kind of fix could be verified end-to-end
by the fixing agent itself: open the popover, `UIClick` the Sign-in chip,
close the browser, `UIScreenshot` to confirm the chip now reads "Cancel,"
`UIClick` it, `UIScreenshot` again to confirm it's back to "Sign in" —
without a human's hands needed to close the loop.

## 8. Open questions

1. Can the MCP-connected agent runtimes actually render an image returned
   inline in a tool's `content` array, or must screenshots go through a
   file+pane hand-off (§4.4)? Blocks the Screenshot response-shape
   decision.
2. `srv`↔CEF-host IPC proxying (§4.3) — what does that contract actually
   look like today, and is proxying `browser_api` calls through it
   straightforward, or does it need its own design pass?
3. Selector-based `click` only, or does a coordinate-based fallback need to
   exist for elements `query.js` can't resolve (e.g. `<canvas>`-rendered
   content)? Recommend selector-only for Phase 1; revisit if a real
   canvas-automation need shows up.
4. The capability-flag mechanism assumed by §6.2/§4/§7 of
   `SPEC_AGENT_APP_API_MCP_BINDINGS` doesn't exist yet anywhere in code —
   is building it in scope for this spec's Phase 1, or a separate
   prerequisite spec?
5. Should there be a `computer-use-pane.md`-style "one active UI-automation
   session at a time" limiter, even scoped to CDP/DOM-only actions, to
   bound how much of a human's attention an agent can simultaneously
   command via the visible UI? Not strictly required by the CDP mechanism
   itself, but worth deciding deliberately rather than by omission.

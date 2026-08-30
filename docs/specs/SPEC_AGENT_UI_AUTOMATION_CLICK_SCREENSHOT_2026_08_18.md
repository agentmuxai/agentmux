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
- **Fixed for real, P0 (host_ipc.Register) — three review rounds:**
  1. *Original bug:* the registration handshake unconditionally overwrote
     `state.host_ipc` with whatever the caller supplied, over the same
     shared `X-AuthKey` every agent's environment holds — any agent could
     permanently hijack the credential for the whole session, with no
     self-heal.
  2. *First fix, then P1 re-review regression:* rejecting any conflicting
     re-registration outright broke a real, already-supported recovery
     path — srv survives a CEF host crash as a Job Object sibling, and the
     launcher's crash-budget relaunch restarts just the host, which
     legitimately gets a fresh `ipc_port`/`ipc_token` and needs to
     overwrite the dead registration. Fixed: probe the currently-registered
     host's own `/health` route before rejecting — still alive → reject
     (hijack), unreachable → accept (crash-restart recovery).
  3. *Second P0 re-review:* the liveness probe only helps once
     `state.host_ipc` already holds something to compare against. At every
     srv startup, and the window after any `restart_backend` before the
     host re-registers, `state.host_ipc` is `None` — nothing to probe — so
     any caller won that race outright, and could then stand up its own
     always-200 `/health` responder to permanently defeat the liveness
     check on every future legitimate registration too. Closed with a new
     credential, `AGENTMUX_HOST_REG_SECRET` — a shared secret known only to
     srv and the paired host, never to any agent process. Sourced from the
     launcher (`agentmux-launcher::srv_spawner::SrvSpawnResult::host_reg_secret`,
     minted once per `spawn_srv` call and given to both host and srv's
     spawn env, so it survives every host-only crash-restart the same way
     `auth_key` already does) or, in host-owned-spawn/dev mode, generated
     by the host itself and passed to srv at `spawn_backend` time. Checked
     BEFORE the `state.host_ipc` None/Some branching, via constant-time
     comparison (`host_ipc.rs::secret_matches`, same "HMAC then
     `Mac::verify_slice`" idiom as the WhatsApp webhook signature check) —
     an agent has no way to learn this secret, so it can never win the
     registration race regardless of `state.host_ipc`'s current value.
- **Fixed for real, P0 (Codex P1, escalated to P0-blocking by reagent
  after an initial doc-only pass didn't close it):** `block_id` is no
  longer a request field on `/api/v1/ui/*` at all. Every request instead
  carries an HMAC-SHA256 signature (`UiAutomationAuth`) over the caller's
  own `agent_id`, using that agent's own `AGENTMUX_JEKT_KEY` — the SAME
  per-agent signing key already shipped for jekt sender authentication
  (`agentmux_common::jekt_sign`), reused rather than inventing a parallel
  credential system. srv verifies the signature against the claimed
  agent's key on file, and only THEN derives that agent's actual current
  block_id server-side, from the global `ReactiveHandler` registry — never
  from the client. An agent without agent X's key cannot produce a valid
  signature claiming to be X, regardless of what it puts in the request
  body; the earlier "not a server-side enforcement boundary" caveat no
  longer applies to these three routes (see
  `agentmux-srv/src/server/ui_handlers.rs`'s module doc comment for the
  mechanism, and its regression test for the exact attack this closes:
  signing correctly as agent Z while claiming to be agent X). This IS a
  genuine, narrower instance of the broader systemic gap noted below —
  it doesn't retrofit the fix onto any OTHER existing App-API MCP-bound
  route, only these three.

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

> **SUPERSEDED FOR CAPTURE (2026-08-30).** §6's own-pane-only default was a
> *recommendation* — its own text says cross-agent targeting "if it's ever
> wanted at all" should be a separate capability, i.e. it defaulted closed
> because no mechanism existed to be selective, and it was never ratified by
> the repo owner. The owner has since directed that agents be able to capture
> anything. `SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`
> replaces it for **screenshots** with a tier model (open for every
> agent-to-agent tier, withheld only across an OS-user boundary, audited
> throughout).
>
> **Still in force for `UIClick`/`UIQuery`.** Reading and acting are different
> risks — this section's strongest concrete objection was clicking "Confirm"
> on another agent's destructive dialog, which no screenshot can do. Those
> tools keep own-pane scoping until separately decided.
>
> Reviewers: do not flag capture code for violating this section.


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

## 6.1 2026-08-21 addendum — narrow interim exception for `CaptureWindow` (repo-owner confirmed)

**This is a real, narrow, repo-owner-confirmed exception, not a claim of a
blanket policy change** — recorded here specifically so it's a citable spec
diff, not just a commit-message assertion, matching this repo's own
established standard for distinguishing a genuine decision from an
unverified one (see `CLAUDE.md`'s jekt security section for the same
pattern applied to a different subsystem: a real change ships with a spec
diff and tests, not just prose).

**What's confirmed, directly, by the repo owner, in this conversation, on
this date:** PR #2709's `CaptureWindow` tool — OS-level window capture via
`xcap`, which §5 places out of Phase-1 scope in its general form — ships
with **audit logging only**, not the capability-flag gate §6
recommendation 2 requires, as an accepted interim state. The full gate is
tracked separately: issue #2714 (agentmuxai/agentmux).

**Why this is narrower than "§5's out-of-scope boundary no longer applies":**

1. §5's stated reason for excluding the `xcap`/`computer-use-pane.md`
   approach was "arbitrary third-party apps, real OS input queue, no DOM
   ground-truth" — `CaptureWindow` doesn't have that shape. It's
   screenshot-only (no input injection at all — `enigo` is not a
   dependency), and scoped to AgentMux's own windows specifically (matched
   via `app_name()`, verified in testing to exclude non-AgentMux
   applications — confirmed capturing a password-manager window was
   possible before this scoping and is not possible after it). The
   residual capability is closer to "see a different AgentMux instance,"
   not "drive an arbitrary desktop app."
2. It also excludes the caller's own instance (verified in live testing
   against this repo's actual running process tree, not assumed — see PR
   #2709's commit history for the before/after evidence), which closes the
   worse of the two risks §6 already names for cross-window targeting
   generally (reading/acting on a *different agent's* pane within the same
   instance). What's left is narrower: a different AgentMux *instance's*
   window, which can belong to a different OS *user* on a shared machine —
   a real residual risk, not eliminated, which is exactly why the interim
   answer is audit logging (detection) rather than a claim that the risk is
   fully closed.
3. §6 recommendation 4 (audit logging, "actor, target, timestamp") is
   satisfied for this tool specifically — every call appends an NDJSON
   entry to `capture-window-audit.log` in the instance's own private data
   dir.

**What's still genuinely open, not resolved by this addendum:** §6
recommendation 2's capability-flag gate itself. This addendum does not
claim that requirement is satisfied — it records a deliberate, scoped,
time-bounded exception to shipping without it for this one tool, pending
issue #2714. A future PR that wants to *widen* `CaptureWindow` (e.g. add
input injection, or drop the AgentMux-only/own-instance-exclusion scoping)
would need its own fresh justification — this addendum covers exactly what
shipped in PR #2709, not a general loosening of §5/§6.

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

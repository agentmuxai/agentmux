# Analysis + Plan: Widening `UIScreenshot`/`UIClick`/`UIQuery` beyond "your own pane"

**Status:** analysis, not implemented. One half has a clear, low-risk path; the
other half is a genuine security-design task, not a quick widening.
**Author:** AgentY
**Date:** 2026-08-21
**Related:** `docs/specs/SPEC_AGENT_GENERIC_PANE_OPEN_TOOL_2026_08_21.md` (the
sibling "make agent-facing pane control general-purpose" effort this
continues), `docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`
(the existing design — already anticipates and explicitly defers exactly this
ask), `agentmux-srv/src/server/ui_handlers.rs`, `agentmux-mcp/src/main.rs`
(`UIScreenshot`/`UIClick`/`UIQuery`/`FocusWindow` handlers),
`agentmux-cef/src/browser_api/{routes.rs,resolver.rs,scripts/query.js}`.

---

## 0. Motivation and a scope correction made during this analysis

The concrete trigger: while testing ABF, I couldn't screenshot a *separate
`task dev` instance's window* (a different OS process, different `srv`,
different agent registry entirely) — my `UIScreenshot` tool only sees my own
pane. The operator's response, generalizing the ask: *"UIScreenshot needs to
be a general purpose tool. you should have access to everywhere."*

**Investigating the actual mechanism surfaced that "everywhere" bundles two
genuinely different capabilities with very different risk profiles, and the
motivating case (a separate instance's window) is the lower-risk one:**

1. **Cross-instance / cross-window** — screenshotting or clicking into a
   *different AgentMux window entirely* (a separate `task dev` process, a
   separate portable build, another window on the desktop generally). This
   is architecturally closer to general OS window automation than to
   anything AgentMux's own agent-to-agent trust model governs — there is no
   shared `srv`, no shared auth key, no "another agent's pane" involved at
   all in the motivating case (an empty/idle dev window).
2. **Cross-pane, same instance** — one agent's `UIScreenshot`/`UIClick`
   reaching into a *different agent's pane within the same running
   instance*. This is the one that's actually security-sensitive, and —
   important finding below — it's not a gap nobody thought about. It's a
   **deliberately deferred, already-designed-for, previously-exploited**
   piece of this exact feature.

This doc treats them separately because the right next step for each is
different: (1) is close to shippable; (2) needs a real decision, not just an
implementation.

---

## 1. Cross-instance / cross-window targeting

### 1.1 What exists today

Nothing in AgentMux's own tool surface reaches a different instance/window —
confirmed directly: I could not drive the `task dev` window's UI from
production earlier this session (`mcp__agentmux__UI*` tools are scoped to
"your own pane and shared app chrome," full stop, and there's no
cross-instance variant).

### 1.2 Why this is the lower-risk half

- No shared authentication domain to worry about leaking across — a
  different instance is a different `srv` process with its own `X-AuthKey`,
  its own agent registry. Nothing in *this* instance's trust model is
  implicated.
- The underlying OS already exposes what's needed: any window on the desktop
  can be captured/interacted with via standard Win32 APIs
  (`PrintWindow`/`BitBlt` for screenshots, `SendMessage`/`PostMessage` or
  UI Automation for clicks), independent of AgentMux entirely. This isn't a
  gap in AgentMux's architecture so much as a missing convenience tool
  wrapping a capability that's already available to anything running on the
  machine (I already have `Bash`, and could drive this via a PowerShell
  script today, ad hoc — the gap is a *supported, repeatable tool*, not raw
  capability).
- Risk is closer to "can an agent see/interact with the desktop generally"
  (a machine-level capability question, same category as `Bash` already
  granted) than "can Agent A read Agent B's private conversation" (the
  cross-pane question below).

### 1.3 Recommended plan

Build a new tool — e.g. `CaptureWindow` — that:
- Takes a window title/handle (or a partial-title match) as its target,
  independent of any AgentMux-specific block/tab/window id.
- Implemented via the same OS-level mechanism I already improvised ad hoc
  this session (PowerShell + `user32.dll`), formalized into a real,
  repeatable tool rather than a one-off script each time.
- Does **not** need to touch `ui_handlers.rs`'s signed-identity scheme at
  all — it's a different code path entirely (no `srv` proxy, no CEF-host
  `TargetCache` resolution), which is exactly why it's lower-risk: it
  doesn't need to reason about AgentMux's own agent-to-agent trust boundary
  because it isn't crossing one.
- Scope question worth a deliberate answer before building, not left
  implicit: should this be screenshot-only (matching the motivating need),
  or also support clicking into an arbitrary window? Clicking into an
  arbitrary *other application's* window is a materially bigger capability
  grant than reading pixels from one — recommend screenshot-only for v1,
  revisit click separately if a real need for it shows up.

This can proceed without further security review beyond the above scope
question — it doesn't touch the mechanism §2 is about.

---

## 2. Cross-pane, same-instance targeting

### 2.1 The actual current mechanism (verified against live code)

`UIScreenshot`/`UIClick`/`UIQuery` do **not** derive "your own pane" from a
client-supplied block id at all (unlike `OpenEditor`/`FocusWindow`, which do
send `AGENTMUX_BLOCKID` verbatim). Instead:

1. The MCP tool (`agentmux-mcp/src/main.rs`) HMAC-signs a fixed identity
   tuple using `AGENTMUX_JEKT_KEY` (the same per-agent signing key jekt
   sender-verification uses) — `sign_ui_automation_auth()`, producing
   `UiAutomationAuth { agent_id, ts_secs, sig }`. This is the *only* payload
   sent; no block/pane id is transmitted by the client at all.
2. The server (`ui_handlers.rs`'s `verified_block_id`) verifies that
   signature against the claimed agent's own key on file, rejects anything
   stale (>300s) or forged, and **only then** looks up that agent's *actual
   current* block_id from a live, server-owned registry
   (`reactive::handler::get_global_handler()`) — never trusting anything the
   client claims about which pane it is.
3. That server-derived block_id (never client-supplied) is what actually
   gets proxied to the CEF host for the screenshot/click/query.

**The underlying CEF/CDP layer has no such restriction** — its
`TargetCache::resolve` already works against an arbitrary `block_id`, any
pane in the system. The "own pane only" boundary is entirely an
`agentmux-srv` *policy* decision layered on top of a capability that's
already technically unrestricted underneath.

### 2.2 This is not an oversight — it's deliberate, documented, and was
### already exploited once

`docs/specs/SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6
("Security model — the actual hard part") **already considered and rejected**
exactly the ask being made now. Quoting its own reasoning: a Click/Screenshot
tool that could target another pane "can read another pane's on-screen
content (credentials in a form, private conversation text) and act on
another agent's UI (e.g. clicking 'Confirm' on someone else's destructive
dialog) — not just steal focus." It explicitly defers cross-agent targeting
as "a distinct, explicitly higher-sensitivity capability" pending a
capability-flag gate that **does not exist in code today** (confirmed: zero
hits for any capability-flag mechanism anywhere in the repo).

This isn't theoretical caution — the same spec's own "Review findings
(2026-08-19, PR #2662)" section documents **two real P0 vulnerabilities**
found and fixed after initial implementation:
- `block_id` was originally accepted as a plain client-supplied field —
  meaning any agent could already read the shared instance-wide `X-AuthKey`
  from its own environment and simply claim a different pane's block_id.
  Fixed by replacing it with the signed-identity scheme in §2.1.
- The DOM-ownership check for the "shared chrome" exception (`query.js`)
  originally checked whether a matched element was *inside* a foreign pane
  but not whether it *contained* one — meaning `body`/`#root` was waved
  through as chrome and could see/click into every pane on screen. Fixed by
  checking descendants too.

There's a real regression test locking the fix in:
`ui_click_rejects_a_forged_agent_identity` (`agentmux-srv/src/server/tests.rs:715-749`)
— an agent signs correctly as itself but claims to *be* a different agent;
rejected.

### 2.3 The precedent to explicitly avoid

`FocusWindow` already supports targeting an arbitrary `window_id` — but by
sending the raw client-supplied id straight through with **zero ownership
verification**, unlike the signed-identity scheme above. The UI-automation
spec's own authors already named this as the anti-pattern *not* to repeat
for Click/Screenshot, verbatim: *"any agent can already focus … a different
agent's window … A Click/Screenshot tool inheriting this exact pattern would
be strictly worse."* **Do not model a widened design on `FocusWindow`'s
mechanism** — it's the one the original design explicitly rejected for this
exact capability, for documented reasons, not an oversight to copy.

### 2.4 What a real widening would actually require

Not a parameter addition. At minimum:
1. **A real authorization policy**, decided deliberately (a product/security
   call, not an implementation detail) — options include: always-allow
   within the same instance (closest to "everywhere," but reopens exactly
   the risk §2.2 already found and fixed once); an explicit per-agent
   opt-in/capability flag (the original spec's own deferred proposal);
   scoping to "panes in the same workspace as the caller" as a middle
   ground; or per-request human/target-agent confirmation for anything
   crossing the boundary (matching this repo's own jekt `ESCALATE=required`
   pattern for a comparable trust decision).
2. **Extending the signed-identity scheme, not bypassing it** — e.g. signing
   over a claimed *target* block_id too, so a forged-target attempt is
   cryptographically rejected the same way a forged-identity attempt already
   is, rather than reverting to `FocusWindow`'s trust-the-client shape.
3. **Screenshot needs separate treatment from Click/Query.** Even within the
   *already-shipped* "shared chrome" exception, Screenshot was deliberately
   left stricter than Click/Query (no chrome exception at all) because a
   screenshot's single rectangular clip can't exclude another pane sitting
   between the caller and a chrome element the way a DOM-ownership check
   can. Any widening proposal needs to solve this per-tool, not assume one
   mechanism covers all three.
4. **New tests in the existing idiom**, not invented from scratch — this
   area already has a real regression-test pattern to extend
   (`signed_ui_auth()` test helper, `agentmux-srv/src/server/tests.rs:646-845`),
   including a forged-identity attack test to mirror for whatever new
   forged-target attack a widened design introduces.

### 2.5 Recommendation

**Do not implement this as a quick widening.** Treat it as what the original
spec already correctly scoped it as: a distinct, higher-sensitivity
capability requiring its own explicit design pass and a real answer to
"what's the authorization policy" (§2.4.1) before any code changes — not
because it's impossible, but because this exact shortcut (skip the policy
question, ship the parameter) produced two real P0s the first time this
feature was built. The honest plan is: pick an authorization model
deliberately, write it up as its own spec (extending
`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` rather than
starting over), and implement against the existing signed-identity
foundation rather than around it.

---

## 3. Summary / next steps

| Capability | Risk | Status | Next step |
|---|---|---|---|
| Cross-instance/window capture (§1) | Low — no shared trust domain, OS-level capability already available via `Bash` | Ready to build | Implement `CaptureWindow` (screenshot-only for v1) as a new MCP tool, independent of `ui_handlers.rs` |
| Cross-pane, same-instance targeting (§2) | High — real, previously-exploited (PR #2662) agent-to-agent trust boundary | Needs a deliberate authorization-policy decision first | Write a follow-up spec extending `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6 once that policy question has an actual answer; do not implement ahead of it |

Immediate recommendation: proceed with §1 (`CaptureWindow`) now — it directly
serves the motivating need (the `task dev` window) with no open design
questions blocking it. Treat §2 as a separate, longer-lead item requiring
the operator's own product/security call on the authorization model before
any implementation starts.

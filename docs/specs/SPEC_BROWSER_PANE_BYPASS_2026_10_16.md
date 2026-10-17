# SPEC: Bypass: the person lets the agent act without asking in one browser pane

**Status:** active — B-1 built (§10): grant from the agent's request and from the button, the srv gates, the red strip, `end_bypass`. Decisions on the open questions are in §11.
**Date:** 2026-10-16
**Author:** Naki #2 (naki-naki-2), at the repo owner's request
**Trigger (repo owner, 2026-10-16):** a bulk job in an agent-driven browser pane needed about 75 delete clicks on one site, and each one waited for the person to approve it in the pane. "Do we need to do each manually 70 times? … write a spec that gives an agent superaccess for anything in the currently running pane. Make it a red 'bypass' button next to Take over … since we cannot sit and wait here for each of these."
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (ownership §3, approvals §5.4, Take over §5.5, host-only answers §5.6), `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md` (site lists). It replaces that spec's unbuilt "don't ask again on this site for this pane" override (§5.4, last bullet).
**Related:** a hand-off and a site-list question share a pane's one attention slot, which leaves a pane blank when a sign-in redirect leaves its sites during a hand-off. That fix is separate and independent of this one.

---

## 1. Summary

- **Two ways to get the person's OK, both one click:**
  - The agent asks for it with `Browser` action `request_bypass` ("I need to delete 75 old drafts on github.com"). The pane shows the request, and the person answers OK once.
  - The person turns it on unasked with the red **Bypass** button next to **Take over** in the header of a pane an agent is driving.

  Either way the person sees what it covers and for how long, and confirms.
- While it is on, the agent's committing clicks in that pane (Submit, Delete, Send, …, through `Browser` `click`) are made **without asking**. Everything else stays as it is: `eval`, UI click by selector and Enter/Space stay refused on a driven pane, because only `click` goes through the checks that keep the promises below (§4.1).
- It is **bound** to this pane, this agent, the site(s) the pane was on when it was turned on, and a time limit. It ends on **End bypass**, the time limit, Take over, closing the pane, or the agent stopping. Leaving the bound sites pauses it.
- **Only the person can turn it on**, through the host, the same way approvals are answered. No agent route, shortcut, command or synthesized click can turn it on.
- **Not bypassed:** the agent still never types into password, one-time-code or card fields, and hand-off works as before. Bypass removes waiting for approval; it doesn't give the agent secrets.
- The pane shows it plainly the whole time: a red "Bypass on" strip with who, where and how long, and every action taken under it in the action log.

## 2. Why

Approvals (§5.4 of the 2026-10-07 spec) stop a page from talking an agent into committing something. They are right for one-off actions and wrong for a bulk job the person has already decided on. 75 approvals in a row turns the person into a click relay. That is no safer, because nobody reads the 40th identical banner, and it is slow. The person needs a way to say "for the next hour, on this site, the agent may act without asking", with the risk shown plainly.

## 3. The button and the states

### 3.1 Off (default)

The header strip of a driven pane reads `Driven by <agent> [Limited to N sites] [Take over]`. Bypass adds:

- **[Bypass]**, red (`danger` tone), to the right of Take over. Tooltip: "Let <agent> act in this pane without asking you first."

### 3.2 Turning it on

Clicking **Bypass** opens a confirmation panel directly under the header strip, in the pane's own flow. It is not a floating popover: the page is a native window drawn over anything layered on it, and the panel must stay inside the pane's DOM (§5). The page moves down while the panel is open:

> **Let Naki #2 act without asking?**
> In this pane, on **github.com**, Naki #2's clicks that submit forms, delete or send go through **without asking you first**. It still can't type passwords or codes or run scripts, and payments still ask.
> Duration: ( 15 min | **1 hour** | 4 hours )
> **[Turn on bypass]** [Cancel]

- The sites listed are the bound sites (§4.2), computed when the panel opens.
- The default duration is 1 hour. "Until I end it" still ends on Take over, close and agent stop.
- **Turn on bypass** is the only control that grants it, and only on a pointer click: a key press on the focused button (a click with `detail` 0) does nothing.

### 3.3 On

The header strip turns red: `Bypass on: Naki #2 acts without asking · github.com · 47 min left [End bypass] [Take over]`. Also:

- *(B-2)* The pane's tab shows a small red badge, so the state is visible when the pane isn't focused.
- *(B-2)* A notification fires when bypass turns on and when it ends ("Bypass ended for Naki #2: time limit"), so a person who walked away knows.
- *(B-3)* The action log (§5.5 of the 2026-10-07 spec, not built yet) marks each action taken under bypass. Until then srv logs each one at `info` (`[browser-bypass] committing click made without asking`, with the agent, pane, button and site): `⚡ clicked "Delete draft" on github.com/…`.

### 3.4 Paused

When the page goes to a site outside the bound sites (§4.2), the strip turns amber: `Bypass paused: the page left github.com [Extend to this site] [End bypass]`. Approvals apply again until the page is back on a bound site or the person extends it. "Extend to this site" uses the same confirmation as §3.2, listing the added site.

### 3.5 Ending

Bypass ends on any of: End bypass, the time limit, Take over, closing the pane, the owner changing, and srv restarting (a grant is never restored from disk). Ending is immediate. A committing action already sent to the page is not recalled, and the next one asks again.

## 4. What a grant covers

### 4.1 Lifted while on

| Today on a driven pane | Under bypass |
|---|---|
| A committing click returns `needs_approval` and waits for Approve (srv `ui_handlers.rs`, approval branch of `act`) | Made at once. The host's fingerprint check still runs, so the click sent is exactly the one classified. The result says `bypassed: true` |
| `Browser` `eval` refused ("could submit a form without the user's approval") | Still refused |
| `UI` `click` by selector refused | Still refused |
| `Browser` `key` with Enter or Space refused | Still refused |
| `set_files`: workspace files only | Unchanged (workspace only): an exfiltration guard, not a waiting one |

**Why only `click`:** the promises the person reads ("payments still ask", "it can't type passwords or codes") are enforced on `act`: the host classifies each click, srv applies the payment check, and `fill` refuses secret fields. `eval`, a click by selector and Enter/Space bypass all of that. A script can press any button or read any field, and Enter submits whatever form has focus. Lifting them would make the confirmation's text false (muxreview on #5045). Every bulk job in view is a run of clicks, so this costs nothing in practice.

### 4.2 Bound

- **Pane:** the pane it was granted on, and the popup windows and popup panes it opens (its chain). Not other panes of the same agent.
- **Agent:** the pane's owner at grant time. If ownership changes, the grant ends.
- **Sites:** the origins the person saw in the confirmation: the pane's current origin, plus its site list if the pane has one. Each bypassed action checks the origin of the page it acts on, as the host reports it at action time, not as the agent claims it. If it's off the bound sites, the action takes the normal path (approval or refusal) and the strip shows Paused.
- **Time:** the chosen duration, checked by srv on every action, plus a timer that takes the strip down.

**Why bind to sites:** the threat approvals guard against is a page instructing the agent. A grant bound to the site the person chose limits a hostile page elsewhere (a redirect, an ad, a link the agent follows) to the normal approval path. Without the binding, one click would let any page the agent reaches commit anything.

### 4.3 Never lifted

- The secret-field guard (§5.1): no typing into password, one-time-code or card fields, and no reading their values.
- Hand-off (§5.2): while the pane is handed to the person, the agent's tools are refused, bypass or not.
- Site-list questions (allowed-origins spec §4): a navigation off the pane's list is still asked about. Bypass doesn't widen the list.
- Ownership: bypass doesn't let an agent act on a pane it doesn't own.
- Identity: bypass doesn't lend a session (§6 of the 2026-10-07 spec).

## 5. Who can turn it on

The answer to "can an agent approve itself?" must stay no. So:

- **Route:** `POST /api/v1/host/browser_bypass` with `X-Host-Token` only, the same as `/api/v1/host/browser_attention`. srv returns 403 to any other caller, including any agent holding the auth key. Body: `{pane, on: true|false, minutes?: 15|60|null, origins}`.
- **Path from the button:** the confirmation's Turn on button calls the host over host IPC (`browser_bypass_set`), whose token lives only in the front end and the host. The host posts to srv.
- **srv re-checks before granting:** the pane exists, is driven, the owner is unchanged since the confirmation opened (it sends the owner it showed), the origins match the pane's current page, and no hand-off is pending.
- **Turning it off** goes through the same route, but srv also accepts it from the agent itself: an agent may end its own bypass (`Browser` action `end_bypass`) and never start one.
- **Not reachable by an agent's tools, and tested** (§9):
  - `UI` `click`/`query` reach only the caller's own pane and app chrome outside every pane (`scripts/query.js`, `__amq_allowed_for`). On a browser pane they act on its page, not the app document. The Bypass button, the confirmation and the request banner all render inside the browser pane's own `[data-blockid]` subtree, so no agent's selector reaches them. A CDP click is a trusted event, so `event.isTrusted` would not tell them apart: the scoping is the guard, and it must stay that way. Never portal these controls to the document root.
  - No shortcut and no command palette entry, so `UI` `command` and `keys` can't reach it.
  - `browser:bypass` is a **srv-only meta key** (like `browser:owner_agent` and `browser:attention`). Clients may not write it, and it is stripped from new, restored and layout-opened panes.

## 6. Agent-facing behaviour

- A committing click made under bypass returns `bypassed: true` and the grant's remaining minutes (the `Browser` result reads "done without asking"), so the agent knows its actions are no longer being checked by a person and can be more careful.
- The `Browser` tool description adds one line: "If the person turned on Bypass for your pane, committing actions run without asking: double-check before every one, and end it (`end_bypass`) when the job is done."
- **Asking for it:** `Browser` action `request_bypass({pane, reason, minutes?})`. This is the main path: an agent that knows it faces a run of approvals asks once, up front.
  - The pane shows a red banner (a new attention kind, `bypass`): "**Naki #2** asks to act without asking you first: *delete 75 old drafts*. On **github.com**, for **1 hour**. It still can't type passwords or codes. [Allow without asking] [No]". The duration is a chooser defaulting to what the agent asked for, capped at the maximum (§11.1).
  - The answer goes through the host exactly as in §5. Allow grants the same bound grant as the button; No leaves approvals in place.
  - The call waits like `handoff` (default 10 min) and returns `granted` with `expires_at`, or `declined`, or `timed_out`. On `declined` the agent carries on with per-action approvals or asks in chat; it never asks again in a loop (at most once per pane per 10 minutes, enforced by srv).
  - Refused while the pane is handed to the person or another question is up, with a reason, so the agent retries after.
- **Using it:** after `granted`, the agent works through the job. Each committing action runs at once and returns `bypassed: true`. If a result comes back `needs_approval` (the grant ended, or the page left the bound sites), the agent stops and reports rather than retrying blindly.

## 7. Where it lives

| Layer | Change |
|---|---|
| srv | New `server/browser_bypass.rs`: the grant table (in memory only), `grant`/`end`/`active_for(pane, agent, origin)`, the expiry timer, and ending on Take over, close, owner change and agent stop (hooks next to `browser_owner` and `browser_attention::cancel_for`). The host route. The approval branch of `act` in `ui_handlers.rs` checks `active_for` and, if active, proxies the approved act directly. srv-only meta key `browser:bypass` `{agent, origins, expires_at}` for the UI. Logging: `info` on grant, end (with reason), and every bypassed action (agent, pane, origin, action name). |
| host (cef) | IPC `browser_bypass_set` → srv host route; `act` returns the page origin it acted on (it already reads it for the fingerprint); trusted-event check if §5 requires it. |
| frontend | Bypass button, confirmation panel, red/amber strip with countdown, End bypass, tab badge; `browser:bypass` read in `browser-model.ts`; the action-log marks. Reuse the attention banner's components. |
| mcp | `end_bypass` (phase 1), `request_bypass` (phase 2); `bypassed` in results; the description line. |

## 8. Security

- **What bypass gives up, said plainly:** while on, a page on a bound site that tricks the agent can make it commit things the person didn't intend. The person accepts that for a site and a time they chose. Binding to sites and time (§4.2), the visible red strip (§3.3) and the action log are what keep the trade-off reasonable.
- **What it doesn't give up:** secrets (§4.3), ownership, the site list, and the rule that only the person grants (§5).
- **Same OS user:** as elsewhere in MuxAPI, a process running as the user can drive CEF directly. Bypass doesn't change that residual.

## 9. Tests

- srv: `request_bypass` shows the `bypass` banner and grants only on the person's Allow through the host. No, a timeout and Take over return without a grant. A second request within 10 minutes of a No is refused. A request during a hand-off is refused with a reason.
- srv: a grant from a non-host caller is 403. A grant is refused when the owner changed since the confirmation, a hand-off is pending, or the origins don't match the current page.
- srv: under an active grant, a committing `act` on a bound origin runs without a banner and returns `bypassed: true`. On an unbound origin it asks as before.
- srv: the grant ends on time-out, Take over, pane close, owner change and agent stop, each logged with its reason. After any of these the next committing act asks again.
- srv: `eval`, UI click and Enter/Space stay refused under a grant. Secret fields are refused under a grant.
- srv: `browser:bypass` can't be written by a client meta update, and is stripped on restore.
- frontend: the button shows only on driven panes. The confirmation lists the bound sites and durations. The strip counts down and turns amber when paused. A key press on Turn on bypass doesn't grant.
- Agent reach: `UI` `click` with a selector for the Bypass button, from the owning agent and from another agent, doesn't grant. No command or shortcut grants.
- e2e (isolated dev build): open a pane on a test site with several delete buttons, turn on bypass for 15 minutes, and have the agent click all of them with no banner. Navigate off the site: it pauses. End bypass: the next delete asks.

## 10. Phases

1. **B-1:** grant and end, from both the agent's request and the button: the `request_bypass` banner, the Bypass button and confirmation, the red strip, the host route, the srv gate for committing clicks, binding, expiry, ending hooks, logging, `end_bypass`. This is what the bulk job needed.
2. **B-2:** tab badge and notifications, if they didn't make B-1.
3. **B-3:** the action log, if §5.5 of the 2026-10-07 spec still hasn't built it.

## 11. Decisions (repo owner: "use your best recommendations", 2026-10-16)

1. **Duration:** 15 minutes, 1 hour (default) or 4 hours. There is no "until I end it": 4 hours is the maximum, and an agent's `minutes` rounds down to one of the three.
2. **Payment:** clicks named like Pay, Buy, Purchase, Order, Checkout, Check out, Place order, Transfer, Donate or Subscribe still ask under bypass (`browser_bypass::is_payment`). The cost of a mistake is money, and bulk jobs rarely pay.
3. **Scope across panes:** one grant per pane and its popup chain.
4. **The agent's session ending** doesn't end a grant. Ownership survives the agent restarting (2026-10-07 spec §3), and the time limit bounds the grant anyway.

Still open:

1. **"All prompts" and the site list:** proposed, bypass doesn't answer site-list questions, and leaving the bound sites pauses it (§3.4, §4.3). The alternative is an "also allow other sites" box in the confirmation, off by default. It would make a long job fully uninterrupted, but a page could then send the agent anywhere and act there without asking.
2. **Remote and cloud agents:** grants are local to the srv the pane is on. Nothing here needs WAN, but an agent driving a pane through another instance should be checked.

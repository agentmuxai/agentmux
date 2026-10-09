# SPEC: Popups from a browser pane stay in AgentMux, and an agent can drive them

**Status:** active — P1 implemented (agentmux#4478): popups open as panes beside the opener (§3.2 Option B), ownership is inherited, snapshots list popups. P2 and P3 open.
**Author:** AgentX@narko, 2026-10-08
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (ownership, `BrowserHandoff`),
`SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md` (the popup-redirect rule),
`SPEC_BROWSER_PANE_IDENTITIES_2026_09_22.md` (popups inherit the opener's profile).

## 1. Problem

A page in a browser pane that calls `window.open` (or a form with `target=_blank` that the host
treats as a popup) goes one of two ways today (`crates/cef/src/client/lifecycle.rs`,
`on_before_popup`):

- **A known sign-in provider's authorization URL** (`is_known_idp_host` and
  `is_oauth_authorization_url`, `crates/cef/src/commands/platform.rs`): a real CEF child popup,
  which shares the opener's cookies and keeps `window.opener` and `postMessage` working.
- **Anything else with an http(s) URL:** the system browser (`open_url_in_default_browser`). The
  popup leaves AgentMux, takes none of the pane's session with it, and cannot return a result to
  the page that opened it.

The second path broke a real flow on 2026-10-08: an agent drove a cloud provider's console in a
pane it had opened, the human signed in, and a step opened a popup. It landed in the person's
system browser. The agent could not see it, the pane never learned the outcome, and the person
had to finish the step by hand in a different browser profile.

Two separate gaps:

1. **Wrong place.** The popup is outside AgentMux, outside the pane's profile.
2. **Not drivable.** Even the allowlisted OAuth popup has no block id and no `browser_panes`
   entry; `TargetCache::resolve` (`crates/cef/src/browser_api/resolver.rs`) deliberately never
   returns it. An agent that opened the opener cannot snapshot, click, or hand off in it.

## 2. Goals

- A popup opened by a user-visible action in a browser pane appears inside AgentMux, in the
  opener's profile, where the human can see and use it.
- If the opener is an agent-owned pane (`browser:owner_agent`), the popup is too, with the same
  Take over and Pause controls, and every `Browser*` tool accepts it as `pane`.
- `window.opener` and `postMessage` keep working where a flow needs them.
- Nothing becomes easier for a hostile page: a popup the user did not cause still does not
  appear.

Non-goals: tabs inside a pane; driving a pane the human opened (still B5 of the 2026-10-07
spec); popups from non-browser panes.

## 3. Behavior

### 3.1 When a popup is admitted

`on_before_popup` already receives the user-gesture flag. A popup from a browser pane is admitted
as an AgentMux popup (instead of the system browser) when **all** hold:

1. The request came from a user gesture, or from an agent-driven action in the opener (a
   `BrowserClick` is a gesture; page script alone is not).
2. The URL is http(s). `file:`, `javascript:` and `data:` stay refused.
3. The per-pane cap is not reached (today 8 pending popups; keep it).
4. Either the host is on the existing sign-in allowlist (unchanged behavior), **or** the target's
   registrable domain matches the opener's, or the opener is an agent-owned pane (§3.4).

Anything else keeps today's behavior: the system browser for an external URL, silence for an
internal one. The allowlist stays the boundary against a page that spawns popups for phishing;
this spec widens it only to popups the person or their agent caused.

### 3.2 Where it appears

Two options, in order of preference:

| | Option | Opener link | Cost |
|---|---|---|---|
| **A** | **A floating pane** (the existing floater kind, `crates/cef/src/floating_pane.rs`), hosting the CEF child popup | kept | the Chrome runtime owns the popup window and ignores `on_popup_browser_view_created` sizing (`app/mod.rs`); reparenting it into a floater is the unknown. Needs a spike. |
| **B** | **A sibling pane in the same window**, the URL loaded as a normal navigation | lost | no `window.opener`, so a flow that posts its result back to the opener (some sign-in and payment widgets) does not complete. Redirect-based flows do. |

Recommendation: ship **B** first (P1) because it needs no new window machinery and covers
redirect flows, which is most of what a console or sign-in does; spike **A** (P2) for
opener-dependent flows. Each popup shows its address in the pane's own address bar and a
"Popup from <opener's site>" strip (`browser:popup_from`, the opener's origin), so the person
can always tell where they are and which site opened it (§5). The site, not the opener's
title: srv knows the opener's URL, not its title, and a title is the page's to choose.

### 3.3 Closing, focus and the opener

- A popup pane is an ordinary browser pane once open: the person closes it like any other.
  `window.close()` from its page does nothing, because Chromium honors it only for a window a
  script opened, and this pane was opened by AgentMux (one of Option A's gains, P2).
- Closing the opener leaves its popups open (P1). Closing them with it is P3, if it proves
  wanted: a popup the person is still using shouldn't vanish because they tidied the opener.
- The native OAuth popup (Option A's precursor) is unchanged, including its quit-gate exclusion.

### 3.4 Ownership and driving

- A popup gets a block id, a `browser_panes` entry in `Live` state, and `browser:popup_of =
  <opener block id>`.
- **Ownership is inherited, server-side:** srv writes `browser:owner_agent` on the popup block
  from the opener's value, never from the page or a client. A popup of a pane nobody owns is
  nobody's.
- **Discovery:** the next `BrowserSnapshot` of the opener lists its open popups, each with
  its pane id, address, and whether the agent may drive it (`popups: [{pane, url, yours}]`
  from srv; the MCP text lists them above the page content). P3 adds `popup` as a
  `BrowserWaitFor` condition.
- **Where it's decided:** srv, through the host-only route `/api/v1/host/browser_popup`
  (the host's IPC token, as for `browser_attention`): only srv knows whether an agent owns the
  opener. The host asks from its own thread and opens the system browser on a refusal or an
  error, so a slow or absent srv only ever means today's behavior.
- Every `Browser*` and `UI*` tool takes the popup's id as `pane`, subject to the same owner check
  as any owned pane, **and driven as part of its opener**: while the opener exists, the agent
  must still own it (Take over on the opener ends the agent's hold on its popups too), and a
  hand-off or approval waiting on the opener pauses its popups. A popup whose opener closed stands
  on its own.
- **Races:** the cap is checked and a slot reserved under one lock, and given back if the pane
  doesn't open, so popups reported at once can't overshoot it; the opener's owner is checked again
  once the popup pane is open, so a Take over during the opening leaves the popup unowned. `BrowserHandoff`, Take over and Pause work on it, and a handoff on the
  opener pauses its popups.
- Unchanged: an agent never types into a `[secret]` field; a sign-in in a popup is a hand-off
  like any other.

## 4. Profile and session

A popup uses the opener's request context. On Windows panes use the shared default context
(`browser_pane/creation.rs`); on the Linux and macOS Views path the parent window's context
(`creation_views.rs`). `SPEC_BROWSER_PANE_IDENTITIES` flags `creation_views.rs:126-130` as a
place a popup might get a different one; P1 adds a test that a popup reads the opener's
cookies on every platform, and fixes that spot if it does not.

## 5. Security

1. **Gesture and domain gates (§3.1) are the only widening** of what can open. A page with no
   user action opens nothing.
2. **The URL is always visible** in the popup's header, and the title says which pane opened it.
   A popup must not be able to hide its address.
3. **Ownership is server-side**, written from the opener by srv, as in the 2026-10-07 spec.
4. **A cap per pane** (8) and a cap per window, with an overflow popup going to the system
   browser as today.
5. **No new route to the human's panes:** an agent still drives only what its identity owns, and
   a popup is owned only if its opener was.

## 6. Where it lives

| Part | Change |
|---|---|
| `crates/cef/src/client/lifecycle.rs`, `handlers.rs` | `on_before_popup` takes the user-gesture flag; a non-OAuth popup from a browser pane goes to srv from a background thread (`offer_popup_as_pane`), with the system browser as the fallback |
| `crates/cef/src/client/helpers.rs` | `backend_browser_popup` over a shared `post_as_host` (also used by `backend_browser_attention`) |
| `crates/srv/src/server/browser_popup.rs` | the §3.1 rule (`decide`, `same_site` on the public suffix list via the `psl` crate), the cap, the opener-to-popups record |
| `crates/srv/src/server/ui_handlers.rs`, `routes.rs` | `POST /api/v1/host/browser_popup`; the popup opens through `open_pane` beside the opener with `browser:popup_of`, `browser:popup_from`, and an inherited `browser:owner_agent`; snapshots gain `popups` |
| `crates/srv/src/server/browser_owner.rs` | clients may not write `browser:popup_of` or `browser:popup_from` |
| `agentmux-mcp` | the snapshot lists popups; `BrowserSnapshot`'s description says how |
| frontend | the "Popup from <site>" strip (`popupFromAtom`); the two meta keys in `MetaType` |

No new pane machinery: a popup is an ordinary browser pane, so the resolver, profile, and
lifecycle code are unchanged.

## 7. Tests

Automated (P1):
- `browser_popup` unit tests: the gesture gate, the site gate on the public suffix list
  (`a.github.io` and `b.github.io` are different sites; IP addresses and `localhost` match only
  themselves), the owned-opener widening, the cap, web URLs only, the opener-to-popups record.
- `a_popup_opens_as_a_pane_beside_its_opener_and_inherits_its_owner` (srv route test): only the
  host token can report a popup; an owned pane's popup to another site opens as a pane the same
  agent owns and can drive, with `browser:popup_of` and `browser:popup_from`, and is listed for
  the opener's snapshot; no gesture, no pane; a person's pane opens a same-site popup unowned
  (the agent can't drive it) and sends another site to the system browser; a non-browser pane
  is refused; the cap holds and closing a popup frees a slot.
- `browser_owner`: a client can't write either popup key, set or clear.
- MCP: the snapshot's popup lines; frontend: `popupFromAtom`.

By hand, in a `task dev` build: a clicked `window.open` from a pane opens a pane beside it,
signed in with the opener's cookies, with the strip; a timer-opened one goes to the system
browser; an agent-owned pane's popup appears in its snapshot and takes `BrowserClick`.

## 8. Phases

| | Scope |
|---|---|
| **P1** (done) | Option B: admit under §3.1, open as a sibling pane in the opener's profile, inherited ownership, `popups` in the snapshot, the "Popup from <site>" strip |
| **P2** | Spike Option A (a real child popup in a floater) for flows that need `window.opener`; decide from the result |
| **P3** | `popup` wait condition; closing a popup with its opener, if wanted; a setting to prefer the system browser for people who want the old behavior |

## 9. Open questions

1. **Does P1 cover enough?** Which real flows post a result back through `window.opener`
   (payment and some single-sign-on widgets do)? A list from the operator decides how soon P2
   matters.
2. **Should a popup from an unowned pane to a different domain ever be admitted?** This spec says
   no (system browser), to keep the phishing boundary; the cost is that a human-driven pane's
   cross-site sign-in still leaves AgentMux.
3. **Windows Chrome runtime:** can the popup window it owns be hosted in or snapped to a
   floater, or must P2 use its own window?
4. **A per-pane opt-out** (the 2026-10-07 spec's `allowed_origins`): should an allowlist that
   would refuse a navigation also refuse a popup to the same place? Proposed: yes.
5. **Popups to `localhost` and `127.0.0.1` are dropped without a word.** `on_before_popup`
   treats those hosts as internal (`is_external_http_url`) and cancels a pane's popup to them,
   neither as a pane nor in the system browser: unchanged here, and it keeps a page from popping
   the app's own frontend in a pane. Found while testing P1 against a local test page (served on
   `127.0.0.2` instead). A local dev site that opens a popup gets nothing; should a popup to
   loopback whose opener is on that same loopback origin be admitted?

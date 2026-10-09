# SPEC: A browser pane's new windows open the way the page asked, and the agent that owns the opener drives them

**Status:** active — N1 implemented (stacked on agentmux#4478); N2 and N3 open.
**Author:** AgentX@narko, 2026-10-08
**Builds on:** `SPEC_BROWSER_PANE_POPUPS_ADOPTED_2026_10_08.md` (P1, agentmux#4478: popups as
panes beside the opener), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (ownership, hand-off,
approvals), `SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md` (why popups are gated),
`REPORT_BROWSER_PANE_GOOGLE_LOGIN_INSTANCE_EXIT_AND_UAC_2026_08_11.md` (the native OAuth popup).

## 1. Summary

1. **The page that opens a window chooses what it gets**, through what it asked for:
   - `window.open` with popup features (a size, `popup`) is a request for a **bare popup
     window**: it opens as a real CEF child popup, no address bar, the way "Sign in with Google"
     already does. `window.opener`, `postMessage` and `window.close()` work.
   - A link with `target=_blank`, `window.open` without features, or a request for a new window
     is a request for a **full browsing context**: it opens as a **browser pane beside the opener,
     with its address bar** (P1's mechanism).
2. **Which requests are honored is P1's rule** (a click; the opener's own site, a known sign-in
   provider, or an agent-owned opener; at most 8 per opener). Anything else goes to the system
   browser, as today.
3. **The agent that owns the opener drives both kinds**, with the same `Browser*` and `UI*` tools,
   by ids its snapshot lists.
4. **The person stays in charge**: a bare popup's hand-offs and approvals appear on the opener's
   banner, Take over on the opener ends the agent's hold on everything it opened, and the opener
   lists its open popup windows with Show and Close.

## 2. What the page asks for, and what it gets

CEF reports the request as `on_before_popup`'s `target_disposition`:

| The page did | Disposition | Today (`main`) | P1 (#4478) | This spec |
|---|---|---|---|---|
| `window.open(url, name, "width=…")` / `"popup"` | `NEW_POPUP` | known sign-in provider: native popup; else system browser | pane | **native bare popup** |
| `window.open(url)`, `target=_blank`, middle-click | `NEW_FOREGROUND_TAB` / `NEW_BACKGROUND_TAB` | loads in the opener's own frame | unchanged | **pane beside the opener** |
| Shift-click, `window.open` asking for a new window | `NEW_WINDOW` | system browser | pane | **pane beside the opener** |
| Not honored under §3 | any | system browser | system browser | system browser |

The tab row is the one behavior change outside popups: a `target=_blank` link used to replace the
opener's own page, which lost the page the person was on; a browser opens a tab, and a pane
beside the opener is AgentMux's tab. A link without `target` still navigates in place.

Observed on 2026-10-08 (Windows, CEF Chrome runtime, a `task dev` build): the native popup is a
`CefBrowserWindow`, sized as the page asked (500×600 requested, 518×647 with its frame), placed
in front of the AgentMux window, **without an address bar**. Its window title is the page's
title, or its address when the page has none: the Google error page showed
`accounts.google.com/signin/oauth/…`, while a test page titled "Popup page" showed just that. So
the title bar is no proof of where the window is; the opener's strip is (§7, §8.4).

## 3. Which requests are honored

P1's rule (`browser_popup::decide`), for both kinds:

- a user gesture (a click or key press; an agent's CDP click counts, it is trusted input);
- an http(s) target;
- fewer than 8 of its kind open from this opener (popup windows, counted by the host; popup
  panes, counted by srv, which reserves the slot before opening so concurrent requests can't
  overshoot);
- and one of: the target is on the opener's site (registrable domain, public suffix list); it is
  a known sign-in provider's authorization URL (today's native case); or an agent owns the opener.

Two cases P1 left open, now decided:

- **A person's pane, another site:** not honored; the system browser, as today. A bare window
  from an arbitrary site with no address bar is the spoofing case the gate exists for; sign-in
  providers are already covered by the allowlist. (Decided, was P1 §9 Q2 and native-spec Q3.)
- **Loopback** (`localhost`, `127.0.0.1`): honored when the target has the **same origin** as
  the opener (scheme, host and port) and that origin is not one of AgentMux's own (the frontend's
  dev origin, srv's endpoint). Today these are cancelled silently. (Decided, was P1 §9 Q5.)

The decision is made **in the host**, inside `on_before_popup`, because creating a native popup
is that callback returning `false`: it cannot wait on srv. So:

- the rule moves to `agentmux-common` (`popup_rules`), used by host and srv;
- srv pushes the set of agent-owned browser-pane block ids to the host whenever it changes
  (OpenBrowser, Take over, close, a failed lookup) and when the host registers, on a host route
  (`/agentmux/browser/owned_panes`, host IPC token). It only widens which clicked requests are
  honored; who may *drive* a window is srv's own record (§5), so a stale set can at worst let one
  click open in-app instead of in the system browser;
- the host counts each opener's open popups and popup panes.

A pane request honored by the host goes to srv as in P1 (`/api/v1/host/browser_popup`, which then
opens the pane); srv re-checks ownership there with its own record. If srv declines (its record
disagrees with the host's hint, or the pane cap), the request goes to the system browser, links
included: the host asks srv from its own thread and can no longer load the link in the opener's
frame. A native popup is created at once and registered after (§4). The routing is one pure
function (`crates/cef/src/client/popup_route.rs`) with the table above as its tests.

## 4. Identity and lifecycle of a bare popup

- **Id:** the host's label for it, `popup-<uuid>` (it already exists, is unique, and is what the
  CDP session attaches by). Never a block id.
- **Created:** `on_before_popup` queues `(opener block id, url)` and returns `false`;
  `on_after_created` takes the entry, tags and registers the browser as today, records
  `popup -> opener` in host state, and reports it: `POST /api/v1/host/browser_popup_window
  {event: "opened", popup, opener, url}` (host token, background thread).
- **Navigated** (N2): the same route with `event: "navigated"` keeps the address current; srv
  accepts it today, the host doesn't send it yet.
- **Closed** (`window.close()`, the person, or its pane closing, whose force-close of its popups
  goes through the same path): `event: "closed"`.
- **Popups of popups** count against the root pane: the opener of a popup's popup is the popup's
  own opener (host records), so the cap and ownership hold across a sign-in that opens a second
  window.
- **srv restart:** records are in memory, like ownership; popups are no longer listed or drivable
  until reopened.

A popup pane (the tab kind) is a block, exactly as in P1: `browser:popup_of`,
`browser:popup_from`, an inherited `browser:owner_agent`, the "Popup from <site>" strip.

## 5. Ownership and addressing (srv)

- `browser_popup` keeps `popup id -> {opener, url, owner}`; `owner` comes from `browser_owner`
  for the opener at registration, never from the message.
- **`pane` accepts a popup id.** `target_block_id` gains a popup branch: allowed only if the
  popup's recorded owner is the caller **and** the caller still owns the opener
  (`browser_owner::check` on the opener). Take over on the opener therefore ends the agent's hold
  on its popups. A popup pane is a block with its own owner, as in P1.
- **The opener's snapshot lists both kinds**: `popups: [{pane, url, yours, kind: "window"|"pane"}]`;
  the MCP text says which is which.

## 6. Driving a bare popup (host)

- **Resolver Path 0:** an id naming a live `BrowserKind::Popup` browser resolves to it with
  `scope_to_block: false`, its page being its own. Every route already resolves through
  `open_cdp_for_block`, and reference tables are keyed by the id, so snapshot, click, fill,
  select, check, set-files, wait, navigate, back/forward/reload, screenshot and `UIQuery` all work
  unchanged.
- **CDP** is the in-process DevTools channel (`browser_api/cdp.rs`), which works on any `Browser`;
  `cdp::on_browser_closed` already runs for popups.

## 7. The person in the loop

A bare popup has no AgentMux header; what would appear there appears on the **opener pane**:

- **Banners:** `BrowserHandoff` and a committing click's approval on a popup show on the opener's
  banner, naming the popup window and its address. One request at a time per opener, popups
  included (the per-block lock is taken on the opener).
- **The popup-windows strip:** while an opener has bare popups open, its header lists each one's
  address with **Show** (bring it to the front) and **Close**, from a srv-written meta key on the
  opener (`browser:popup_windows`).
- **Take over** on the opener ends the agent's hold on it, its popups and its popup panes.
- **Secrets:** an agent never fills a `[secret]` field; a sign-in in a popup is a hand-off, done
  by the person in the popup window.

## 8. Security

1. **What is honored is no wider than P1**, plus same-origin loopback (§3).
2. **Driving is srv's decision**, from its own records; the host's owned-pane set only widens
   which clicked requests open in-app.
3. **Popup registration uses the host IPC token**, which agents never see.
4. **The address is always visible**: a popup pane has its address bar; a bare popup's address
   is in the opener's strip. Its own title bar shows whatever title the page chose (§2), so a page
   can name its window anything; N2 puts the origin in front of it.
5. **Profile:** both kinds use the opener's request context
   (`SPEC_BROWSER_PANE_IDENTITIES` §G6 must hold for private-identity panes).
6. **The pane's own allowlist** (`OpenBrowser({allowed_origins})`, 2026-10-07 spec §8) applies
   to its popups and popup panes: a target it would refuse to navigate to is refused here too.
   (Decided, was P1 §9 Q4.)

## 9. Where it lives

| Part | Change |
|---|---|
| `agentmux-common` | `popup_rules` (from srv's `browser_popup`): `decide`, `same_site`, the loopback same-origin rule; the `psl` crate moves here |
| `crates/cef/src/client/lifecycle.rs`, `handlers.rs` | disposition → window or pane; admission in the host; the `(opener, url)` queue; register, navigate and close messages; popups of popups to the root opener; tab dispositions no longer load in the opener's frame |
| `crates/cef/src/state/` | the owned-pane set; `popup -> opener`; per-opener counts; `live_popup_label` |
| `crates/cef/src/browser_api/` | resolver Path 0; the `owned_panes` route; show and close a popup (for the strip) |
| `crates/srv/src/server/browser_popup.rs` | popup-window records and owners; the opener's strip value (`browser:popup_windows`) |
| `crates/srv/src/server/ui_handlers.rs`, `routes.rs`, `main.rs` | `POST /api/v1/host/browser_popup_window`; `target_block_id` popup branch; banners on the opener for a popup target (`banner_for`); `kind` in listings; the owned-pane push task (`spawn_owned_panes_sync`, woken by `browser_owner::changed`, every minute regardless, and on host registration) |
| `crates/srv/src/server/browser_owner.rs` | `owned_panes`, `changed`; `browser:popup_windows` is srv-only (updates refused, stripped from new blocks) |
| `crates/cef/src/client/popup_route.rs` | the routing table, pure and tested |
| `crates/cef/src/ui_tasks/popup.rs`, `ipc.rs` | Show and Close (`browser_popup_show` / `browser_popup_close`) |
| frontend | the opener's popup-windows strip (Show, Close); the banner names the popup window (`window`) |

## 10. Relationship to P1 (#4478)

P1 is the pane half of this spec and stays: it can merge as is. What N1 changes in it: `NEW_POPUP`
requests become native popups instead of panes, tab dispositions start using its pane path, and
the rule moves to `agentmux-common`.

## 11. Decisions (the open questions, settled)

| Question | Decision |
|---|---|
| Does a bare popup show its address? | Not as a bar (by design, like the OAuth popup). The opener's strip shows it; the title bar shows the page's own title (§2, §8.4), and N2 prefixes it with the origin. |
| Where does it open on Windows? | In front of AgentMux, at the size the page asked (§2). Show in the strip covers it falling behind later. |
| Window or pane? | What the page asked for: popup features → bare window; tab or window → pane (§1, §2). |
| A person's pane, other site? | System browser, as today (§3). |
| Loopback targets? | Same origin as the opener, and not AgentMux's own: honored (§3). |
| The pane's `allowed_origins`? | Applies to its popups (§8.6). |
| Several agents? | A window belongs to the opener's owner; sharing a pane between agents (2026-10-07 spec B5) extends to its windows when that exists. |
| Popups of popups? | Count against the root pane; ownership follows it (§4). |

## 12. Tests

- **Rule** (`agentmux-common`): P1's tests, moved, plus loopback same-origin (honored; another
  port, AgentMux's own origins, and other loopback hosts refused) and the disposition mapping.
- **srv:** a popup from an owned opener is drivable by its owner only; from an unowned opener by
  nobody; Take over on the opener refuses further calls; a closed popup is refused and dropped; a
  hand-off on a popup puts the banner on the opener and locks it; the owned-pane push on record,
  Take over and close; only the host token can register, rename or close popups.
- **Host:** resolver Path 0 resolves a live popup and refuses a closed one or a pane label; the
  admission reads the owned set and the cap; a popup of a popup counts against the root.
- **Live** (`task dev`): a clicked `window.open(…, "width=…")` on the same site opens a native
  window sharing cookies, with `window.opener` set and `window.close()` closing it, listed in the
  opener's strip (Show, Close work); a `target=_blank` link opens a pane with its address bar; an
  agent's `OpenBrowser` pane clicks a popup open, sees it in its snapshot, fills and clicks in it,
  and hands off a sign-in the person completes in the window.

## 13. Phases

| | Scope |
|---|---|
| **P1** (#4478) | Popup panes, inherited ownership, snapshot listing. Merge as is. |
| **N1** | The disposition split; native admission in the host (rule in `agentmux-common`, the owned-pane push, loopback rule); bare-popup registration and close; resolver Path 0; srv popup records and the `pane` popup branch; banners on the opener; the popup-windows strip. |
| **N2** | Live addresses (`browser_popup_navigated`); the popup window's title prefixed with its origin; `allowed_origins` for popups. |
| **N3** | A `popup` condition for `BrowserWaitFor`; a setting to send all new windows to the system browser. |

# Spec: browser panes hibernate when hidden, and say so when they wake

**Date:** 2026-10-20
**Status:** active. Phase 0 (the spike, §9) done: `docs/reports/REPORT_BROWSER_PANE_HIBERNATION_SPIKE_2026_10_20.md` (about 550 MB freed for seven pages, wakes 0.35–2.3 s). Phase 1: #5290. The operator decided on 2026-10-21: on by default after 30 minutes hidden, and panes in hidden tabs start asleep at launch (§12). Phases 2 to 4 remain.
**Author:** korp (lead, per the operator, 2026-10-20)
**Trigger:** Operator, 2026-10-20: *"We need smart Hibernation, especially for browser panes, which we should start first."* and *"during restore, can you add a note to the user saying restoring, with est eta, and also the reason, explain that to save memory agentmux hibernates browser panes."*
**Builds on:**
- Camper's draft `SPEC_HIDDEN_PANE_MEMORY_AND_BROWSER_DISCARD_2026_10_18.md` (in Camper's clone, not on `main`): Chrome's discard rules (§4), the fork API for an in-place discard (§5), and the browser-pane lab (§7), whose numbers are quoted here.
- The renderer report of 2026-10-19 (korp, not yet on `main`): which renderer runs what, and what hiding frees today (nothing).
- `docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md`: the host-decides, page-may-decline pattern reused in §5.
**Supersedes, for browser panes only:** the 2026-10-19 decision to park hibernation (P4.4 of Camper's `PLAN_PERFORMANCE_AND_MEMORY_TRACKING_2026_10_19.md`) and P4.5's "discard only as an opt-in". Agent and terminal panes stay parked.

---

## 1. Summary

A browser pane that has been out of sight for a while **hibernates**: AgentMux closes its page, which ends its renderer process and frees its memory, and keeps everything needed to bring it back: address, title, icon, cookie jar. When the pane is about to be seen again, it **wakes**: the page loads again, and until it has, the pane shows a card that says it is restoring, about how long it will take, and why it was asleep.

- **Why browser panes first:** each one is a whole Chromium renderer, plus one per cross-site frame. Five ordinary pages cost about 700 MB, more than the app's whole 500 MB budget, and hiding them frees none of it (Camper's lab, run 1). Closing them freed about 475 MB at once (run 2).
- **What it doesn't free:** the GPU process's share (about 120 MB for seven pages) stays after the pages are gone (report §3.2).
- **What it costs the user:** a page load when they come back, 0.3 to 3 s by the host log (§3), mostly hidden by waking on intent (§6). Without a CEF fork change the page also loses its back/forward list, scroll position and form input; Phase 3 (§10) keeps them.
- **What stays awake, always:** a visible pane; a pane an agent is driving; a page playing or capturing media; a page with unsaved input; a pane the user pinned awake (§5.3).

## 2. What exists today

- A browser pane's native page is created by the frontend when the view mounts (`frontend/app/view/browser/use-pane-rect-sync.ts` `createPane`) and closed when it unmounts. While the pane's tab or pane tab is hidden, the page is collapsed to 0×0 and keeps running: renderer, scripts, timers, compositor.
- Chromium believes a collapsed pane is visible (`document.visibilityState` stays `visible`; Camper's lab, run 1), because AgentMux hides the wrapper window and native occlusion is off (`CalculateNativeWinOcclusion`, `crates/cef/src/app/mod.rs`). So none of Chromium's background throttling applies.
- What a pane already persists in block meta: its `url`, its identity (`browser:identity`), agent ownership and grants. Title and icon live only in the frontend's pane-state reducer (`frontend/app/store/browser-pane-state/types.ts`).
- A pane's cookie jar outlives its native page: closing and recreating the page (tear-off, redock, remount) keeps the tab signed in, Incognito included, while srv says a block uses the jar (`crates/cef/src/browser_pane/identity.rs`, `retain_jars`).
- The host knows when a pane collapses and shows again (`input_owner::pane_laid_out`, `PaneCollapsed` / `PaneShown`), which renderer serves each browser (`crates/cef/src/renderer_map.rs`), and the system's memory pressure (`crates/cef/src/memory_pressure.rs`: Normal, Warn, Critical).
- The `Browser` tool reaches a pane through srv (`/api/v1/ui/browser/*`) and the host's resolver (`crates/cef/src/browser_api/resolver.rs`, path 1: the pane's live browser). With no live browser it fails.

## 3. How long a wake takes

From this machine's host log (`browser pane created on UI thread` to the first `emit-nav-state … raw_cef_is_loading=false`), 2026-10-19 and 20, by kind of page:

| Page | Create to loaded |
|---|---|
| GitHub issue or PR, first load of the session | 2.3–3.8 s |
| GitHub issue or PR, later | 1.4–2.3 s |
| agentmux.ai, docs.agentmux.ai, a localhost dev server | 0.3–1.0 s |

A wake is a pane creation plus a page load, so these are its expected range. The spike measured wakes directly, cache warm: example.org 0.36 s, docs 0.44–0.49 s, Wikipedia 0.53–0.94 s, The Guardian 0.75 s, OpenStreetMap 0.76–1.07 s, Reddit 1.7–2.0 s, GitHub 2.0–2.3 s; a 600 ms hover before the click had three of seven pages ready before the tab showed (report §4).

## 4. States

A browser pane is in one of three states. The state is block meta, so every window, a window reload and a restart all agree:

| State | Native page | Meta `browser:sleep` |
|---|---|---|
| **awake** | exists | absent |
| **hibernated** | none | `{ since, reason, url, title, favicon, loadMs }` |
| **waking** | being created and loaded | still set; cleared when the page has loaded |

- `reason`: `hidden` (the policy, §5), `pressure` (the policy under Warn or Critical), `launch` (§5.4), `manual` (the command, §7.3).
- `loadMs`: how long this pane's page took to load the last time it did, for the card's estimate (§7.2).
- Hibernating writes the meta first, then closes the native page. Waking creates the page from `url` with the pane's identity, as a mount does today, then clears the meta once the page reports loaded.

## 5. When a pane hibernates

### 5.1 Who decides

**The window decides, with two answers from the host.** (An earlier draft had the host propose candidates, as the recycle governor does. Built, the window already held everything but two facts, so it asks for those instead.)
- **The window** (`frontend/app/view/browser/hibernation.ts` `policyTick`, every 30 s) knows how long each browser pane has been hidden (the pane-tab visibility signal: a hidden window tab, an inactive pane tab, a minimized window), what agents and the person are doing with it, and its settings.
- **The host** answers `memory_pressure_level` (Normal, Warn, Critical) and, per candidate, `browser_pane_hibernation_check`: whether its page plays media, has a changed form field, has a `beforeunload` listener, or has DevTools open (`crates/cef/src/browser_api/hibernation.rs`, read in an isolated world). A check that can't run keeps the pane awake.
- **Order:** the longest-hidden first, as Chrome orders its discards. Ordering by renderer size, as the first draft had, needs a pane-to-process map the window doesn't have; it is a later refinement if memory pressure shows it matters.
- A pane that declines says why in the log (`[hibernate] stays awake reason=…`) and is asked again next tick.

### 5.2 Thresholds

A pane is a candidate when it is hidden (collapsed, in a hidden window tab, an inactive pane tab, a minimized pane or a minimized window) and has been for at least:

| Pressure | Hidden for | Order |
|---|---|---|
| Normal | **30 minutes** | longest hidden first, one per tick |
| Warn | 5 minutes | longest hidden first, up to 3 per tick |
| Critical | 1 minute | longest hidden first, all candidates |

`browser:hibernateafter` (minutes) replaces the Normal figure. The Warn and Critical figures are constants. 30 minutes is a starting point for the spike to check against how often the operator returns to hidden panes.

### 5.3 Never hibernated

Checked by the page at decision time; any one declines:
- **Visible** in any way (shown, or under a hover-wake, §6).
- **An agent is using it:** `browser:owner_agent` is set, an Auto-approve grant (`browser:bypass`) is live, a `Browser` tool action reached it in the last 10 minutes, an approval banner is waiting, or a hand-off to the user is in progress. Agents drive panes in hidden tabs (three did on 2026-10-18).
- **Media:** camera, microphone or screen capture (`on_media_access_change`, host); a playing `<audio>` or `<video>` element (page check). Web Audio isn't visible to the page check; Phase 2's fork change can add `IsAudible`.
- **Unsaved input:** an edited form field (value differs from its default), or a `beforeunload` listener (CDP `DOMDebugger.getEventListeners` on `window`).
- **Something pending:** an HTTP-auth prompt, a JavaScript dialog, a download in progress, a file chooser, an open popup window (`browser:popup_windows`), DevTools attached.
- **Pinned awake:** the pane's `browser:keepawake`, or its site in `browser:keepawakesites`.
- **Active while hidden:** its title or icon changed while hidden (a chat, an inbox), as Chrome's "background activity" rule. Cleared when it is shown.
- **A local dev server page** (`localhost`, `127.0.0.1`): reloading it can rerun the developer's app state. Off the list only if the spike shows it is harmless.

### 5.4 At launch

Today every browser pane in every tab loads at launch. With hibernation on, a browser pane in a tab that isn't shown at launch **starts hibernated** (`reason: launch`), with the URL and title it had, and its page is never created. It wakes like any other. "At launch" is a pane that mounts hidden within 20 s of its window's first page load, not a reload: a window reload keeps its browser panes' pages (#4944), and the recycle governor's reload must stay invisible. The §5.3 checks the window can make without a page apply; the page checks can't, since there is no page yet. This cuts launch time and memory for the operator's usual window of four tabs and nine browser panes.

### 5.5 Settings

- `browser:hibernate` (bool): the feature. **Default: on**, recommended (§12, decision 1).
- `browser:hibernateafter` (minutes, default 30).
- `browser:keepawakesites` (list of origins).
- Per pane: **Keep awake** in the pane's menu and on the card, stored as `browser:keepawake`.

## 6. When a pane wakes

Waking starts **before** the pane is seen whenever there is a sign that it will be:
- the pointer rests on its window tab's pill for 100 ms, or presses it;
- the pointer rests on its pane tab's pill, or presses it;
- the keyboard switches to its window tab or pane tab (the switch starts the wake; the tab shows at once with the card);
- something reveals the pane: a notification, a `Pane` open or focus, a search result;
- the `Browser` tool acts on it (§8).

A pane woken on intent keeps its sleep record until it is shown: if it has loaded by then it appears at once, with no card; if no switch comes within 10 s it goes back to sleep, loaded or not. (The spike first left a loaded pane awake, and a pass of the mouse over the tab bar woke a whole tab: report §4.3.)

A pane woken while its tab is hidden is created at 0×0, so its page never draws over the tab that is shown, and laid out on show. The spike measured every such page at its pane's size once shown (report §4.4).

## 7. The card

### 7.1 What it says

Shown in place of the page from the moment the pane is visible until its page has loaded:

> **Restoring github.com/agentmuxai/agentmux/pull/4138**
> About 2 seconds
>
> AgentMux put this browser pane to sleep to save memory. A sleeping pane frees its page's memory, usually 100 to 200 MB, and reloads when you come back to it.
>
> Back and forward history, scroll position and anything typed into the page aren't kept yet.
>
> [Keep this pane awake]  ·  Settings

- The **first line** names the page by its saved title and icon (the address when there's no title).
- The **estimate** (§7.2), or "Still loading…" once the estimate has passed by more than half.
- The **reason**, in one sentence, with the typical saving. Under pressure it says so: "Your computer was low on memory, so AgentMux put this browser pane to sleep…"
- **What isn't kept**, only while it isn't: Phase 3 (§10) removes the line.
- **Keep this pane awake** sets `browser:keepawake`. **Settings** opens `browser:hibernate`.
- The card shows only the first time the user sees each pane after it slept. A pane that wakes on hover and finishes before it is shown never shows one.

### 7.2 The estimate

- From the pane's own `loadMs`, the time its last load took.
- With none, 2 s (the spike's measurements, report §4.1). A per-site history (the last loads of each origin) is a later refinement.
- Rounded to "About 1 second", "About 2 seconds", "A few seconds" (over 3.5 s).

### 7.3 Layout and access

- The card fits the pane's minimum width (250 px): the text wraps, the buttons stack.
- It is the pane's content, not an overlay, so it never covers another pane, and it uses the theme's colours (`am-*` tokens in the app, not page styles).
- A screen reader hears it as a status (`role="status"`), once.
- The pane header shows a small moon icon while the pane is hibernated, with the same sentence as its tooltip. The command palette has **Hibernate Browser Pane** and **Wake Browser Pane** for the focused pane (`reason: manual`).

## 8. Agents and hibernated panes

- **The `Browser` tool wakes the pane first.** srv sees `browser:sleep` on the target block, asks the pane's window to wake it, waits for the page to load (at most 30 s), then runs the action. The tool's answer says the pane was woken, so the agent knows the page reloaded. A pane in no open window (a closed floater) answers with an error saying so.
- **An agent's work keeps a pane awake** (§5.3), so a hidden tab where agents are working is never put to sleep under them.
- **`Pane` `layout`** reports a hibernated browser pane with `"hibernated": true`, so an agent knows before it acts.

## 9. Phase 0: the spike

Half a day, in my own `task dev` build with its own data folder, never in the operator's instance. No fork change. Built behind a hidden setting so it can merge as the base of Phase 1 if it holds up.

**Build only what the measurements need:**
- the meta state (§4) and the frontend's hibernate and wake: write the meta, close the native page, render a plain card; on wake, create the page from `url`;
- the **Hibernate** and **Wake** commands (§7.3);
- wake on show and on tab-pill hover (§6).

**Measure**, on Windows, production build of the branch, with the same sites as Camper's lab plus the operator's kinds of pages (a GitHub PR and an issue, signed in; docs.agentmux.ai; Wikipedia; a news page; a paused video; an Incognito pane; a localhost dev server):
1. **Memory freed:** each pane's renderer processes, the GPU process and the app total, before and at +30 s and +5 min after hibernating all panes in a hidden tab. The question Camper's run left open: does the GPU process give its share back.
2. **Wake time:** command or show to page loaded, per site, cold and warm; and with hover-waking, how often the page is ready before the tab is shown.
3. **What survives:** signed-in state (GitHub), Incognito jar, a page's `sessionStorage`, title and icon; what is lost (history, scroll, form input) and whether that matches the card.
4. **Edge cases:** a page loaded at 0×0 then shown; a wake while the window is reloading; hibernating a pane whose tab is torn off.

**Output:** a short report next to this spec with the numbers, and a go or no-go for Phase 1's defaults and for the fork work.

## 10. Phases

| Phase | What | Needs |
|---|---|---|
| **0** | The spike (§9) | nothing |
| **1** | Hibernation without a fork change: states (§4), the host's proposals and the page's checks (§5), launch-asleep (§5.4), wake on intent (§6), the card (§7), agents (§8), settings | Phase 0's numbers |
| **2** | Fork: `CefBrowserHost::SetHidden(bool)` calling `WebContents::WasHidden` / `WasShown` for windowed browsers. Hidden but awake panes then get Chromium's own throttling and stop compositing. Optional: `IsAudible()` for §5.3 | a CEF rebuild (narko, Korp) |
| **3** | Fork: in-place `Discard` / `IsDiscarded` / `LoadIfNecessary` with `WebContentsDiscard` on (Camper's §5). A hibernated pane then keeps its history, scroll and form input, and the card loses its "isn't kept" line. Phase 1's close-and-recreate stays as the fallback when a discard fails | a CEF rebuild; a ceftest for scroll, form and history |
| **4** | Freeze: a hidden pane awake for 5 minutes is frozen over CDP (`Page.setWebLifecycleState`), CPU to zero, with §5.3's exceptions | Phase 2, so freezing a page Chromium thinks is visible isn't fighting it |

Agent and terminal panes are a separate spec once this one has shipped Phase 1: they need srv to own their timers and live status first (the agent pane's failure retry, question timeout and working indicator live in the pane today; discussed with the operator on 2026-10-19).

## 11. Tests

- **Unit:** the candidate order and thresholds per pressure level; every §5.3 exception declines with its reason; the estimate's fallbacks and rounding; the card's copy for each `reason`.
- **Frontend:** a hibernated pane renders the card and creates no native page; a wake creates it from `url` and clears the meta after load; hover-waking returns to sleep after 10 s without a switch; a pane shown during its wake shows the card, one shown after doesn't.
- **srv:** a `Browser` action on a hibernated pane wakes it, waits, then acts, and its answer says so; a wake that times out fails the action with that reason.
- **Host:** proposals only for collapsed panes; a declined pane isn't proposed again until shown; nothing proposed with `browser:hibernate` false.
- **End to end** (the spike's script, kept): hibernate five panes in a hidden tab, check renderer exit and memory; wake by hover and by switch; GitHub still signed in; Incognito still in its jar.

## 12. Decisions for the operator

1. **Default on?** Decided 2026-10-21: **on by default, 30 minutes hidden**, sooner under memory pressure. (Camper's draft and the operator's 2026-10-18 note had a reload on show as opt-in only; wake-on-intent and the card are what changed that.)
2. **Launch-asleep** (§5.4): decided 2026-10-21, yes.
3. **Local dev servers** never hibernate (§5.3): built that way; revisit if a reload proves harmless.
4. **Phase 2 and 3 fork work:** after Phase 1 ships and is measured in daily use, or in parallel. Still open.

Agent: korp@narko

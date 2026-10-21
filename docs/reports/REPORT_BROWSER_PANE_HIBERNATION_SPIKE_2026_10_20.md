# Report: browser-pane hibernation spike (Phase 0)

**Date:** 2026-10-20
**Status:** analysis. Measured; Phase 0 of `docs/specs/SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md` §9.
**Author:** korp
**Build:** production portables of branch `korp/browser-hibernation-spike` on `main` @ `30daa7c28`, Windows 11 (narko), each run in an isolated instance (own data folder, CDP port 9341). Lab script: built on Camper's browser-pane lab and the memory lab's `driver.mjs` / `sample.mjs`.

## 1. Verdict

**Go for Phase 1.** Hibernating seven ordinary pages in a hidden tab freed about 550 MB within 5 seconds, half the app's memory. Wakes take 0.35 to 2.3 s, a hover before the click hides part of that, and everything the card promises held. One design change came out of it (§4.3), and the GPU process doesn't give its share back (§3.2), which is a separate problem.

## 2. Setup

Seven browser panes in one window tab: Wikipedia (an article), GitHub (a repo page), The Guardian, Reddit (a subreddit), OpenStreetMap, docs.agentmux.ai, and example.com as a probe (navigated to example.org for a two-entry history, with a cookie, `sessionStorage` and `localStorage` set; the Wikipedia page scrolled to 2,500 px). Then a new tab, which hides them, and the spike's **Hibernate Hidden Browser Panes** command.

Run 1 measured memory up to 5 minutes after hibernating; run 2 (after the §4.3 change) the wake paths again, the pages' size after a hidden wake, and a hover with no switch.

## 3. Memory

### 3.1 What hibernating frees

Private working set, MB:

| Phase | App total | Browser-pane renderers | Window renderer | GPU process |
|---|---:|---:|---:|---:|
| Empty window | 360 | none | 234 | 45 |
| 7 pages visible | 1,100 | 497 (13 processes) | 279 | 161 |
| Hidden 60 s | 1,132 | 494 | 296 | 174 |
| **Hibernated +5 s** | **574** | **7 (1 process)** | 230 | 175 |
| Hibernated +30 s | 570 | 7 | 231 | 172 |
| Hibernated +5 min | 529 | 8 | 229 | 168 |

Run 2 repeated it: 1,125 → 580 MB at +5 s.

- The pages' renderer processes, 13 of them because site isolation gives cross-site frames their own, end within 5 seconds. The one left is not a page's.
- The window's own renderer also drops about 65 MB, the pages' placeholders and nav state.

### 3.2 The GPU process keeps its share

It grew from 45 to about 175 MB with the pages, and is still 168 MB five minutes after they are gone (run 2: 199 → 197 at +30 s). Closing pages doesn't return it, so neither will the in-place discard. That is the GPU process's own retention, worth its own look (the D2 track of the performance plan).

### 3.3 Waking everything at once costs more than the pages did

15 s after all seven woke together, their renderers held 828–850 MB, against 494 before they slept: the pages loading their ads and media again, all at the same moment. Phase 1 wakes only the panes being looked at, so this is the worst case, not the usual one.

## 4. Waking

### 4.1 How long

From the wake's start to the page loaded, cache warm (the same pages a few minutes earlier):

| Page | Click, run 1 | Click, run 2 |
|---|---:|---:|
| example.org | 0.36 s | 0.38 s |
| docs.agentmux.ai | 0.49 s | 0.44 s |
| Wikipedia | 0.94 s | 0.53 s |
| The Guardian | 0.75 s | 0.77 s |
| OpenStreetMap | 0.76 s | 1.07 s |
| Reddit | 1.73 s | 2.01 s |
| GitHub | 2.04 s | 2.30 s |

The spec's estimates (§3 there, from the host log: 0.3–1 s light, 1.4–2.3 s GitHub) hold. With all seven waking at once these are an upper bound for a single pane.

### 4.2 A hover before the click

Resting on the tab's pill for 600 ms, then clicking (run 2): the three light pages had loaded before the tab showed and appeared with **no card**; the others showed the card for 0.56 to 2.0 s instead of 0.7 to 2.3 s. The press alone starts the wake about 30 ms before the tab shows, which is nearly nothing.

### 4.3 A hover with no switch: the change

Run 1 woke every page on a hover and left any page that loaded within 10 s awake: a pass of the mouse over the tab bar woke the whole tab, back to about 990 MB. Now a page woken on a hover or a press **stays asleep until it is shown**: if it loaded first it appears at once with no card, and if no switch comes within 10 s it goes back to sleep, loaded or not. Run 2: all seven abandoned at 10.0 s, page targets back to 0. Two renderer processes (110 MB) were still exiting at +15 s; the app was at 739 MB against 580 asleep.

### 4.4 Pages woken while hidden lay out at the pane's size

A pane woken while its tab is hidden is created at 0×0 so its page never draws over the tab that is shown. Once shown, every page measured `innerWidth`×`innerHeight` equal to its pane (530×483, and 260×446 for the narrower one). Nothing needs the real size at creation.

## 5. What survives a wake

| | Before | After |
|---|---|---|
| Cookies (so sign-ins) | `hib=1` | kept |
| `localStorage` | `1` | kept |
| `sessionStorage` | `1` | **lost** |
| Back/forward history | 2 entries | **1** |
| Scroll position | 2,500 px | **0** |

Exactly what the card's "isn't kept yet" line says. Phase 3 (the in-place discard in the CEF fork) is what keeps history, scroll and form input.

## 6. The card

- Shown from the moment the pane is visible until its page has loaded; on the click path, 0.33 to 2.25 s.
- Not shown for a page that loaded during a hover.
- The spike logs `card-shown` again whenever the pane's meta changes (the page's address updates during a load); only the log repeats, not the card. Phase 1 logs it once.

## 7. Changes to the spec from this

1. §6: a wake on intent keeps the sleep record until the pane is shown, and goes back to sleep after 10 s without a switch, loaded or not (§4.3).
2. §6: a pane woken while hidden is created at 0×0 and laid out on show (§4.4); the spec's open question is answered.
3. §3 / §7.2: the measured wake times replace the host-log estimates as the defaults.
4. §1 and §10: the GPU process's share is not freed by hibernating; the expected saving per pane is its renderers only.

## 8. Next

Phase 1, without a fork change: the host's proposals and the page's checks (spec §5), launch-asleep (§5.4), the `Browser` tool waking a pane before it acts (§8), the pane header's moon icon and the settings (§5.5, §7.3), then a soak with the operator's real tabs.

Agent: korp@narko

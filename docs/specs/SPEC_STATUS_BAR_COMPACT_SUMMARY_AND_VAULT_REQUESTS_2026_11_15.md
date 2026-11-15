# Spec: one compact status bar summary, and Vault requests that notify there

**Status:** active (approved by the operator through AgentY, 2026-11-15; PR 1 shipped in #7455: the Vault count and the kind ratchet; PRs 2 to 4 as listed in section 7)
**Author:** Masty@starpower. **Task:** #43 (build #44). **Read against:** origin/main `b966bedbe`, 2026-11-15.
**Asked by the operator:** "lets make sure we get value requests a notification area in the status panel, in fact, lets get a condense info part that lets you know 20 agents, 4 tasks, 4 vaults, etc, put it in a compact form, (we already have for agents running and tasks, perhaps others, lets consolidate all those". "Value requests" is read as Vault requests (AgentY's reading).
**Added by the operator, 2026-11-15, verbatim:** "also, we need pending questions, its often the case I hear the tone requesting a question or a browser permission but I cant find where it is coming from. we need those in the status bar". AgentY's reading (not the operator's words): the rule becomes **no attention tone without a row the person can find**.

## 1. What the status bar counts today

Every item is in `frontend/app/statusbar/StatusBar.tsx` (`registerBuiltinStatusBarItems`). Only those that count something the person can act on matter here.

| Item | Counts / says | Source | Click |
|---|---|---|---|
| `agent-counts` (AgentCountsStatus) | agents with a process ("N running"), those waiting on the person ("N waiting on you"), Kanban tasks that need them ("· N tasks") | `runtimeCounts(sharedAttentionRows)`, `attentionItems` (this window's waiting panes joined with srv's `waiting_on_user` rows), `useKanbanNeeds()` (tasks plus account episodes) | nothing waiting: opens the Swarm. Otherwise the AttentionListPopover: a row per waiting pane with "Go"/"Open", and a Kanban row |
| `vault-requests` (VaultRequestsStatus) | "N Vault requests" that wait with **no agent pane on screen** (`block_id` empty on `vaultUses()` and `vaultRequests()`) | srv's `vault` event, kept in `view/vault/vault-store.ts` | opens the Vault pane's Requests list at the first one |
| `update` | update downloaded / ready to restart | UpdateStatus | restart popover |
| `version-skew` | window and backend versions differ | `srv-info` | reload |
| `muxbus` | cloud relay reconnecting / sign-in needed | `muxbus-delivery` | sign in |
| `connectivity` | Offline / Connection issues | `store/connectivity` | popover |
| `config` | config errors | atoms | opens the errors |
| `browser-pause-all` | agent browsing paused | `browser-pause-all` | resumes |
| `multi-input` | "type into all terminals" is on | global atom | turns it off |
| `last-sound` | "who was that": the last sound, for a minute | agent runtime | Audit row |
| `token-usage`, `history-usage` | usage meters, the failing-model dot | stores | popovers |
| `backend`, `system-stats`, `gpu`, `host`, `version` | health and hardware meters, the instance chip | various | popovers |

Findings that shape the design:

- **The bar already holds two partial answers for Vault.** A request asked as a card in an agent pane is counted by the agent's "waiting on you"; one with no pane on screen is counted by `vault-requests`. Neither says "Vault" for the first kind, and a request whose pane closed after it was asked is in neither: the cards spec lists this as a known gap (`SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md`, C3 "found in the Linux look check", the status count "covers" only requests with no pane). It is the likeliest reason a request can wait unseen; this spec closes it by counting every pending request, whatever its pane.
- `agent-counts` is kept at the last fit tier while anything waits (`slotDropAt`), `vault-requests` has no drop tier. Both rules carry over.
- The meters, the modes (multi-input, browser pause) and the faults (offline, version skew, config, update) are not "things waiting on the person to answer". They stay as they are.

## 1b. Every place that asks the person, and the tone

**The tone.** Only one sound is an "ask": `agent.waiting.for.input` (`source: "ask"` in `notification/sound/sounds.ts`). The sound service refuses it from the event bus, so it plays only for a pane (or an `app:` key) in the waiting registry (`notification/waiting-for-you.ts`), whose members feed the toast and the attention list. Every other sound is an "event" (a turn ended, a message taken) and names its pane. So the question "what asked?" always has an answer in the registry; the gaps below are where the registry's answer never reaches a row. Read from code on `b966bedbe`; nothing here was observed at run time.

| Who asks | Where it enters the registry | Goes to a row today? | Part of the summary |
|---|---|---|---|
| An agent's question (AskUserQuestion) | `useAgentQuestions`, source `question` | Yes, in the pane's row (this window's registry, plus srv's `waiting_on_user` row for other windows) | Asks |
| A tool permission | `useAgentDecisions`, source `permission` | Yes, same | Asks |
| "Will stop unless you keep it running" | source `shutdown` | Yes, same | Asks |
| Browser hand-off or approval of a committing click (`browser_attention.rs`) | srv `userattention`, kind `browser`, in the browser pane | Yes, unless Settings → Sounds "notify:waiting:browser" is off, which also drops its row (`browserWaitsShown`) | Browser |
| SSH or host consent (`app_api/connections.rs`, `agent_notice.rs`), desktop control (`app_api/desktop.rs`), an upgrade ask (`upgrade_request.rs`) | `userattention`, kind `consent` | In the pane's row when `block_id` is set; **with no pane it is an `app:` request: it plays the tone (only while the window is not in front), sends no OS notification, and `waitingPanes()` skips it, so no row anywhere** | Asks |
| A widget install (`widget_agent_handlers.rs`) | kind `widget` | Same as consent | Asks |
| A pane asks about the person's own click (`sagas/pane_confirm.rs`) | kind `confirm` | In the pane's row | Asks |
| A Vault use or new-entry request (`vault_ask.rs`, `vault_handlers.rs`) | kind `vault` | A card in the pane counts as the agent's wait; no pane: the Vault item; a pane closed after asking: neither (§1) | Vault |
| Cards in an agent pane (`pane-requests.ts`: Vault cards, others) | each card is a source of its pane's wait | Yes, as a pane row | Asks (Vault ones: Vault) |
| A Kanban task that needs the person (`Tasks ask`, a held task, an account episode) | not the registry: `useKanbanNeeds()` | By design no tone or toast; a row in the list | Tasks |
| A page's camera, microphone, location or notification request in the **person's own** browser pane (`pane-media-permission-prompt.tsx`) | none found: no registry entry | An in-pane prompt only; no tone from this path | **Gap** (Browser seat, Korp@narko): decide whether it joins the registry |
| The same request in a pane an **agent drives** (`cef/client/agent_permission.rs`) | srv decides it through the sensitive-operation countdown and files a request when it denies | Not a registry entry | **Gap** (Korp@narko, AgentY for the countdown): the operator's "browser permission" tone may come from here or from a hand-off; to confirm with Korp which path plays it |

**Gaps, each with an owner:**
1. **`app:` requests have a tone and no row** (consent, widget install, upgrade and desktop asks with no pane, a Vault ask with no pane). This is the likeliest source of "a tone I cannot find". Fix in the summary: read srv's open list (`notify.attention`, `user_attention::open_requests()`, which holds every open announcement whether or not it has a pane) in addition to this window's registry, so a request with no pane is a row; its click goes to where it is answered (§2). Owner of the registry: AgentX's lane (agent pane and notification); srv's list is AgentY's.
2. **A request in another window is a row only through srv's `waiting_on_user` rows** for agent waits; srv's own announcements reach every window when `window_ids` is empty. Verify in the build with two windows.
3. **Turning browser waits off removes the row with the tone.** The summary keeps counting them (muted, no tone) so the person can still find one that is open.
4. **The two browser permission paths above** have no registry entry. Korp@narko owns the browser pane; the build asks it whether to register them (a `userattention` kind `browser`, which the summary already counts).
5. **Anything that plays the ask tone with no queryable list** is, by the sound service's own rule, impossible for a new sound (it does not compile without `source`); the gaps are the registry entries that skip the row, so a ratchet test is added: every kind `userattention` can carry maps to a part of the summary (a new kind with no part fails the test).

## 2. The summary

One item, `agentmux:summary`, replaces `agent-counts` and `vault-requests`. It is a row of parts, each an icon and a number, the same look for every part (the existing `status-bar-item` style and the kit's colours):

`robot 20 · question 2 (warn) · globe 1 (warn) · vault 4 (warn) · kanban 4`

Order, left to right: **agents running** (quiet), **Asks** (every question, tool permission, consent, confirm, widget install and shutdown ask: warning colour), **Browser** (hand-offs, approvals and permission prompts of browser panes), **Vault**, **tasks** (Kanban tasks that need the person). "Asks" replaces today's "waiting on you". The tooltip of each part spells it out ("4 Vault requests wait on you"); `aria-label` carries the whole sentence. A word is not shown beside the number: at the bar's width four words cost more than four icons. (Decision D1.)

- **Zero states.** A part with zero is not drawn, except that "agents running" shows while any agent runs. With nothing running and nothing waiting the item is not drawn, as today.
- **One click behaviour.** Any click on the item opens one popover (`AnchoredPopover`, the AttentionListPopover's own frame, no window modal) with a section per part that has rows: **Asks**, **Browser**, **Vault**, **Tasks**, then the recently ended asks (§4). Each row names what waits, how long, and has one button that goes to where it is answered. With nothing waiting, a click opens the Swarm, as today. A click on a part opens the popover with that section first.
- **Narrow bar.** The item keeps its tier rule: never dropped while anything waits. Parts shed from the right: first "agents running", then "tasks", then the item shrinks to one icon with a badge carrying the total that waits (Asks + Browser + Vault + tasks). Asks, Browser and Vault go last. At the last tier it is the badge alone.
- **Counting once.** A pane that waits on a Vault request is one row, under Vault, not also under Asks (and likewise a Browser wait is under Browser only): the join in `mergeAttention` tags such an item by `block_id` and the summary drops it from the agents' count. The total never exceeds the number of distinct things asked of the person.

**Rows and where a click lands (every part).** A row says who asks (the agent's name in its colour), what in a few words (the registry's `what`: "asks to approve a Bash call", "needs you in the browser"), how long it has waited, and has one button. The button is the existing `goToAttention`: `revealBlock` for a pane (it is the one reveal path), `openAgentInActiveTab` for a parked agent. Cases the build checks in its own dev build and the look checks, each with what happens:
- **Pane in another tab or window:** the reveal switches to it (to confirm for a second window).
- **A background pane tab, a minimized or hibernated pane:** revealed, and a hibernated pane wakes (to confirm with Korp@narko for browser panes); the request stays open meanwhile.
- **A pane closed since:** the request is gone with it: the registry ends a wait when its block is deleted (`closed`), srv's holder ends its own wait (the call is denied, as for a closed pane today), and the row leaves at once. A click on a row that goes in the same moment says "that was closed" in a notice, not nothing.
- **No pane (an `app:` request):** the button goes to where it is answered: Settings → Connections for a host consent, the widget prompt for an install, the Vault Requests list for a Vault ask, the upgrade prompt for an upgrade ask. Each kind names its target in one table in the build; a kind with no target fails the ratchet test.

## 3. Vault requests

- **What is pending:** every entry of srv's `vault` event lists, `uses` (an agent's use of an entry waiting for a yes) and `requests` (an agent asking to add an entry), whatever `block_id` says. Agent2-6 (the Vault seat) confirmed these two lists are everything the Vault itself waits on, a decided or timed-out one leaves them (no held or expired state), and the event's shape is not changing (one additive field is coming with batch import; it will say when). I add no second source. Not in that event, so not in the count: the "Save to Vault?" offer and the sign-in countdown notice (block meta `browser:signin_offer`, `browser:signin_pending`), and entry-expiry reminders. A use with `goes_ahead` true is a countdown that proceeds on silence unless the person vetoes; it counts (a veto is theirs to give) and its row says "goes ahead in N s", not "waits".
- **Row:** the requesting agent's name in its colour, the entry's name (or "a new entry" for a request), how long it has waited (`waitedFor`), never a value. A use that goes ahead on its own after a countdown says so as the card does ("goes ahead in 0:12 unless you deny"); a drone's use never does (it is denied on silence).
- **Click:** srv does not say whether a request's pane is on screen (`block_id` is the only pane field, empty when asked outside a pane), so the frontend decides: if this window's attention registry knows the pane, go to its card (the existing `goToAttention`); otherwise the row says "no pane" and goes to the Vault pane's Requests list at that row (`openVaultAt(id)`).
- **Attention.** While one waits: the Vault part and the item's badge in the warning colour. When a new one arrives: the part pulses once (a short highlight, no repeat), only when the Animations setting allows motion (`motion.ts` policy; Off gives no animation, the colour change stays). **No new sound and no new toast:** srv's attention already plays the tone and shows the toast for a request with a pane (`vault_ask`), and the cards spec lists an OS notification for a request with no pane as its own follow-up; this spec does not add a second channel. The badge and the colour are what it adds, and they stay for as long as the request waits, which is the part that failed to happen. (D4.)
- **Operator-only:** the Vault part keeps `data-operator-only`, so a widget cannot read or click it.

## 4. What else belongs in it

First build: agents running, **Asks** (questions, tool permissions, consent, confirm, widget install, shutdown, `app:` requests included), **Browser** (hand-offs, approvals, and the permission prompts once Korp registers them), Vault, Kanban tasks (Needs you plus account episodes, as `useKanbanNeeds` counts them today).

Later, one part each, in the same shape (each a follow-up task, none needed for the tone rule):
- agents that are **signed out or out of quota** (`signed_out`, `limited` in `AgentRuntimeState`): they wait on the person to sign in or to wait;
- **held messages** for a stopped agent;
- **Review column** tasks, if the operator wants them apart from Needs you.

Not in it: the update chip, version skew, offline, MuxBus, config errors, the browser-pause and multi-input modes and every meter. They are state of the app, not requests to the person, and merging them would hide the faults the bar is there to show.

**"What was that sound?"** The popover also shows, under its live sections, the last few asks that **already ended** (answered, denied, timed out, closed), each with how it ended and how long it waited, for two minutes. The registry already knows each end (`waiting-ended`, with `submitted` or `closed`), so it needs a small ring beside `sound-history.ts`, no new source. It answers the case the operator describes after the fact: the tone played, someone answered it in another window or it timed out, and the row is already gone. It is for asks only; tool tones and event sounds stay with the "who was that" chip (`LastSoundStatus`) and the Audit. (D6: recommended yes.)

## 5. Elsewhere

- **Tray tooltip:** could reuse the same pure summary function ("20 agents · 2 waiting · 4 Vault"); a separate task, since the tray is host-side.
- **Mobile feed:** the data is srv's (`vault` event, agent runtime rows, Kanban needs), so the feed can show the same counts without the frontend; out of scope here.

## 6. Build, tests, checks

- **Shape:** a pure `summarize(rows, items, vaultUses, vaultRequests, tasks)` returning the parts with counts, tones and rows (vitest, no DOM), then `StatusBarSummary.tsx` and the popover's Vault section reusing `AttentionListPopover` and `waitedFor`. `AgentCountsStatus.tsx` and `VaultRequestsStatus.tsx` go; `status-bar-fit.ts` keeps one id.
- **Tests:** vitest for the summarizer (each part, zeros, the once-only join of a Vault-waiting agent, a request whose pane is closed, a request that is answered, an `app:` request with no pane becoming a row, browser waits kept when the setting hides them, the ended-asks ring), a ratchet over the kinds `userattention` can carry (each maps to a part and has a click target), for the item (parts shown and hidden, tooltip, badge, click opens the popover or the Swarm, Animations Off gives no pulse), and for the fit tiers (parts shed in order, the item kept while anything waits). The existing `AgentCountsStatus`, `status-bar-tiers` and `status-bar-visibility` tests move to the new item.
- **Screenshots** from my own `task dev` build at three widths (wide, mid, narrow), dark and light, with a fake Vault request, a waiting agent and a Kanban task mounted over CDP (no real agent started).
- **Look checks** on macOS (mine), Windows and Linux before merge.

## 7. Decisions for the operator (recommendation first)

- **D1. Icon plus number per part, no words** (recommended), or number plus a short word ("4 Vault"). Words read faster but take about twice the width and go first in a narrow bar.
- **D2. One summary item that replaces `agent-counts` and `vault-requests`** (recommended) versus keeping them and adding a summary beside them.
- **D3. The Vault count is every pending use and request, whatever its pane** (recommended), counted once and not also as an agent waiting; a countdown use counts and says "goes ahead in N s".
- **D4. Attention is the badge, the colour and one pulse; no new sound or toast** (recommended). Alternative: an OS notification for a request with no pane, which needs a decision from Agent2-6's seat (cards spec follow-up).
- **D5. First build holds all of §4's first list: running, Asks, Browser, Vault, tasks** (recommended; the operator's new line puts questions and browser requests in it), signed-out and quota agents next, then held messages.
- **D6. The popover also lists the last few asks that already ended, for two minutes** (recommended), so "what was that sound?" has an answer after it was answered elsewhere.
- **D7. Browser waits turned off in Settings still count, muted, with no tone** (recommended): the setting silences them; it must not hide them from the one list.

**Phases.** PR 1: the summarizer over srv's open list (`notify.attention`) as well as the window's registry, so every ask has a row, the Vault count over every pending request, the kind ratchet, tests (no visible change except the counts). PR 2: the item and popover with Asks, Browser, Vault and tasks, the click targets, replacing the two items; screenshots; the three look checks. PR 3: the ended-asks ring (D6). PR 4+: the browser permission paths once Korp@narko registers them; one part each for signed-out and quota agents, then held messages.

**As built, PR 2.** `agentmux:summary` (`SummaryStatus.tsx`, with `useSummary` in `use-summary.ts`) replaces `agent-counts` and `vault-requests`; `status-bar-fit.ts` keeps one id (`SUMMARY_ID`), and the item is kept while the summary's own `waiting` total is above zero (a Vault-only wait, an `app:` request or a Kanban task keeps it too). Each part is a button (icon, number, tooltip in words). The list is `SummaryPopover.tsx` on the attention list's frame, one section per part with rows, the clicked part first. A narrow bar sheds the agents at fit tier 2, the tasks at tier 3 and shows the one badge at tier 4 (`_summary.scss`). Row buttons go through `goToSummaryRow` (`summary-go.ts`): a pane by `goToAttention`, a Vault request by `openVaultAt`, the Kanban row to the Kanban, and a request with no pane to the place its kind is answered: a host consent to Settings → Devices, a widget install to Settings → Widgets, a desktop request to Settings → Agents, an upgrade to the instance panel, anything else to the Swarm. Not yet built: D7 (a browser wait turned off in Settings still counts, muted) needs the registry to keep those rows, and goes with the browser permission paths in PR 4; D6, the ended-asks ring, is PR 3.

# Report: reactive coverage, which views go stale and one cheap way to fix them

**Task:** #21823 (research). **Author:** Loap@narko, 2026-11-03. **Code read:** `origin/main` at `ef8ebc8ac`. No live check was run.
**Asked by the operator (2026-11-03):** "i notice connectors did not update as accounts were connected ... instead I switch pane tabs and switched back and I saw the accounts. this leads to a greater audit to make sure we have good reactive coverage. ... connectors is definitly a nueral center (same with settings) that we need high-signaling"

## 1. Recommendation

1. **Fix the Connectors cause now, one line:** the service-OAuth path (`account.oauth.*`, used for Google and GitHub, and by an agent that connects Google itself) saves the account and never publishes `IdentityAccountsChanged`, the event the Connectors list refreshes on. Publish it in `oauth_client.rs::complete()` after a successful finish. (Naki-2's #21822 is the owner and confirmed it; their fix also covers `set_status` and Google grant and ungrant, see 3.1a.)
2. **Settings values are already live.** What is stale in Settings is a short list of panels that fetch once on mount: Hidden agents, Pinned senders, Agent site grants, Devices, Browser profiles. Each already has, or can cheaply get, an event; they need a subscription, not a new system.
3. **Do not add a signal per element, and do not add a new framework.** The app already has the right pattern in three places (accounts cache, Kanban, MCP). Copy it as one small helper, `createDomainStore`, and move the stale fetch-once views onto it.
4. **Add a gate that only shrinks:** a view file that fetches over RPC on mount and has no live source in it must be on a baseline list or say why it is fetch-once. Today's baseline is 25 files, most of them modals and forms where fetch-once is right.

## 2. How sure each claim is

Five read-only passes covered Connectors and accounts, Settings, Vault / Memory / Kanban and the other data views, Swarm / Audit / the agent pane, and the event infrastructure. I re-read the code for every claim that leads to a fix and marked it **verified**. A claim from a pass that I did not re-read is marked **reported**. Two claims from the infrastructure pass were wrong when I checked (it said `agentskills:changed` and `remoteschange` are never published; they are, `agent_handlers/skills.rs:75` and `remote/remotes.rs:215`), so reported items are leads, not facts.

## 3. Findings, ranked by how much a person relies on the view

| # | View | Verdict | Cause | Status |
|---|---|---|---|---|
| 1 | **Connectors, Accounts list** | stale for an account connected by an agent or another window | the service-OAuth path publishes no event (below) | verified |
| 2 | Connectors, MCP servers | live | `onMuxEvent(McpChanged)` (`mcp-model.ts:137`); srv publishes at `app_api/mcp.rs:138` and others; `mcp-observed.ts` refetches after a reconnect | reported |
| 3 | Connectors, account availability | live | event plus a 30 s poll (`account-availability.ts:39`) | reported |
| 4 | **Settings, every value section** | live, hand edits included | `config_watcher_fs.rs` reload, `Config` broadcast, `setFullConfigAtom` (`global.ts:244`), `settingsAtom` memo, sections read `settingsAtom()` | verified |
| 5 | Settings, Hidden agents | stale until remount | `onMount(load)` only (`hidden-agents.tsx:48`); `agents:changed` is published on rename and seat, but the hidden flag's setter (`app_api/viewer.rs::register_agent_hidden`) publishes nothing, so hiding needs an srv wake-up as well as a subscription | verified |
| 6 | Settings, Pinned senders | stale | `onMount(load)` only (`pinned-senders.tsx:60`) | verified |
| 7 | Settings, Agent site grants | stale | `onMount(load)` only (`agent-site-grants.tsx:56`); an agent can add a grant | verified |
| 8 | Settings, Devices | partly | `onMount(load)` plus a re-load when `viewerPairedAtom` changes (`devices-section.tsx:102`) | verified |
| 9 | Settings, Browser profiles | stale | loaded once on mount (`browser-section.tsx:228`) | verified |
| 10 | Vault entries | live | `onMuxEvent(Vault)`; srv publishes on create, update, delete, restore, import, generate, revoke (`vault_handlers.rs`) | reported |
| 11 | Vault, "kept secrets" | stale | loaded once; no event when the keychain changes | reported |
| 12 | Memory (personal, global, skills, bundles) | live | `agent:memory:changed:<id>` and the others, 250 ms debounce | reported |
| 13 | Kanban | live | `WorkQueueChanged` with a 150 ms debounce plus a 15 s poll while visible (`kanban-held.tsx:81-85`) | verified |
| 14 | History (agents) | stale | no subscription; only a Refresh button | reported |
| 15 | Shortcuts | stale | loaded once | reported |
| 16 | Drone runs | partly | run events live, the list fetched once; srv publishes `drones:changed` (`drone_handlers.rs:133`) | reported |
| 17 | Swarm, this instance | live | `onMuxEvent` plus a poll while visible | reported |
| 18 | Swarm, other channels and LAN peers | poll only | the host-global registry has no event; polled about every 5 s while visible | reported |
| 19 | Audit | poll only, 5 s while at the newest page | no audit event exists (see the Audit spec, #21665) | verified |
| 20 | Status bar config item, system status | live | read `fullConfigAtom` | reported |
| 21 | Agent pane model and account pickers | unclear | one pass says "fetched once at launch"; the account picker imports the shared account cache (`agent-model.ts:28`), so it follows the accounts event; the model picker is not checked | not checked |

### 3.1 Connectors: the cause of the symptom (verified in code, not reproduced live)

- The Accounts list reads a module-level cache, `identity-model.ts`. It is refreshed by one app-lifetime `onMuxEvent(IdentityAccountsChanged)` (line 434) and by `refreshAccountCache()` wherever the view itself changes something.
- srv publishes that event from: the CLI sign-in path (`identity_auth_persist.rs:101`), the identity RPC handlers (upsert, key verify, delete: `agent_handlers/identity.rs:459, 601, 874`), the agent-binding API (`app_api/identity.rs:312`), `credential_watch.rs:91` and `resolver/inject.rs:924`.
- It does **not** publish from the service-OAuth path. `identity/oauth_client.rs` `finish()` saves the account (`identity_upsert_with_mirror`, line 806) and `complete()` calls `after_sign_in` (line 843); neither publishes. Google and GitHub connects use this path, and so does the agent-driven Google connect (`google_connect.rs` drives `oauth_client::manager()`).
- The only refresh on that path is the panel that started the flow: `OAuthConnectPanel.finishSuccess` calls `refreshAccountCache()` (line 101). An account connected by an agent, or from another window, reaches the list only when something else refreshes the cache. Remounting does: `IdentityViewModel` calls `refreshAccountCache()` when it is built (`identity-model.ts:650`). That is "switch tabs and back".

### 3.1a Naki-2's findings from the fix (#21822, folded in 2026-11-03)

Naki-2 read the same code and confirms 3.1 (the service-OAuth commit in `oauth_client::finish` never published; only `OAuthConnectPanel` refreshed by hand, in the window that started the flow). They found three more gaps of the same kind, on the same path:

- `oauth_tokens::set_status` (a refresh refused turns the account to `needs_reauth`, and back to `valid`) changes the account and announces nothing.
- Google grant and ungrant change what an agent may use, and announce nothing.
- `GoogleServicesSection` fetched its agent and grant lists once per mount and had no subscription.

Their fix, on branch `naki-2/connectors-live-update`: `crates/srv/src/identity/accounts_changed.rs` (the broker installed at startup, an `announce()`) called at those four commit points, and `GoogleServicesSection` re-fetches on the event. That is the "publish where it saves" rule of §5 applied, and the starting list for the second gate in §6.

Not fixed, and not on the account event: the Google call-log lines (Google tool calls by agents) do not push to the open details panel. One event per call would make every card re-fetch, so it needs its own event scoped to the account (a row in §3 for the build).

### 3.2 What the unverified lines suggest

Most stale views are **fetch once on mount, with an event already available** (`agents:changed`, `drones:changed`, `config`). That is why one cheap pattern covers most of them.

## 4. What srv publishes

srv declares 112 events (`backend/mps_events.rs`) and, by a rough scan, every one but `SingletonClaim` has a publish site. The gaps are therefore paths that change data without calling the publish, as in 3.1, not missing event types. Payloads are mostly bare wake-ups (`()`), a few carry the record or a version (`waveobj:update`).

Domains checked and their publish status: accounts (**gap: service OAuth**), MCP (yes), Vault (yes), Memory (yes), Kanban / work queue (yes), agent definitions (yes), Config (yes), Skills (yes), Remotes (yes), Drones (yes). Not checked: History, Shortcuts, Browser profiles, site grants, pinned senders, devices (the frontend has no subscription; whether srv would publish is the first thing the build checks).

**Reconnect.** After the websocket returns, `mps.ts` re-subscribes every event type (`mpsReconnectHandler`), and `ws:eventsdropped` re-fetches MuxObjects (`resync.ts`). Neither re-fetches a domain store: an event missed while disconnected is lost until the view remounts. Where this is handled, it is by hand: `mcp-observed.ts:123` re-fetches after a reconnect, with a comment saying why, and `launcher-event-store.ts` re-syncs every 30 s.

## 5. The pattern: one subscription and one store per domain

In Solid a signal is not a cost per element: a store holds the domain once, and a view reading `store.accounts[id].name` re-renders only what reads that field. So the rule is one store per domain, fed once, read by many.

The three views that already do this well, to copy:

- **Accounts cache** (`identity-model.ts`): module-level cache, `primeAccountCache()` at start, a single app-lifetime event subscription guarded against doubling, `subscribeAccountChanges` for readers.
- **Kanban** (`kanban-held.tsx`): the event with a 150 ms debounce, plus a 15 s safety poll while visible.
- **MCP** (`mcp-observed.ts`): the event plus a re-fetch after a reconnect.

`createDomainStore` is the three combined, about 40 lines in `frontend/app/store/`:

```ts
createDomainStore({ name, events: [WpsEvent.X], load: () => RpcApi.ListXCommand(...), safetyPollMs? })
// -> { items(), loading(), refresh(), stale() }
```

It subscribes once for the app's life, debounces a burst to one `load` (150 ms), re-runs `load` after a reconnect, optionally polls while a view reads it, and exposes the data as a Solid store. A view that today does `onMount(load)` replaces it with `const x = useDomain(xStore)`. Where an event carries the changed record or a version, `load` can apply it instead of fetching (an optional `apply(payload)`); the default is a re-fetch, which is correct and simple because the events are mostly bare wake-ups.

On the srv side the rule is: **the function that changes the data publishes**, not its caller. 3.1 is the counter-example: `complete()` saved the account and left the announcement to the callers.

## 6. The gate

Two cheap checks, both scripts like the existing ratchets:

1. **Fetch-once view check** (`scripts/check-reactive-coverage.mjs`, with a baseline that only shrinks): fails on a file under `frontend/app/view/` that calls an `RpcApi.*Command` from `onMount` or `createResource` and has no live source in the same file (`onMuxEvent`, a domain store, `pollWhileVisible`, `setInterval`). A deliberate fetch-once view (a modal, a form, a preview) adds `// reactive: fetch-once, <why>`. A prototype run (a regex over the files, so a rough count) found 36 files that fetch on mount and 24 with no live source in the file: Hidden agents, Pinned senders, Agent site grants, Devices, the fork-slug migration, the Notifications and Recording sections, Drone, Toolchain, the editor view and History pagination, and about a dozen modals, forms and previews that would take the marker. (Browser profiles is stale too but its file has another subscription call, so the regex misses it: the gate is a floor, not a full audit.)
2. **Publish-where-it-saves check** (a test in `crates/srv`): a list of the functions that write an account, and a test that each calls the announcement (or is in a list with the reason), so a new write path cannot skip it the way `oauth_client::complete` did. Starting list: `identity_upsert_with_mirror` call sites. Rust: tested on charlie.

A gate cannot tell a view that *should* be live from one that is not; the marker makes that a decision someone writes down.

## 7. Build, in three PRs

1. **Connectors (now, Naki-2's #21822):** publish in `complete()`, with a test that a finished OAuth connect announces the change.
2. **`createDomainStore` and the stale Settings panels:** the helper, Hidden agents (an srv event for the hidden flag, which publishes none today), Pinned senders, Agent site grants, Devices and Browser profiles moved onto it (adding an event on srv for the ones with none), tests with a fake event.
3. **The gate:** the fetch-once check with its baseline, the publish-where-it-saves test, and the other stale views (History, Shortcuts, Drone, Vault kept secrets) as the baseline shrinks.

## 8. Open questions for the operator

1. **Settings panels that need a new srv event** (site grants, pinned senders, browser profiles, devices): recommend adding one `settings-data:changed` wake-up for all four, not four events. Alternative: poll while the panel is open.
2. **Safety poll for the Settings and Connectors stores:** recommend 30 s while visible, as Kanban does at 15 s, so a lost event heals itself. Alternative: events only.
3. **Gate strictness:** recommend the baseline plus the `reactive: fetch-once` marker. Alternative: warn only for a month.

## 9. Not done

No live check; Naki-2's findings (3.1a) are theirs as told to me, their PR is not yet read; the model picker, History, Shortcuts, Browser profiles, site grants, pinned senders and devices were read by the research passes and re-read only where marked verified; cross-machine staleness (a change on another computer) was not audited beyond the Swarm's polling.

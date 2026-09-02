# Spec: reactive updates across the Armory

**Date:** 2026-09-02
**Status:** Proposed
**Motivated by:** direct question, then a direct request to broaden it —
*"if you write a memory, will I see the update in Manoz in the armory? we
want it reactive, so the stat shows up immediately"*, followed by *"build the
reactive updates, across the entire armory in fact."*

## Scope: an audit first, because most of this already exists

Before designing anything Armory-wide, the actual question was: **which of
Armory's five tabs (Accounts, Memory [Global + Personal], Skills, MCP
Servers, ABF) are already reactive, and which aren't?** Grepping the backend
for every `WaveEvent` it publishes, then checking which frontend view-models
actually subscribe to each, found a much narrower gap than "build reactivity
from scratch everywhere":

| Area | Backend event | Already wired? |
|---|---|---|
| **Accounts** | `identityaccounts:changed` | **Yes.** `IdentityViewModel` (`identity-model.ts`) installs an app-lifetime subscription that explicitly calls out "Armory Accounts tab" as one of its consumers — `AccountsManager` reuses this exact model. Nothing to build. |
| Per-agent MCP/Skill *bindings* (inside a bundle) | `mcp:changed` / `skills:changed` | **Yes.** `bundle-mcp-model.ts` / `bundle-skill-model.ts` both already subscribe. |
| **Global Memory** (brain) | `memories:changed` | **No.** `global-brain-model.ts` fetches once, no subscription. |
| **ABF** (bundle list) | `memories:changed` | **No.** `memory-model.ts` (`MemoryViewModel`) fetches once, no subscription. |
| **MCP Servers** (standalone Armory tab) | `mcp:changed` | **No.** `mcp-model.ts` fetches once — the event exists and is already proven-working (the per-bundle model above consumes it), it's just never subscribed to here. |
| **Skills** (standalone Armory tab) | `skills:changed` | **No.** `skill-model.ts`, same gap as MCP Servers. |
| **Personal (native) Memory** | *(none exists)* | **No — and there's no event to subscribe to yet.** This is the one area needing real backend work; everything below this table already existed before this spec. |

**Consequence for scope:** four of the five gaps are a **three-line fix
each** — add the same `waveEventSubscribe({ eventType, handler: () =>
void this.refresh() })` pattern the already-working models use, in each of
the four models that are missing it. No backend changes needed for those
four; the events they need already exist and are already proven correct by
their existing consumers. The fifth (Personal/native Memory) is the one
piece that needed original design, below.

## Personal (native) Memory — current behavior and the fix

### Current behavior (answered directly, then fixed here)

**No — today it is not reactive.** This was an explicit, documented tradeoff
in `SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md`:

> Consequence, accepted: a count does **not** refresh when an agent's memory
> files change while you sit on the grid... refetching everything on
> unrelated agent edits is the wrong trade.

The grid's count-fetch effect (`native-memory-manager.tsx`) is keyed on the
**set of agent IDs**, not on memory content — it fetches once per agent when
that agent first appears in the set, and never again unless the whole
component remounts. Writing a memory — via the `MemoryWrite` MCP tool, the
Armory's own detail-view editor, or an agent's own filesystem tools writing
directly — does not touch that key, so the card's count sits stale until you
leave and re-enter the tab.

### Design

#### Backend: publish on every recorded version — an event already half-shipped

Mid-implementation, `git log -S` on the planned event name turned up that
**two of the five call sites already published something**: `app_api/mod.rs`'s
`memory_write_impl`/`memory_revert_impl` (commit `bcc478dc`, 2026-08-20, a
different agent) already called
`state.broker.publish(WaveEvent { event: format!("agent:memory:changed:{agent_id}"), ... })`
— but nothing anywhere subscribed to it, so it was a no-op in practice. This
changed the plan: rather than introduce a second, competing convention
(`WaveEvent.scopes`, this spec's original design) alongside an
already-shipped one, every new call site adopts the *existing* convention —
**the agent id baked directly into the event NAME**
(`"agent:memory:changed:{agent_id}"`), not `WaveEvent.scopes`.

One correction made to the existing calls in the same pass: they keyed the
event by the raw `agent_id` **slug** parameter (App API's own calling
convention), not the resolved canonical UUID `version_agent_id` the function
already computes for version storage — the exact slug/UUID keyspace split
`memory_write_impl`'s own doc comment already warns about for versions,
just not previously applied to this event. A frontend subscriber only ever
has `AgentDefinition.id` (the UUID), so a slug-keyed event would have gone
unheard even once a subscriber existed. Fixed to `version_agent_id` at both
existing call sites — see `write_publishes_using_the_resolved_uuid_not_the_raw_slug`
for the regression test.

Every *production* code path that records a new native-memory version funnels
through exactly two `Store` methods
(`agentmux-srv/src/backend/storage/agent_native_memory_versions.rs`):
`agent_native_memory_version_insert` and, for out-of-band writes,
`agent_native_memory_version_insert_if_changed`. Five real call sites, no
more — publish added/fixed at all five:

| Call site | Surface | Status before this spec |
|---|---|---|
| `app_api/mod.rs` `memory_write_impl` | App API write (`MemoryWrite` MCP tool) | Already published — re-keyed to the canonical UUID |
| `app_api/mod.rs` `memory_revert_impl` | App API revert | Already published — re-keyed to the canonical UUID |
| `native_memory_handlers.rs` `write_file` | WS RPC (Armory UI) | New |
| `native_memory_handlers.rs` `revert` | WS RPC (Armory UI) | New |
| `native_memory_drift.rs` fast path + `sweep_one_agent_dir` | Out-of-band write detection | New |

(The storage layer itself has no access to the WPS `broker` — a deliberate
layering boundary — so the publish happens at each call site, immediately
after a successful insert, not inside the storage method.)

`data` is `None` at every call site, matching the pre-existing convention —
the event name alone (which agent) is everything a "refetch this agent"
handler needs; there was no existing `filename` payload to preserve, and
none was added.

For `agent_native_memory_version_insert_if_changed` (the drift-detector call):
only publish when it actually returns a new version — its whole point is
"no-op if content is unchanged" (reconciliation sweep re-hashing every file
every 30s), and publishing on every no-op sweep tick would mean an event
firing for every watched agent every 30 seconds regardless of whether
anything happened, which defeats the point of an event-driven design (may as
well have kept polling). Also fixed a small pre-existing log inaccuracy while
touching this exact branch in the fast path: the old `if let Err(e) = ... {}
else {}` logged "recorded an out-of-band write" even on the Ok(false)
no-op case — restructured to a full `match` (needed anyway to gate the new
publish), which incidentally fixed that too.

#### Frontend: one subscription per agent, re-registered as the agent set changes

Because the event name itself encodes the agent id, `NativeMemoryManager.tsx`
subscribes to `"agent:memory:changed:{id}"` **once per agent currently in the
grid** (`waveEventSubscribe` accepts a variadic list of subscriptions in one
call, one combined unsubscribe), re-registering whenever the agent SET
changes — same `agentIdsKey` dependency and rationale the existing count-fetch
effect already uses (a brand-new-but-equivalent `agents()` array must not
tear down and rebuild every subscription). On receipt:

- **Grid view:** refetch that one agent's count via the same
  `RpcApi.NativeMemoryListCommand` call the existing count-fetch effect
  already makes, updating just `counts()[agent_id]` — not a batch refetch of
  every card. Reuses the existing per-agent stale-response handling (a
  response is only stale if its agent left the map).
- **Detail view:** if the event's `agent_id` matches `selectedAgent()`,
  refetch the file list too (same `NativeMemoryListCommand` call the
  `selectedAgent()`-keyed effect already makes) — so a live write to the
  agent you're actively viewing shows up without navigating away and back.
  A change to a filename you don't currently have selected does not clear
  `selectedFilename()` — only a *file-list* refresh, never forcing you out
  of whatever you're looking at.

**Debounce, per agent ID, ~250ms:** a burst of rapid writes to the same
agent (e.g. an agent scripting several `MemoryWrite` calls in a loop) should
coalesce into one refetch, not one RPC round-trip per write. Keyed per
agent ID so a burst on agent A never delays agent B's own refresh.

#### Known gap — not solved by this design, and why it's acceptable

The fs-watch fast path only watches **known** agents' memory directories
(`list_all_memory_targets` — agents already in `db_agents` or the live
registry). An agent's very first write, before any watch is established for
it, is caught by the WS RPC / App API call sites directly (those publish
regardless of watch state) — so this gap only affects an out-of-band write
(bypassing both RPC surfaces) to a *brand-new, not-yet-tracked* agent, which
falls back to the 30s reconciliation sweep's own latency. This mirrors
`native_memory_drift.rs`'s own documented stance (its module doc comment:
*"Neither [layer] promises zero data loss... precision, honestly bounded"*)
— this spec inherits that same bound rather than trying to close it, since
closing it would mean watching every possible future memory directory
speculatively.

### Tests (Personal Memory)

- Backend: each of the five call sites publishes
  `"agent:memory:changed:{agent_id}"` — keyed by the resolved canonical UUID,
  never the raw slug — on success; `insert_if_changed` does **not** publish
  when the content was unchanged (no-op sweep tick).
- Frontend: an `"agent:memory:changed:{id}"` event for an agent already in the
  grid triggers exactly one `NativeMemoryListCommand` call for that agent
  and updates only that card's count — siblings' `counts()` entries are
  untouched (same "one card's fetch doesn't touch its siblings" invariant
  #2917 already established, extended to the new refresh path).
- An event for an agent whose card resolved with an error, or is still
  loading, still triggers a refetch (recovery path — an agent that errored
  once should get to try again on the next write, not stay permanently
  errored until a full remount).
- An event while the detail view is open for a DIFFERENT agent does not
  touch that view's file list.
- An event while the detail view is open for the SAME agent refreshes the
  file list without clearing `selectedFilename()` (unless the selected file
  was actually the one removed).
- Rapid repeated events for the same agent within the debounce window
  produce exactly one refetch, not one per event.

### Non-goals (Personal Memory)

- **No change to the initial per-agent-set count-fetch effect** — this adds
  a second, event-triggered refresh path alongside it, not a replacement.
- **No toast/notification UI** for "agent X's memory changed" — `data` is
  `None` at every call site (matching the pre-existing App API convention);
  the event name (which agent) is all a "refetch" handler needs, and no
  `filename` payload was added since nothing needs it yet.
- **No retroactive backfill** for the known gap above (brand-new,
  not-yet-watched agent + purely out-of-band write) — see that section's own
  rationale.
- **No polling fallback.** If the WPS connection is down, counts simply stay
  as stale as they are today until reconnect + remount — same failure mode
  that already exists for `agents:changed`-driven refreshes elsewhere in the
  app; not a new risk this feature introduces.

## The other four areas — wire the existing event, nothing more

Each of these already has a working, proven `waveEventSubscribe` consumer
elsewhere in the codebase for the exact same event (the per-bundle
MCP/Skill models for `mcp:changed`/`skills:changed`; `IdentityViewModel`
for `identityaccounts:changed`, not applicable here since Accounts needs no
change). The fix in each case is the identical pattern
`bundle-mcp-model.ts` already uses — add to the constructor:

```ts
private unsubChanged: () => void;
constructor(...) {
    ...
    void this.refresh();
    this.unsubChanged = waveEventSubscribe({
        eventType: "<event>",
        handler: () => void this.refresh(),
    });
}
```

plus disposing `unsubChanged()` wherever the model's existing teardown path
is (`dispose()` for a class ViewModel, `onCleanup` for a function
component) — mirroring whatever that file already does for its other
cleanup, not inventing a new lifecycle pattern per file.

| File | Event | Refresh method already exists? |
|---|---|---|
| `frontend/app/view/brain/global-brain-model.ts` | `memories:changed` | Yes — `refresh()` |
| `frontend/app/view/memory/memory-model.ts` (`MemoryViewModel`, ABF) | `memories:changed` | Yes — `refresh()` |
| `frontend/app/view/mcp/mcp-model.ts` | `mcp:changed` | Yes — `refresh()` |
| `frontend/app/view/skill/skill-model.ts` | `skills:changed` | Yes — `refresh()` |

No backend changes for any of these four — `memories:changed`, `mcp:changed`,
and `skills:changed` are already published correctly (proven by their
existing consumers) and already carry everything the fetch-based `refresh()`
methods need (nothing — `refresh()` re-fetches wholesale from the RPC, same
"unscoped, refetch on any change" pattern `bundle-mcp-model.ts` already
uses, not a targeted single-item update the way Personal Memory's design
above needs one).

**Why `memories:changed` for BOTH Global Memory and ABF, and is that a
problem?** No — `memories:changed` is the existing umbrella event for the
whole bundle/memory backend area (published from both
`agent_handlers/memory.rs` and `app_api/bundle.rs`), not scoped per-tab. A
change relevant to one tab firing a refresh in the other is a wasted RPC
call at worst (both `refresh()` methods are cheap, idempotent full-list
fetches), never a correctness issue — same as how `agents:changed` already
fans out to several unrelated-looking consumers (`AgentPicker`,
`HiddenTemplatesSection`, etc.) today.

### Tests (the other four areas)

- For each of the four: an event of the matching type triggers exactly one
  additional `refresh()`-equivalent RPC call; an event of a DIFFERENT
  event type does not.
- The subscription is torn down on unmount/dispose (no handler fires after
  cleanup — mirrors the existing `bundle-mcp-model.test.ts`/
  `bundle-skill-model.test.ts` pattern of asserting
  `hub.handlers.has(eventType)` flips to `false` after disposal).

### Non-goals (the other four areas)

- **No backend changes** — see above.
- **No debounce** added to these four — unlike Personal Memory's per-agent
  targeted refresh, these are already-cheap wholesale list refetches using
  the exact pattern proven safe by their existing sibling consumers; adding
  new machinery here would be inventing a problem that isn't observed
  anywhere else this pattern is already used.
- **No change to Accounts** — already fully reactive, confirmed by the audit
  above; nothing to do.

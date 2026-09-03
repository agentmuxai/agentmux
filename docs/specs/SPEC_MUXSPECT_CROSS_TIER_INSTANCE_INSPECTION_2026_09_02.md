# muxspect Phase 2: cross-tier instance inspection (same-host channels + LAN)

**Status:** proposed
**Date:** 2026-09-02
**Author:** Manoz@Area54
**Motivating incident:** live-debugging a `task dev` build on branch
`manoz/fix-activity-dock-subagent-backfill-flash` required inspecting a
*second*, freshly-launched AgentMux instance's live state (agent Lzop's
subagent backfill/reconciliation). `muxspect` — the tool built exactly for
this — could not reach it (Phase 1, current-instance-only). The workaround
was manual: `netstat` to find the second instance's CDP port, a hand-rolled
CDP `MutationObserver` script, and `grep`-ing that instance's raw srv log
file for `reconcile_stale_subagents` lines. That workaround found the real
bug (§"Motivating incident" detail below), but it took a dozen ad-hoc steps
a purpose-built tool should collapse into one command.

---

## 0. This is a planned extension, not a new idea — read this first

`docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` §7 already
named "Phase 2: cross-instance query" as the next step after Phase 1 shipped.
`docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`
§3 point 3 already proposed the specific missing primitive this spec builds
(a real "list live instances" command cross-referencing actual processes
against disk). `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
already built and shipped the *transport, trust, and consent* pattern this
spec reuses (jekt-riding request/response, per-agent visibility setting,
TOFU-pinned LAN identity) — for transcripts specifically — and its own
Non-goals section left the door open in so many words: *"A future second
remote-read need should get its own narrow protocol rather than generalizing
this one prematurely."* This spec is that second need.

**Nothing here invents a new trust or transport model.** Every mechanism
below is an application of infrastructure that already exists, is reviewed,
and is (mostly) already shipped. The new work is: a same-host instance
registry (doesn't exist yet, in any form), a generalized forward path for
muxspect's existing read routes (today only `describe`/`transcript` forward
cross-channel; nothing forwards to LAN except one narrow agent-lookup), and
one new jekt content type for the LAN hop, built the same way
`transcript_request` was.

---

## 1. Scope

### In scope
- **Phase 2a — same-host, cross-channel.** A `muxspect instances` command
  listing every live AgentMux channel/instance on this machine (channel
  name, version, PID, port, agent count, uptime) — ground-truthed against
  actual running processes, not just disk contents. Generalize the existing
  single-purpose cross-channel forward (used today only by `describe` and
  the transcript route) into a general-purpose forward any read-only
  muxspect route can use, addressed by channel name.
- **Phase 2b — LAN.** A new `muxspect_request`/`muxspect_response` jekt
  content-type pair, riding the same signed jekt envelope and LAN Ed25519
  identity (TOFU-pinned) the existing `transcript_request` pair uses,
  carrying `{route, params}` and returning the same JSON shape the local
  HTTP handler would produce. Gated by the same visibility-setting +
  trust-grant infrastructure already built for transcripts (§4.3 — this is
  the one point needing explicit confirmation, not a default I'm assuming).

### Out of scope (explicitly, matching this codebase's own established sequencing)
- **WAN.** Every prior phase in this family (transcript visibility, jekt
  trust hardening) sequenced LAN before WAN and left WAN for a dedicated
  follow-up once LAN ships and settles. Same here.
- **Any mutating route.** `muxspect dock/clear` is the one mutating route
  muxspect exposes today. It stays local-instance-only in this spec — no
  remote tier gets write access to another instance's live state. If a
  genuine need for remote dock-clear shows up later, it gets its own spec
  and its own (almost certainly stricter) trust rule, the same way this
  spec is treating instance inspection as its own narrow thing rather than
  folding it into transcript visibility.
- **A general remote-RPC framework.** `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`
  §4 already scoped that as a separate, much larger, explicitly-sequenced-last
  piece of work (gated on WAN per-agent authorization actually being
  enforced, which it isn't yet). This spec's `muxspect_request` is
  deliberately narrow — one route name + one params object, allowlisted to
  muxspect's own existing read handlers — not a general verb space.
- **Agent-less-channel visibility on the LAN tier.** Phase 2a's same-host
  registry closes the "channel with no registered agents is invisible"
  gap locally (§3.1). Doing the same over LAN is a natural follow-on but
  adds discovery-protocol surface (`lan_discovery.rs` TXT records would
  need a new field) this spec doesn't touch — LAN in this spec only reaches
  a channel that already has ≥1 registered agent, same limitation
  `muxspect find`'s LAN tier has today.

---

## 2. Phase 2a: same-host instance registry

### 2.1 The gap, precisely

Two things exist today and neither is "list every live instance":

- `agentmux-launcher/src/other_instances.rs`'s `enumerate_sibling_instances()`
  walks `<channels_root>/*/versions/*/` and probes each sibling's
  single-instance pipe for liveness — but this is launcher-internal,
  diagnostic-log-only (`log_older_running_instances()`), never exposed to
  any RPC caller.
- The shared reactive registry (`agentmux-srv/src/backend/reactive/registry.rs`,
  `~/.agentmux/shared/agents/reactive/<agent>/<channel>.json`) is
  **agent-keyed**: a channel with zero registered agents (all shell/terminal
  panes, no agent CLI) has no entry and is invisible to every
  `muxspect find`/`conversations`/`verify-sender` cross-channel tier today.

### 2.2 Design: instance-level heartbeat, same pattern as the agent registry

Add `~/.agentmux/shared/instances/<channel>.json`, one file per running srv
instance, written on startup and refreshed on a short interval (reuse
whatever interval the existing memory-heartbeat logging already uses as a
starting point — this needs an actual number chosen during implementation,
not guessed here). Shape:

```json
{
  "channel": "manoz-fix-activity-dock-subagent-backfill-flash",
  "version": "0.55.32",
  "pid": 13120,
  "local_url": "http://127.0.0.1:60304",
  "auth_key": "...",
  "started_at": 1788399530000,
  "updated_at": 1788399605000,
  "agent_count": 1
}
```

This is the exact same "well-known directory + per-owner JSON file, `pid`/
`updated_at` freshness, `auth_key` embedded for forwarding" shape the agent
registry already uses (`registry.rs:22-62`) — same eviction logic
(`cleanup_stale_shared`'s `pid_alive()` check) reused, not reinvented.
`agent_count` is read from the existing `ProcessBroker`/agent-registration
count at write time — cheap, already computed for other purposes.

**Why a new file instead of extending the agent registry to allow zero-agent
entries:** the agent registry's file-per-`(agent, channel)` shape means an
agent-less channel has literally nothing to key a file on without changing
that shape for every existing reader. A parallel, channel-keyed file is the
smaller, more legible change, and cheap to reconcile (each is a straight
`pid`-liveness check).

### 2.3 New route + CLI command

- `GET /api/v1/muxspect/instances` (`muxspect_handlers.rs`, new handler,
  same auth middleware as every other route in the file) — reads
  `~/.agentmux/shared/instances/*.json`, does the same `pid_alive()` sweep
  the agent registry does (evict stale entries, don't just trust the file),
  returns the live set.
- `muxspect instances` (`muxspect.mjs`, new subcommand, same
  `apiGet`/render pattern as `muxspect list`) — table: channel, version,
  pid, agents, uptime.

### 2.4 Generalizing the forward path

Today, exactly two routes (`describe`, the transcript route) forward a
request to a specific OTHER channel, and only when reached via `find`'s own
match logic — there's no way to say "run `dock` against channel X"
directly. Add an optional `?channel=<name>` param, handled once, centrally
(not per-route): if present, resolve `<name>` in the new instance registry,
forward the request verbatim to that instance's `local_url` with its
`auth_key` (same bounded-timeout forward pattern already in
`muxspect_handlers.rs:931`'s `CROSS_CHANNEL_PREVIEW_TIMEOUT_MS`), return its
response. This makes `list`, `describe`, `dock`, `background-tasks`,
`verify-sender`, `layout` all immediately cross-channel-capable with one
shared code path, instead of each route needing its own forward logic the
way `describe`/transcript do today.

`dock/clear` explicitly does NOT get this treatment (§1, out of scope).

### 2.5 Trust for Phase 2a

**None needed beyond what already exists.** Same host, same OS user, same
`~/.agentmux/shared/` directory every other cross-channel mechanism in this
codebase already trusts implicitly (the agent registry's `auth_key` embedding
already assumes this). No jekt tier is involved — this is a same-machine,
same-privilege-level RPC (`agentmux-mcp`, the CLI, and every existing
cross-channel forward already work this way). No new CLAUDE.md jekt-rules
change needed.

---

## 3. Phase 2b: LAN

### 3.1 Design: `muxspect_request` / `muxspect_response`, riding the jekt envelope

Directly mirrors `transcript_request`/`transcript_response`
(`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`,
`agentmux-common::transcript_request`): a new payload type carried inside
the *same* signed jekt envelope (no new crypto — reuses per-agent LAN
Ed25519 signing + `db_lan_peer_pubkey_pins` TOFU pinning exactly as-is).

```
MuxspectRequest  { route: String, params: JsonValue }
MuxspectResponse { status: "ok" | "denied" | "error", body: JsonValue | null, reason: String | null }
```

`route` is allowlisted server-side to muxspect's own existing read-handler
names (`list`, `describe`, `find`, `dock`, `background-tasks`,
`verify-sender`, `conversations`, `layout`) — never a free-form path, never
`dock/clear`. This allowlist is the entire "RPC verb space" this spec adds;
it is not a general method registry the way
`SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` §4
scoped its (still-unbuilt, deliberately-deferred) general RPC layer to be.

The responding agent's own srv resolves the request against ITS OWN local
muxspect handlers (i.e. the receiving side literally calls the same
handler function `handle_muxspect_dock` etc. would call for a local HTTP
request) and returns the result inline in `MuxspectResponse`, rather than
the requester making a second hop — this avoids opening any new inbound
HTTP surface on the LAN beyond what jekt delivery already requires.

### 3.2 Discovery — no changes needed

`lan_discovery.rs`'s existing mDNS/UDP-broadcast discovery + the
`GET /agentmux/reactive/agent` agent-presence lookup already answers "which
LAN peer hosts agent X" and already carries that peer's LAN public key.
Nothing here needs a new discovery mechanism — `muxspect_request` just needs
an agent name to route through, exactly like `transcript_request` does.

### 3.3 The one real design question: what gates disclosure?

This needs explicit confirmation before implementation, not a default
assumed here. Two options:

**Option A — reuse `conversation_visibility` as-is.** The existing setting
(`private` / `trusted_peers` / `ask`) and its `db_conversation_trust_grants`
table already implement exactly the question this spec needs answered:
"should this agent disclose internal state to that peer?" `muxspect dock`/
`describe`/`background-tasks` are less sensitive than a full transcript
(structured metadata — process status, retry counts, activity summaries —
not conversation content) but are still real information disclosure (e.g.
`dock` reveals what an agent is actively doing right now). Reusing the same
setting keeps the user's mental model to one dial ("who can see into this
agent") instead of two.

**Option B — a new, separate `remote_inspection_visibility` setting.**
Keeps "can read my conversation" and "can query my process/activity state"
as independently tunable — a user might reasonably want peers to see "is
this agent alive and what's it doing" (operationally useful for fleet
management) without granting transcript access, or vice versa.

**Recommendation: Option B**, on the same three-state shape
(`private`/`trusted_peers`/`ask`) and the same `db_conversation_trust_grants`-
shaped table (parameterized by request kind rather than a second table), for
exactly the reason a fleet operator plausibly wants those two dials
independent — but this is a judgment call about user-facing behavior, not a
technical constraint, and should be confirmed with the repo owner before
implementation the same way `transcript_request`'s own tier rule needed
explicit confirmation (§3.4).

### 3.4 Tier rule — needs repo-owner sign-off, same as transcript_request did

Per `CLAUDE.md`'s jekt security rules: *"Any change to CLAUDE.md's jekt
rules table requires explicit, live repo-owner confirmation before it's
real"* — this isn't optional process, it's the documented lesson from the
PR #2536 incident. Proposed rule, symmetric to
`SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`:

- `muxspect_request` is always forced `TIER=sensitive` (it's a
  disclosure-shaped question, same reasoning as transcript_request).
- `ESCALATE=required` is NOT relaxed by a verified sender when the
  responding agent's own visibility setting (§3.3) is `ask`, or
  `trusted_peers` with a requester not on the allowlist — identity proof
  answers *who's asking*, not *whether this should be shown*, same
  rationale as the transcript rule.
- For `private` or `trusted_peers`-with-allowlisted-requester, auto-resolve
  per that mode's existing design intent (deny / approve respectively),
  same as transcript_request.

This should ship as its own dedicated
`SPEC_JEKT_MUXSPECT_REQUEST_TIER_RULES_2026_09_XX.md`, written and confirmed
the same way the transcript one was, not folded silently into this spec's
own authority.

### 3.5 What Phase 2b deliberately does NOT build

- No `muxspect conversation-requests`-style approve/deny CLI beyond what's
  strictly needed for `ask` mode — reuse whatever Phase B/C's own deferred
  UI work lands as, don't build a second one.
- No caching/offline answer — a LAN peer that's unreachable just times out
  (bounded, same posture as the existing `remote_fetch_required: true`
  liveness-only LAN tier `find`/`conversations` already have).
- No new mDNS/broadcast TXT fields — this rides jekt, not a new discovery
  payload.

---

## 4. Non-goals recap (mirrors the discipline of every prior spec in this family)

- Not a general remote-RPC framework (§1, §3.1).
- Not WAN (§1).
- Not a mutation path of any kind on any remote tier (§1, §2.4, §3.1).
- Not a new discovery mechanism for LAN (§3.2) or same-host (§2.2 reuses the
  established registry-file pattern, doesn't invent a new one).
- Not a redesign of the tier vocabulary, transport, or LAN identity/pinning
  model — all reused verbatim from already-shipped work.

## 5. Rollout sequencing

1. **2a first, alone.** No trust-model risk, immediately useful (closes the
   exact gap this session's live-debugging hit), and the generalized forward
   path built here is a prerequisite building block for 2b's response side
   anyway (the responding agent's own srv needs to be able to answer any
   allowlisted route locally regardless of which transport the request
   arrived over).
2. **2b's tier-rule spec, written and confirmed, before any 2b code.**
   Matches how `transcript_request`'s rule was handled — spec and
   confirmation first, implementation second, not concurrently.
3. **2b implementation**, once 1 and 2 are settled.

## 6. Testing

- 2a: `muxspect instances` against a real second `task dev` instance
  (exactly this session's own motivating scenario) — assert it lists both
  channels with correct pid/agent-count, and that a killed instance's stale
  file gets evicted (mirror the agent registry's own
  `cleanup_stale_shared` test pattern).
- 2a forward path: `muxspect dock --channel=<other>` against a real second
  instance with a genuinely different dock state than the caller's own —
  assert the correct instance's data comes back, not the caller's own.
- 2b: reuse the transcript_request test harness's shape
  (`SPEC_MUXSPECT_PHASE_B_POLICY_AND_TIER_ENFORCEMENT_2026_08_22.md`'s own
  tests are the template) — one test per visibility mode × allowlist state,
  plus a TOFU-pin mismatch test (a second key claiming an already-pinned
  agent name must be rejected, not silently accepted as a rotation).

## 7. Open questions for the repo owner

1. Option A vs. B in §3.3 — one shared visibility dial or two independent
   ones?
2. Confirm the §3.4 tier rule before it's written up as its own dedicated
   spec (per CLAUDE.md's own process requirement for jekt-rules changes).
3. Heartbeat refresh interval for §2.2 — not chosen here; needs an actual
   number picked against real overhead during implementation.

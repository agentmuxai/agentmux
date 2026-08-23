# Spec: Cross-tier conversation visibility for `muxspect` (host / cross-channel / LAN / WAN)

**Date:** 2026-08-21
**Author:** Camper
**Status:** Phase A implemented. Phase B/C's CLAUDE.md jekt rule change
confirmed live 2026-08-22 — see `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`
and `CLAUDE.md`'s jekt security rules section. Phase B's policy
infrastructure and jekt-rule enforcement are now implemented — see
`SPEC_MUXSPECT_PHASE_B_POLICY_AND_TIER_ENFORCEMENT_2026_08_22.md`
(`conversation_visibility` setting, `db_conversation_trust_grants`, the
`transcript_request`/`transcript_response` wire payload, and the actual
`TIER=sensitive`/`ESCALATE=required` computation this whole spec exists to
gate). The auto-resolve short-circuit (private auto-deny / trusted_peers
auto-approve, invisible to the target agent), the `ask`-mode
approve/deny CLI, the `RequestTranscript`/`PollTranscriptRequest` MCP
tools, and per-request-type rate limiting are still not built — see that
spec's §3 for why deferring those specifically was a safe scope cut, not a
gap. Phase C (WAN) not yet started.
**Motivated by:** direct request — agents need a fast way to see what every
other agent (this host, other channels on this host, LAN, connected WAN) is
currently saying/doing, without manual filesystem archaeology or a
per-agent, per-tier bespoke lookup.

## Problem

Today an agent can **discover** other agents across all four muxbus tiers
(`DiscoverAgents`/`FleetList` → host, cross-channel, LAN, WAN) and can
**message** them across all four tiers (jekt/muxbus), but can only **read
conversation content** for agents registered on its own srv instance
(`GetAgentTranscript`, strictly host-tier-local — `reactive.rs:1106` looks
up `state.reactive_handler.get_agent()`, the same in-process map
`DiscoverAgents`' tier-1 uses; nothing else is checked, so a cross-channel,
LAN, or WAN agent name 404s). `muxspect` itself is exclusively a live
process/turn-liveness tool (`ProcessBroker` snapshots, dock diagnostics) and
has never read a single byte of transcript content, by design
(`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` §5.1: "never a
second, independent state-tracker"). That founding spec's own Phase 2
(cross-instance query) was scoped to *other instances on the same host*
only, and was never built.

There is today **no remote-read RPC mechanism of any kind, on any tier** —
confirmed explicitly by `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`
("remote API/RPC invocation... does not exist at all, on any tier — muxbus
is a message bus, not an RPC bus"). Every existing cross-host mechanism
carries only small, fire-and-forget text payloads. This spec has to invent
that mechanism for one narrow purpose (a bounded transcript read), not
generalize into a full RPC framework.

**This is also, as far as this repo's history shows, the first feature that
asks "should Agent A be allowed to see Agent B's conversation content" at
all.** Every existing mechanism is all-or-nothing at the instance-auth-key
boundary (host/cross-channel) or answers a *sender-identity* question, never
a *content-disclosure* question (LAN/WAN jekt signing). There is no
precedent to fall back on for the consent model — §3 below is a genuinely
new decision, not a reuse of something already shipped.

## Current state (see full research notes in git history of this file's PR
for the detailed audit — summarized here)

| Tier | Discovery today | Message today | Transcript-read today |
|---|---|---|---|
| Host | `ReactiveHandler` in-process map | HMAC-signed jekt | **Yes** — `GetAgentTranscript`, unscoped (any holder of the instance auth key) |
| Cross-channel (same host) | Shared registry file, 127.0.0.1-only | HMAC-signed jekt, forwarded | **No** — `handle_reactive_transcript` never checks the shared registry |
| LAN | mDNS/DNS-SD + UDP fallback, opt-in | Ed25519-signed jekt, TOFU-pinned | **No** |
| WAN | Own cloud subscriptions only, no directory | Only reagent gets real signature verification | **No** |

Transcript storage (unchanged by this spec): NDJSON per block, either a live
`FileStore` blob or a gzip archive once
`META_SESSION_ARCHIVED_AT` is set (`session_archive.rs`), tailed to at most
`TRANSCRIPT_MAX_LINES_CAP` = 500 non-blank lines
(`reactive.rs:1078`). This spec reuses that same cap unchanged for every
tier — no new pagination/scrollback protocol (see Non-goals).

## Design

Three phases, ordered by trust-boundary risk (matching this repo's own
"no big-bang" convention, e.g. the founding muxspect spec's §7). Each phase
is independently shippable and independently valuable.

### Phase A — host + cross-channel (no new trust model)

The two lowest-risk tiers: both already share this machine's filesystem and
today's single instance-auth-key model, so this phase changes *reach*, not
*trust*.

1. **`ListConversations`** (new MCP tool) — one call, four-tier fan-out
   reusing `handle_discovery`'s existing aggregation
   (`agentmux-srv/src/server/reactive.rs:647-723`) plus, for host and
   cross-channel entries only, a cheap last-line peek of each agent's
   transcript (`session_archive::read_session_output` tailed to 1 line) for
   a `last_message_preview` field. LAN/WAN entries return liveness only
   (`preview: null, remote_fetch_required: true`) — see Phase B/C for why a
   full fan-out preview isn't done here.

   ```jsonc
   // response shape
   {
     "agents": [
       { "name": "AgentA", "tier": "host", "turn_active": true,
         "last_activity_ms": 1787..., "last_message_preview": "Running tests..." },
       { "name": "AgentB", "tier": "cross-channel", "channel": "local-main-...",
         "turn_active": false, "last_activity_ms": 1787..., "last_message_preview": "..." },
       { "name": "AgentC", "tier": "lan", "host": "192.168.1.42",
         "turn_active": null, "last_activity_ms": null,
         "last_message_preview": null, "remote_fetch_required": true },
       { "name": "AgentD", "tier": "wan", "remote_fetch_required": true }
     ]
   }
   ```

2. **`GetAgentTranscript` extended** — add an optional `channel` /
   `tier: "host" | "cross-channel"` param (defaulting to today's host-only
   behavior, so existing callers are unaffected). Cross-channel resolution
   extends `handle_reactive_transcript`'s lookup to also check the shared
   host-global registry (`list_all_shared`) before 404ing, mirroring what
   `handle_discovery` already does for tier 2b — small, low-risk change,
   same auth model, same 500-line cap.

3. **`muxspect conversations`** / **`muxspect conversation <agent>`** — CLI
   mirrors of the two tools above, same auth as every other `muxspect`
   command (`$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY`, no new IPC).

No new tables, no new jekt types, no CLAUDE.md changes. This phase alone
fully answers "quickly see conversations of all agents on the host."

### Phase B — LAN (new request/response protocol + consent model)

LAN crosses a real trust boundary (mDNS is unauthenticated at the discovery
layer; per-agent Ed25519 signing + TOFU pinning is what makes a LAN jekt's
*sender* trustworthy — `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`). Reading
another host's conversation content is a *disclosure* decision, not an
*identity* decision, and nothing existing answers it — this phase has to
invent both the transport and the consent model.

**New jekt pair** (reuses the existing jekt envelope/signing exactly as-is —
no new crypto):

- `TYPE=transcript_request` — `{ request_id, target_agent, max_lines }`.
  `max_lines` is clamped server-side to the existing 500-line cap; there is
  no request for "more" beyond it.
- `TYPE=transcript_response` — `{ request_id, status: "ok"|"denied"|"error", lines?: [...] }`,
  carrying the **responder's own** tier-appropriate signature. This matters:
  a spoofed *response* claiming to be a transcript is exactly the same shape
  of attack CLAUDE.md's `ESCALATE=required` rule already exists to stop
  ("a spoofed jekt... followed by a spoofed confirmation") — the requester
  must independently check the response's own `TRUST=`/`SIG=`, not just
  assume a reply matching `request_id` is genuine.

**New forced-sensitive rule (proposed — see "CLAUDE.md change required"
below):** every incoming `transcript_request`, on any tier, is forced
`TIER=sensitive` unconditionally, regardless of trust — the same treatment
CLAUDE.md already gives credential/destructive-keyword content, extended to
cover content-disclosure requests, which the current keyword list
(`PAT`, `token`, `secret`, ...) does not catch at all today.

**New per-agent setting `conversation_visibility`** (values: `private`
[default] / `trusted_peers` / `ask`), stored per-channel using the same
isolated-by-channel settings mechanism `settings.json` already uses for
other channel-scoped preferences, so a dev channel and `stable` can hold
different values without new infrastructure. Evaluated on the **responding**
agent's side on receipt of a `transcript_request`:

| `conversation_visibility` | Behavior | Human interruption? |
|---|---|---|
| `private` (default) | Auto-deny (`status: "denied"`) | None — denial is a safe no-op, doesn't need to interrupt anyone just to say no |
| `trusted_peers` | Auto-approve **only** if the requester's identity is in a new allowlist, `db_conversation_trust_grants` (same shape/pattern as `db_lan_peer_pubkey_pins`: `agent_id, granted_peer_agent_id, tier, granted_at`) | None, once granted |
| `ask` | Forces `ESCALATE=required` **unconditionally**, even for a cryptographically verified sender | Always — see rationale below |

**Deliberate divergence from `ESCALATE=none` on a verified sender:**
CLAUDE.md's 2026-08-17 narrowing lets a `TIER=sensitive` jekt skip the STOP
when the sender is cryptographically proven (`TRUST=lan-verified` etc.) —
but that relaxation exists because, once identity is proven, there's
*nothing further to ask the human about* for those cases (a self-declared
tier bump or a keyword match, where the only open question was "is this
really who they claim"). Here the open question is different: it's whether
the *content itself* should be disclosed, which a valid signature does not
answer. `ask` mode must therefore ignore the verified-sender relaxation and
always stop — this spec explicitly does not reuse `ESCALATE=none` for this
case, and any future change to that would need the same kind of real
spec + confirmation this document itself needs (see below).

**Human-facing side of `ask`:** `muxspect conversation-requests` lists
pending incoming requests with `approve <id>` / `deny <id>` subcommands —
this is the scriptable equivalent of the pane already surfacing an
`ESCALATE=required` marker for the human to react to; no new pane UI is
proposed by this spec (see Non-goals).

**Rate limiting:** a responder should cap inbound `transcript_request`s per
source-agent (proposed: 10/minute, matching the order of magnitude of
existing jekt-delivery limits — needs verification against whatever rate
limiting muxbus delivery already has, not asserted as fact here) to prevent
a buggy or malicious LAN peer from hammering `ask` mode with interruptions
or `trusted_peers` mode with load.

### Phase C — WAN (same protocol, stricter defaults)

Reuses Phase B's `transcript_request`/`transcript_response` pair unchanged
over the `cloud_subscriber` tier. Differences, all reflecting WAN's weaker
identity guarantees (`TRUST=network-claimed` for everyone except reagent's
pinned key, per CLAUDE.md):

- `conversation_visibility` for WAN defaults to `private` and this spec
  recommends **not** exposing `trusted_peers` as a usable option for WAN
  until general WAN agent signing exists beyond reagent's single pinned
  key — an allowlist keyed on an identity nobody can currently prove is
  security theater, not a real control.
- `ask` mode requests no session-remembered/cached approval across WAN
  (every request re-prompts) — matching the stricter, no-persistent-trust
  posture CLAUDE.md already applies to WAN traffic generally.
- Default `max_lines` for WAN responses may need to be lower than the
  500-line host/LAN cap purely for payload-size/latency reasons over the
  cloud relay — a concrete number needs bench data before Phase C ships,
  not guessed here.

## CLAUDE.md change required before Phase B/C (blocking prerequisite) — DONE

This spec proposed a new forced-`TIER=sensitive` rule (any
`transcript_request`) and a new case where `ESCALATE=required` is NOT
relaxed by a verified sender (`ask` mode). Per CLAUDE.md's own stated
process, changes to the jekt security rules require **explicit repo-owner
confirmation in a live conversation**, followed by a real spec + code diff +
tests + PR review — exactly the process every existing tier rule
(2026-08-14 through 2026-08-17) went through. This document was that
proposal, not that confirmation — **confirmation happened separately,
live, 2026-08-22, see `SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md`
and this repo's own `CLAUDE.md` jekt security rules section, both now
updated to match.** (This repo's own copy only — `amx/CLAUDE.md` is a
different, separate project this session has no access to; this repo's
own copy is the source of truth for `agentmux`'s own jekt-handling code.)
Phase B/C implementation itself is unblocked as of this confirmation, but
not yet built — tracked separately.

## Non-goals

- **No UI dock/panel.** The request was for agent-facing tools; `muxspect`
  CLI + MCP tools cover both agent and human-via-shell use. A dedicated
  cross-host conversation viewer pane is a plausible future phase, not
  bundled here.
- **No pagination/scrollback protocol.** Every tier stays tail-bounded at
  the existing 500-line cap. "Read agent X's full history" is out of scope;
  this is a liveness/recent-activity tool, consistent with `muxspect`'s
  existing identity.
- **No retrofit of visibility scoping onto host/cross-channel.** Those tiers
  keep today's all-or-nothing instance-auth-key model unchanged — flagged
  explicitly as a known inconsistency (anyone holding the instance key can
  already read any local agent's transcript, consent model or not) rather
  than silently glossed over. Closing that gap, if ever desired, is a
  separate spec.
- **Not a general remote-RPC framework.** This is one narrowly-scoped
  request/response pair for one bounded read, not the "invocation"
  mechanism `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`
  found entirely absent. A future second remote-read need should get its
  own narrow protocol rather than generalizing this one prematurely.
- **No new cryptography.** Reuses host HMAC / LAN Ed25519+TOFU / WAN
  reagent-pinned-key signing exactly as they exist today.

## Open questions / risks

- Blocking UX: a LAN/WAN `GetAgentTranscript` call is inherently
  asynchronous (jekt delivery, not synchronous RPC). Proposed: the tool
  blocks up to a bounded timeout (default 20s) waiting for a correlated
  `transcript_response`, then returns `status: "pending", request_id` if
  none arrived — paired with a `PollTranscriptRequest(request_id)` tool for
  the agent to check back later, rather than blocking indefinitely.
- Exact rate-limit numbers (§ Phase B) need to be checked against real
  muxbus delivery limits, not invented independently.
- WAN default `max_lines` needs bench data (§ Phase C).

## Phased plan summary

1. **Phase A** — host + cross-channel conversation visibility. No new trust
   model, no CLAUDE.md change, ships independently and immediately useful.
2. **Phase B** — LAN, gated on repo-owner confirmation of the new jekt rule
   above. New request/response jekt pair, `conversation_visibility` setting,
   `db_conversation_trust_grants` table, `muxspect conversation-requests`.
3. **Phase C** — WAN, same protocol, stricter defaults, gated on the same
   CLAUDE.md confirmation plus Phase B being live and stable first.

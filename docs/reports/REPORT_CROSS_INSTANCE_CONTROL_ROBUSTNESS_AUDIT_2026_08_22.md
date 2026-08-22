# Report: cross-channel/host/network control tooling — robustness audit

Date: 2026-08-22
Status: audit findings — no code changed by this report itself

## 1. Why this exists

AgentMux's stated primary value is far-and-wide (but secure) control of
AgentMux running on other channels, hosts, and networks. This audits the
CURRENT state of every surface that lets one agent observe or control
another instance, across four tiers — same-instance, cross-channel
(same host), LAN, WAN — to find where "far and wide" quietly stops short
of what it advertises, and where "secure" has a real, documented gap.
Follow-up to `REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`,
whose Ext 1–6 already closed the muxlog/muxspect diagnostic side; this
covers the broader control surface (messaging, fleet ops, UI automation,
discovery).

## 2. The four tiers, and what actually reaches each one today

| Tool | Host (own instance) | Cross-channel (same host) | LAN | WAN |
|---|---|---|---|---|
| `DiscoverAgents` / `FleetList` | ✅ | ✅ | ✅ | ✅ |
| `SendMessage` (jekt) | ✅ | ✅ (forward over loopback) | ✅ (mDNS peer, HTTP forward) | ✅ (muxbus cloud relay) |
| `FleetBroadcast` | ✅ | ❌ (bug — §3.1) | ❌ (bug — §3.1) | ❌ (bug — §3.1) |
| `FleetBulkStop` | ✅ | ❌ | ❌ | ❌ |
| `UIClick`/`UIQuery`/`Shell`/`NewTab`/`FocusWindow`/etc. | ✅ | ❌ | ❌ | ❌ |
| `CaptureWindow` | ✅ | ✅ (screenshot-only) | ❌ | ❌ |
| `muxspect` (list/describe/watch/find) | ✅ | ✅ (`find`, Ext 4 this session) | ❌ (Phase B, unbuilt) | ❌ (Phase C, unbuilt) |
| Structured remote command (run this / click this) | — | — | — | **does not exist on any tier** |

Two things stand out immediately:

- **Discovery already sees everything; almost nothing else does.**
  `/agentmux/discovery` (`server/mod.rs:681-757`) genuinely fans out across
  all four tiers — host, `list_all_shared` cross-channel, LAN via
  `lan_discovery.get_instances()`, WAN via
  `cloud_subscriber::subscribed_agents()`. Every OTHER control primitive
  ranges from "narrower than discovery" to "single-instance only." The gap
  between "what you can see" and "what you can act on" is the theme of this
  report.
- **There is no remote-command bus.** Confirmed directly in
  `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md`:
  *"remote API/RPC invocation... does not exist at all, on any tier —
  muxbus is a message bus, not an RPC bus."* The only thing that crosses
  LAN/WAN today is `SendMessage`'s free-text jekt injection into a target's
  conversation stream — no structured verb, no ack beyond delivery.

## 3. In-repo, actionable gaps

### 3.1 `FleetBroadcast` advertises LAN/WAN targets it cannot reach — real bug

`FleetList`/`DiscoverAgents` correctly return LAN and WAN entries (each
carrying a `block_id`, per the discovery response shape). But
`FleetBroadcast`'s own block_id→agent-name resolution
(`agentmux-mcp/src/main.rs:1730-1775`) only ever reads
`discovery.host.addressable` — the host-tier slice. A LAN or WAN `block_id`
returned by the very tool a caller would use to build the target list
silently fails to resolve ("agent not found") when handed to
`FleetBroadcast`. This is inconsistent with `SendMessage`, which reaches
all four tiers for the identical single-target case via
`/agentmux/reactive/inject`'s own cascading resolution
(`server/reactive.rs:386-755`).

**This is a scoped, mechanical, low-risk fix** — no new trust/security
model needed, since it reuses `SendMessage`'s already-shipped,
already-reviewed cross-tier resolution path; it only needs the same
per-target resolution `SendMessage` already does, looped, instead of a
resolution map limited to one tier. Recommend fixing as its own PR.

### 3.2 `FleetBulkStop` is host-only — needs a scoping decision, not a reflexive fix

`fleet_bulk_stop_impl` (`app_api/fleet.rs:164-226`) resolves every target
via the in-process `reactive_handler.get_agent_by_block` — host tier only,
by design or by omission, unclear which. Unlike §3.1, this is a
**destructive** primitive (stopping/killing agents), and extending it to
LAN/WAN means one host can now terminate agent processes on a DIFFERENT
machine it doesn't own the lifecycle of — a materially different risk
profile than broadcasting a text message. Flagging as a decision point,
not proposing a fix: does "far and wide control" mean bulk-stop should
reach other hosts too, or is host-only scoping here deliberate restraint
that should stay?

### 3.3 `muxspect` Phase B/C (LAN/WAN conversation visibility) — blocked on an explicit confirmation that hasn't happened yet

`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md` Phase A
(host + cross-channel) is built (this session's Ext 4/5 extended the same
area). Phase B (LAN) and Phase C (WAN) are fully designed but explicitly
gated — the spec's own words: *"This document is that proposal, not that
confirmation... Phase B must not ship until that confirmation happens and
CLAUDE.md's jekt section is updated to match."* The proposal itself
requires two real jekt-rule changes: a new forced-`TIER=sensitive` case for
incoming `transcript_request` jekts, and — notably — a case where the
2026-08-17 "verified sender → `ESCALATE=none`" relaxation must NOT apply
(reading another agent's live conversation is treated as more sensitive
than the credential-request scenario that relaxation was designed around).
This is squarely a human policy decision, not an engineering task — same
protocol as every other jekt-rule change this session's `CLAUDE.md`
documents (2026-08-14/15/17, each repo-owner-confirmed live before
shipping).

### 3.4 UI automation (`UIClick`/`UIQuery`/`Shell`/etc.) has no cross-instance path at all

Every one of these MCP tools reads `AGENTMUX_LOCAL_URL`/`AGENTMUX_AUTH_KEY`
once at process start (`agentmux-mcp/src/main.rs:630-631`) and never
repoints — hard-scoped to the instance the calling agent's own pane lives
in. `CaptureWindow` is the sole, deliberately narrow exception
(screenshot-only, same host, own-instance excluded, audit-logged —
`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6.1's
2026-08-21 addendum). `docs/specs/computer-use-pane.md`'s "Remote mode —
control another machine over AgentBus" is explicitly listed as an unbuilt
Phase 3 idea (line 191) with no design yet. Not a bug — this is
`muxspect`'s own Phase 1 restriction, applied consistently — but it means
"operate a task dev instance from another agent pane" (this session's
original tooling question) genuinely has no answer today beyond
screenshotting it.

## 4. Out-of-repo — infrastructure/ops, not a code change here

### 4.1 General agent-to-agent WAN jekt signing (issue #2586's other half)

Confirmed: **an arbitrary non-reagent agent's WAN jekt has zero
cryptographic identity proof today** — `source_agent` is just a claimed
field; only reagent's one pinned key is verified
(`cloud_subscriber.rs:944-957`, no other WAN verification path exists in
`reactive.rs`/`cloud_subscriber.rs`). A complete design already exists
(`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §5.1/§6.2): reuse the
exact `agentmux_common::jekt_sign` HMAC pattern already shipped for
host-tier, "just a new home" — but it's blocked on redesigning Cognito M2M
provisioning (current scheme caps at 100 app clients **system-wide**, not
per-account; needs one client per account + a pre-token Lambda injecting
authorized `agent_id`s as a claim). This is real AWS infrastructure work in
`agentmux-cloud`/`shared-infrastructure` — separate repos, live Cognito/
Lambda config — not something to implement blind from this repo.

### 4.2 `ENFORCE_AGENT_BINDING` — built, verified ~90%, never turned on

`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` §1.2 ("Gap A"): the WAN
binding-check enforcement flag lives in `agentmux-cloud` (Lambda env var),
currently **log-only** — it detects a mismatch and warns, never rejects.
`grep -rn ENFORCE_AGENT_BINDING` across `agentmux-cloud`/
`shared-infrastructure` finds it only in the check's own source and test
file — never set in any deployed environment config. The spec's own
remaining steps are explicitly ops, not code: "live-verify per-agent
credential provisioning end-to-end; burn in log-only mismatch monitoring;
flip `ENFORCE_AGENT_BINDING=true`." One real prerequisite (403 rejection
handling, §5.2) is confirmed already shipped in THIS repo
(`cloud_subscriber.rs:670-679`) — so the remaining work is genuinely just
verification + a deployed-config flip in a different repo's Lambda stack,
not new engineering here.

## 5. What's already solid (not a gap — stated for completeness)

- **Host-tier jekt signing** (per-agent HMAC-SHA256, `AGENTMUX_JEKT_KEY`) —
  real, injected only into the owning agent's own MCP process env.
- **LAN-tier jekt signing** (`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md`) —
  real per-agent Ed25519 keypairs, symmetric distribution
  (`lan_discovery.rs:808-890`), TOFU-pinned
  (`db_lan_peer_pubkey_pins`/`verify_lan_signature`). One accepted residual
  risk, explicitly documented in the spec itself: a race on the very first
  lookup for a given `agent_id` is still spoofable — deliberate, not an
  oversight.
- **LAN discovery** (mDNS + UDP broadcast fallback, source-IP-restricted to
  private/link-local ranges) has real bug history (TXT-clobber fix, the
  full-`auth_key`-broadcast vuln closed by splitting a LAN-scoped `lan_key`)
  and is reasonably mature, though the one true end-to-end multi-machine
  discoverability test is `#[ignore]`d by default (CI environment fragile —
  no multicast on GH Actions macOS runners) — verified manually/
  historically, not gated in CI. Worth knowing, not urgent to fix.

## 6. Recommendation

Three different classes of follow-up, deliberately not bundled into one
push:

1. **Ship now, in this repo:** §3.1 (`FleetBroadcast` cross-tier fix) —
   scoped, low-risk, reuses an already-shipped resolution path. Proposing
   a spec + implementation as an immediate follow-up PR.
2. **Needs your decision before any code:** §3.2 (should `FleetBulkStop`
   reach LAN/WAN — a destructive primitive, not a messaging one) and §3.3
   (confirm the two jekt-rule additions muxspect Phase B/C needs, same
   live-conversation-confirmation protocol as every prior jekt-rule
   change this session).
3. **Not this repo's work:** §4.1/§4.2 — real, already-designed,
   already-~90%-built fixes that live in `agentmux-cloud`/
   `shared-infrastructure` and need AWS-side verification + a config flip,
   not new code here. Flagging so they're visible, not attempting them
   from this session.

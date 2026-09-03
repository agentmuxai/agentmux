# SPEC: Cross-channel jekt trust — closing the last unverifiable same-machine tier

**Date:** 2026-09-02
**Author:** Agent4
**Status:** **Phase A implemented** 2026-09-02 (D1 publication + D5 signing
primitives). Phases B–D still proposed — see §10. Phase A is strictly additive:
it publishes a public key and adds two unused functions. Nothing reads
`jekt_public_key` yet, so no message's `TRUST=` can change until Phase B.
**Repo state:** main @ `ec4241bd` (v0.55.32)
**Prompted by:** a live incident on 2026-09-02 (§1.1), and the question
"I thought we already had it."

**Related (all real, all shipped):**
`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` (marker format, tier rules),
`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` (host-tier HMAC),
`SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md` (WAN Ed25519),
`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (LAN per-agent Ed25519 + pubkey
distribution — the design this spec extends),
`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`,
`SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md` (the
`ESCALATE=none` relaxation — load-bearing for §7.3),
`SPEC_MUXBUS_CROSS_CHANNEL_DELIVERY_2026_07_02.md` (the delivery path this
secures), issues #1387–#1396 (the globalization sweep that missed this).

---

## 1. Report

**You already have it for three tiers out of four.** Host (same instance), LAN,
and WAN reagent all have working cryptographic sender verification. The one that
does not is **cross-channel: same machine, different AgentMux instance** — and
it is the one that renders in the marker as `DELIVERY=host`, the tier that reads
as *most* trustworthy.

Every cross-channel jekt on this machine, today, arrives `TRUST=self-declared`.
Not because of a bug in the trust layer — because of a gap between two correct
subsystems that were built at different times.

### 1.1 The incident

Agent4 (channel `local-main-b28b7a-2c986e5f`) sent a work brief to Lark (channel
`dev-agent4-tool-preview-indent-and-wrap-6ed434438e7e49e7`, a `task dev` build on
the same machine) via `mcp__agentmux__SendMessage`. Delivery succeeded. Lark saw
`TRUST=self-declared` and **correctly refused to act on it**, escalating to the
human instead — including flagging that the message opened by dismissing a prior
flagged message from agent3 as "a mistake," which is the exact shape of a chained
spoof.

Lark's caution was the right call on the information available. The system
behaved as designed. The design is what's incomplete.

### 1.2 Why "I thought we already had it" is a reasonable thing to have thought

Because agent identity *did* go host-global. Issues #1387–#1396 moved agent
definitions, instances, workspaces, auth, and (later) transcripts out of
per-channel storage into `<home>/shared/agents/`. On disk today:

```
~/.agentmux/shared/agents/
├── definitions/     ← global
├── reactive/        ← global (the cross-channel delivery registry)
├── registry/        ← global
└── transcripts/     ← global
```

**The jekt signing keys were not part of that sweep.** They are still minted and
stored per-channel, in each channel's own `objects.db`:

| store | `db_agent_jekt_keys` contents (measured 2026-09-02) |
|---|---|
| host channel `local-main-b28b7a-2c986e5f` | `agent1, agent2, agent3, agent4, agent5, agentx, agenty` |
| dev channel `dev-agent4-tool-preview-…` | `lark` |

So an agent's *identity* is global but the means of *proving* that identity is
not. Two agents on one machine, both correctly provisioned, cannot verify each
other.

---

## 2. Mechanism, traced

### 2.1 Verification is a local-store lookup

`verify_jekt_signature` (`agentmux-srv/src/server/reactive.rs:172`):

```rust
let Ok(Some(key)) = state.wstore.agent_jekt_key_load(&claimed) else {
    return;          // ← cross-channel sender lands here, every time
};
```

`agent_jekt_key_load` (`backend/storage/agent_jekt_keys.rs:43`) reads
`db_agent_jekt_keys` in **this instance's own** Store. A foreign-channel sender
is never in it, so the function returns early and `req.sig_verified` stays
`None` → `TRUST=self-declared`.

This is not an oversight in that function — its doc comment is explicit that it
"only ever does anything when `agent_jekt_key_load` finds a LOCAL key." It was
written for a world where the only same-machine sender was a same-instance one.

### 2.2 The signature is almost certainly being produced

The sending side is fine. `inject_jekt_signing_keys_into_mcp_json`
(`backend/agent_config.rs:1275`) patches `AGENTMUX_JEKT_KEY` into the agent's
**MCP server** env block in `.mcp.json` — deliberately not into the agent's own
environment, so a compromised model can neither read nor forge with it.
Verified on agent4:

```
agent's shell env:  AGENTMUX_JEKT_KEY  ABSENT      ← by design
agent's .mcp.json:  AGENTMUX_JEKT_KEY  present     ← 44 bytes, base64
                    AGENTMUX_LAN_KEY   present     ← 44 bytes, Ed25519 seed
```

The message is signed. There is simply nobody on the receiving side holding the
matching secret.

### 2.3 The tier label is wrong, which makes it worse

`resolve_delivery_tier` (`reactive.rs:378`):

```rust
if auth_via == ReactiveAuthVia::LanKey { "lan" } else { claimed.unwrap_or("host") }
```

A cross-channel forward authenticates with the **peer instance's full
`auth_key`** (published in the shared registry entry — see §2.4), not a
`lan_key`. So it resolves to `host`. The reader sees `DELIVERY=host`, which
truthfully means "same machine" but implies "verifiable," which it is not.

`DELIVERY=lan` at least tells you a machine boundary was crossed. Cross-channel
gets the *strongest*-sounding label with the *weakest* guarantee.

### 2.4 The distribution channel already exists

`registry/paths.rs:90` — `resolve_shared_reactive_dir()` →
`<home>/shared/agents/reactive/<agent_id>/<channel>.json`. A real entry:

```json
{ "agent_id": "Agent4",
  "local_url": "http://127.0.0.1:63062",
  "block_id": "7dc7acae-…", "pid": 2456, "updated_at": 1788385773504,
  "auth_key": "<36 bytes>",
  "channel": "local-main-b28b7a-2c986e5f",
  "registration_nonce": 0 }
```

Host-global, one file per (agent, channel), already written on every
registration, already read by the cross-channel forward path. It carries an
*instance* auth key. It carries **no agent key material at all**. That is the
hook point this spec uses.

---

## 3. The second, sharper problem

Right now, `None` ("nothing to check against") is benign — it settles at
`TIER=coord` per the 2026-08-15 narrowing. That is correct *today*, because for
a cross-channel sender there is genuinely nothing to check.

**The moment we can resolve cross-channel identities, that stops being safe.**
If "I couldn't find a key" and "I found a key and you didn't sign" both produce
`None`, an attacker impersonating a cross-channel agent simply omits the
signature and gets the benign path. The fix would be bypassable by not using it.

This is the same class of bug reagentx already caught once on the LAN signing PR:
`verify_jekt_signature` was originally gated on `delivery_tier == "host"`, which
meant claiming `delivery_tier: "lan"` dodged the check entirely. The fix was to
run it unconditionally. Same shape here, one level up.

So §4 has two halves, and **the second is the one that actually buys trust:**
publishing keys (D1–D2) makes verification *possible*; requiring a signature once
identity is resolvable (D3) makes it *meaningful*.

---

## 4. Design

### D1 — Publish an Ed25519 **public** key in the shared registry entry

Add to `AgentEntry` (`backend/reactive/registry.rs`):

```rust
/// Base64 Ed25519 PUBLIC key for this agent, for cross-channel jekt
/// signature verification. Public half only — safe in a world-readable
/// file. Empty for entries written before this shipped (§6).
#[serde(default)]
pub jekt_public_key: String,
```

Derived from the agent's **existing** LAN keypair (`AGENTMUX_LAN_KEY`,
`jekt_sign::generate_lan_keypair`) — no new key material, no new provisioning
step, no respawn required for any agent that already has a LAN key. Written at
the same moment the entry is written.

### D2 — `verify_cross_channel_signature`

New function in `server/reactive.rs`, mirroring `verify_lan_signature`'s
structure but resolving the pubkey from the local filesystem instead of a LAN
round trip (so it is **synchronous** — no network, no rate limiter, none of the
`LanPubkeyLookup::Skipped` hazard):

```
1. claimed = req.source_agent, non-empty                      else → return
2. if agent_jekt_key_load(claimed).is_some() → return
      (same-instance sender; verify_jekt_signature owns it)
3. entries = shared registry entries for `claimed` across ALL channels
4. if entries is empty                       → return   (sig_verified stays None)
5. verified = req.channel_sig present
           && ts within JEKT_SIG_MAX_AGE_SECS
           && any(entry.jekt_public_key verifies the signature)
6. req.channel_verified = Some(verified)
```

Step 2 is the ordering guarantee: a same-instance sender keeps the existing
HMAC path untouched. This function only ever fires for senders this instance did
not spawn.

Step 5 iterates entries because one agent name can legitimately exist in several
channels at once (that is exactly why `AgentEntry.channel` exists). A verifying
match on any of them proves the sender holds that agent's private key.

### D3 — Resolvable identity ⇒ signature required

`channel_verified` is three-state, exactly like `sig_verified` /
`lan_verified` / `reagent_verified`:

| state | meaning | `TRUST=` | escalation |
|---|---|---|---|
| `None` | no entry in the shared registry — a genuine bridge, or an agent that has never registered | `self-declared` | unchanged (benign) |
| `Some(true)` | signature verified against a published pubkey | **`channel-verified`** *(new)* | never forced sensitive by trust alone |
| `Some(false)` | entry found, signature missing or wrong | `unverified` | **forced `TIER=sensitive`, `ESCALATE=required`, unconditionally** |

`Some(false)` is the load-bearing row. It is an *active* red flag — someone
claimed a specific, known agent's identity and could not prove it — and gets the
same treatment as host-tier `TRUST=unverified` and WAN `SIG=invalid`.

**New `TRUST` value, not a reuse of `host-verified`.** This follows the
precedent `lan-verified` set: each tier gets its own label, so a reader can tell
*how* identity was proven, not just *that* it was. `channel-verified` makes the
same strength claim as `host-verified` for the purposes of the 2026-08-17
`ESCALATE=none` rule, and must be added to that rule's verified-sender list.

### D4 — `DELIVERY=channel`, a tier of its own

`resolve_delivery_tier` gains a case: a request that arrived via cross-instance
forward (identifiable — the forwarding peer authenticates with an `auth_key`
belonging to a *different* channel's registry entry) resolves to `channel`, not
`host`. Reserving `host` for genuinely same-instance traffic means the label
finally matches the guarantee.

### D5 — Domain separation and channel binding

Cross-channel signatures use a distinct payload from LAN ones, via a new
`jekt_sign::sign_channel_jekt` / `verify_channel_jekt` pair:

```
"amx-jekt-channel-v1" || msgid || source_agent || source_channel
                      || target_agent || ts_secs || message
```

Two reasons, both necessary:

- **Domain separator** (`amx-jekt-channel-v1`): without it, the same Ed25519 key
  signs byte-identical payloads for LAN and cross-channel. A LAN signature
  captured off the wire could be replayed as a cross-channel one, and vice
  versa. Cheap to add now, impossible to add later without a flag day.
- **`source_channel`**: binds the signature to the channel that minted it, so a
  signature legitimately produced in channel A cannot be replayed as coming from
  channel B. Without it, an attacker who can read one channel's traffic can
  impersonate that agent's messages as if they came from any other channel it
  runs in.

### D6 — Anti-replay window

Reuse host-tier's `JEKT_SIG_MAX_AGE_SECS` (300s), **not** LAN/WAN's 600s.
Cross-channel is a same-machine HTTP call to `127.0.0.1`; it has no
real-network-latency budget to accommodate. The tighter window is the correct
one, for the same reason host-tier uses it.

---

## 5. Rejected alternative: globalize `db_agent_jekt_keys`

This is the obvious move — "identity is already global, make the keys global
too" — and is very likely what was assumed to have happened. It is a one-table
migration and it would make verification work. **Reject it.**

The host-tier key is a **symmetric HMAC secret**. Putting it in a host-global
store hands *every srv instance on the machine* the ability to **mint** a valid
signature for *every agent on the machine*.

That matters specifically because of how this machine is used. `task package`
and `task dev` create a fresh channel per build, and this machine routinely runs
many of them, built from arbitrary feature branches (`local-<branch>-<hash>-…`,
per `SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md`). Today a rogue or simply broken
srv build can only forge identities *within its own channel*. Globalizing a
symmetric secret would let any build on the machine forge as any agent in any
channel — including the stable one. That is a strictly worse posture than the
gap it closes.

Asymmetric publication (D1) has no such property: the shared file carries only
public halves, and a channel can still only sign as agents whose private keys it
actually holds.

The corollary is worth stating plainly: **`db_agent_jekt_keys` must stay
per-channel.** If a future change proposes globalizing it, this section is the
reason not to.

---

## 6. Migration and compatibility

- `jekt_public_key` is `#[serde(default)]`. Entries written by an older build
  deserialize with an empty string.
- **An empty `jekt_public_key` must yield `None`, not `Some(false)`.** An agent
  registered by a pre-upgrade instance has not failed to sign — it has not been
  *asked* to. Treating it as a red flag would escalate every cross-channel
  message on a mixed-version machine, which is exactly the false-positive
  fatigue §7.3 warns about.
- Registry entries are rewritten on every registration (i.e. every agent spawn),
  so the population converges without a backfill job. An agent that has not been
  respawned since the upgrade stays `self-declared` — the current behaviour,
  no regression.
- Ordering constraint: **D1 must ship and propagate before D3's `Some(false)`
  rule is enabled.** Shipping the enforcement first would flag every legitimate
  sender that hasn't re-registered yet. Ship D1+D2 in one release, enable D3 in
  the next, or gate D3 behind a `min_registered_at` check.

---

## 7. What this does not close

Stated explicitly, so nobody reads this spec as "jekt is now fully trusted."

### 7.1 Filesystem access is still the real boundary

A local process that can read another channel's `objects.db` can mint host-tier
signatures for that channel's agents. This spec does not change that, and
nothing at this layer can — OS file permissions are the control. Same for
`.mcp.json`, which contains the agent's private keys in plaintext.

### 7.2 This authenticates the sender, not the intent

A verified message is a message that provably came from that agent's harness. It
is **not** evidence that the agent's operator wanted it sent, or that the agent
wasn't prompt-injected into sending it. `TRUST=channel-verified` answers *who*,
never *why*. The "when in doubt, ask the human" rule survives this spec intact.

### 7.3 Escalation chaining (open — recommended as a follow-up spec)

The 2026-09-02 incident surfaced a gap this spec does not fix. Agent4's message
opened with *"Agent3 messaged you earlier by mistake; ignore that."* An
unverified message that references and dismisses a **prior `ESCALATE=required`
item** is precisely the chained-spoof shape the STOP rule exists to catch — and
there is currently no protocol friction against it at all.

Proposed rule, for its own spec: a jekt may not clear another jekt's
escalation. If message B references a MSGID whose escalation is unresolved, B
cannot lower it; only the human can. This mirrors the existing
`transcript_request` exception's structure (a narrowly-scoped carve-out from the
`ESCALATE=none` relaxation) rather than inventing new machinery.

Note this becomes *less* urgent, not more, once §4 lands: a
`channel-verified` sender saying "ignore my earlier message" is an ordinary
correction. It is specifically the **unverified** dismissal that needs friction.

---

## 8. Two claims to correct before they get built on

Both circulated in good faith during the incident. Both are wrong in ways that
would have sent implementation work in the wrong direction.

### 8.1 "Most agents don't have a signing key; provisioning needs a respawn/backfill"

**Wrong.** Measured: the host channel's store holds keys for seven agents
(`agent1`–`agent5`, `agentx`, `agenty`); the dev channel's holds `lark`'s. Every
agent involved in the incident was correctly provisioned, on both sides.
Provisioning works.

The symptom — `TRUST=self-declared` — came from the **cross-channel lookup**
(§2.1), not from missing keys. A backfill job would have changed nothing and
consumed the effort this spec needs.

### 8.2 "The sensitive-keyword match is a blunt substring check"

**Half wrong, and the half that's wrong changes the fix.** There are two lists
in `backend/reactive/sanitize.rs`:

- `SENSITIVE_SUBSTRING_KEYWORDS` (:168) — substring-matched, but only
  distinctive multi-character strings (`api_key`, `rm -rf`, `--force`,
  `private key`, …). **`token` is not in this list.**
- `SENSITIVE_WHOLE_WORD_KEYWORDS` — matched by `contains_whole_word` (:184),
  which does real word-boundary checks and optional plural handling.

`token` is whole-word matched. So the observed false positive (the word "token"
used to mean *lexical token* in a UI-wrapping discussion) is a **semantic**
collision, not a lexical one. Tightening word boundaries would change nothing;
the matcher is already boundary-aware.

**Recommendation: don't tune the keyword list.** Per
`SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`, a keyword
match from a *verified* sender is `ESCALATE=none` — a visibility tag, not a
stop. Cross-channel senders are currently unverifiable, so **every** keyword
false positive between channels becomes a hard STOP. Landing §4 converts that
whole class into a tag automatically, without touching the list or weakening
detection for genuinely unproven senders.

This is the strongest argument for prioritising this spec: it is also the fix
for the escalation-fatigue problem.

---

## 9. Test plan

Unit (`server/reactive.rs`, `jekt_sign.rs`):

1. Cross-channel sender, valid signature → `channel_verified == Some(true)`,
   `TRUST=channel-verified`, tier not forced.
2. Cross-channel sender, **no** signature, entry present → `Some(false)`,
   `TIER=sensitive`, `ESCALATE=required`.
3. Cross-channel sender, **wrong** signature → `Some(false)`, as above.
4. Cross-channel sender, entry present but `jekt_public_key` empty (pre-upgrade)
   → `None`, `self-declared`, **not** escalated. (§6)
5. Unknown sender, no registry entry anywhere → `None`, `self-declared`.
6. Same-instance sender → `verify_cross_channel_signature` is a no-op; existing
   HMAC path result unchanged. (D2 step 2)
7. Signature valid but `ts` outside 300s → `Some(false)`. (D6)
8. Agent registered in two channels; signature from one verifies. (D2 step 5)
9. **A valid LAN signature, replayed as a cross-channel signature, fails**, and
   vice versa. (D5 domain separator — this test is the whole point of D5.)
10. **A signature minted for channel A, replayed with `source_channel` = B,
    fails.** (D5 channel binding.)
11. Keyword match + `channel-verified` → `ESCALATE=none`. (§8.2)

Integration: two `task dev` instances on one machine, message each other both
directions, assert markers. This is cheap to run — the incident that prompted
this spec was exactly that setup, by accident.

---

## 10. Phasing

| Phase | Content | Ships with |
|---|---|---|
| **A** ✅ | D1 (publish pubkey), D5 (`sign_channel_jekt`/`verify_channel_jekt` + tests 9–10) | **implemented 2026-09-02** |
| **B** | D2 (verification), D4 (`DELIVERY=channel`), `channel-verified` added to the 08-17 verified-sender list; `Some(false)` **not yet** escalating | next release |
| **C** | D3 enforcement (`Some(false)` → forced sensitive) | after A has propagated (§6) |
| **D** | §7.3 escalation-chaining rule | separate spec |

Phase C is the one that must not be rushed forward. Phases A and B are
strictly additive — they can only move messages from `self-declared` to
`channel-verified`, never the reverse.

### 10.1 Phase A as built (2026-09-02)

| Piece | Location |
|---|---|
| `sign_channel_jekt` / `verify_channel_jekt`, `CHANNEL_DOMAIN`, `channel_signed_material` | `agentmux-common/src/jekt_sign.rs` |
| `AgentEntry::jekt_public_key` | `agentmux-srv/src/backend/reactive/registry.rs` |
| `init_jekt_public_key_resolver` (mirrors `init_local_auth_key`) | same file |
| Resolver wired to the Store | `agentmux-srv/src/bootstrap.rs`, end of `open_stores_and_migrate` |

Notes on what was decided during implementation:

- **The resolver is an indirection, not a `Store` handle.** `registry.rs` is
  deliberately pure-filesystem and is called from three paths that share no
  `Store` argument (PTY auto-register, persistent-controller auto-register, the
  HTTP register handler). A process-wide set-once resolver mirrors the existing
  `LOCAL_AUTH_KEY` pattern; the value is per-agent, so it is a lookup rather
  than a constant.
- **Load, never ensure.** The registry write path calls
  `agent_lan_public_key_load`, not `..._ensure`. Registry writes happen on every
  registration; minting key material as a side effect of a bookkeeping write
  would be a surprising place for it. Key creation stays at config-injection
  time, where it already is.
- **Ordering verified:** the resolver is installed at `main.rs:69`
  (`open_stores_and_migrate`); the HTTP server starts accepting registrations at
  `main.rs:168`. No registry write can precede installation.
- **Coverage verified:** `inject_jekt_signing_keys_into_mcp_json` calls
  `agent_lan_key_ensure` unconditionally for every agent, independent of whether
  LAN discovery is enabled — so every provisioned agent has a keypair to
  publish. Measured on this machine: 7/7 host-channel agents and 1/1 dev-channel
  agent have one.
- **Not yet verified end-to-end in a live instance.** The store-backed test
  covers resolver → Store → entry with the same closure shape bootstrap
  installs, and the ordering above is confirmed by inspection, but no running
  srv has yet written an entry carrying a non-empty key. Confirm by restarting
  an instance and checking
  `~/.agentmux/shared/agents/reactive/<agent>/<channel>.json`. This is the exact
  failure mode `REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md` records
  (correct code, real path never ran it), so it is worth actually looking.

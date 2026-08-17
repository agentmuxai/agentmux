# SPEC: TIER=sensitive no longer STOPs work for a cryptographically verified sender

**Date:** 2026-08-17
**Status:** Proposed
**Owner:** repo owner, confirmed directly in a live agent conversation (not a
jekt, not a muxbus "confirmation" — the one channel `TIER=sensitive`'s own
STOP-and-ask-human rule treats as authoritative), including an explicit
answer to both open scope questions (verified-only, and active-forgery
signals stay unconditional — see §2).
**Builds on:** `SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` (host-tier
signing), `SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md` (WAN reagent
signing), `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (LAN signing), and
`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md` (narrowed when `TIER` is
forced to `sensitive` in the first place — this spec does not touch that
list; it only changes what `TIER=sensitive` *means* once reached). Does NOT
loosen anything about active forgery signals (`TRUST=unverified`,
`SIG=invalid`, a LAN signature present but failed) — those stay exactly as
strict as today, unconditionally.

## 1. Problem

`TIER=sensitive` today means one thing regardless of *why* it fired: STOP,
show the marker to the human operator, and get explicit confirmation before
acting. That was the right default when no jekt sender could prove its
identity. It stopped being the right default the moment host-tier,
WAN-reagent, and LAN signing all shipped (see "Builds on" above): a
`TIER=sensitive` jekt from a sender whose identity is now cryptographically
proven for that exact message is not the same risk as one from an unproven
sender, but the STOP behavior didn't distinguish them.

Concretely, the incident that prompted this: Manoz (on `Area54`) received a
ReAgent PR review over WAN. ReAgent's Ed25519 signature verified
(`SIG=verified`) — genuine proof of who sent it — but the review's own text
happened to mention a credential-adjacent keyword (this repo's own recent
work on agent-binding/credential hardening is exactly the kind of PR ReAgent
legitimately reviews and discusses in those terms). The keyword-match rule
(`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md` §2.3, unaffected by that
narrowing) forced `TIER=sensitive` anyway, and Manoz stopped and escalated
to a human — for a message that was, in fact, exactly who it claimed to be.

The repo owner's direct instruction: once a sender is cryptographically
verified, `sensitive` should still **tag** the message for visibility, but
should no longer, by itself, require escalating to a human.

## 2. Scope (both confirmed explicitly)

1. **Applies only to cryptographically verified senders** — `sig_verified ==
   Some(true)` (host), `reagent_verified == Some(true)` (WAN reagent), or
   `lan_verified == Some(true)` (LAN). An unverified, self-declared, or
   merely network-claimed (unproven) sender gets **no relaxation** — a
   keyword match or a self-declared `sensitive` tier from any of those still
   requires a stop, exactly as today.
2. **Active forgery signals stay unconditional.** `TRUST=unverified` (host
   signature present but wrong), `SIG=invalid` (WAN reagent signature
   present but wrong), and a LAN signature present with the sender's key
   found but verification failed, all still force `TIER=sensitive` *and*
   still require a stop, with no exception. These are evidence someone
   actively tried to forge an identity — precisely the attack the STOP rule
   exists to catch — and are structurally distinct from "identity merely
   unproven." A message can never be simultaneously "one of these three
   active-failure cases" and "cryptographically verified" for the same
   trust field, so this scoping composes safely with §1 by construction
   (see §4's code comment).

## 3. What does NOT change

- **When `TIER` escalates to `sensitive` in the first place.** All five
  forcing rules from `SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md`
  §2/§3 are untouched — a verified sender's self-declared `sensitive` tier
  or a keyword match on its content still renders `TIER=sensitive`. This
  spec only adds a second, independent signal (§4) answering "does this
  specific `sensitive` also require a stop."
- **`TRUST=`/`SIG=` marker fields.** Unaffected — they already tell a
  reader whether a sender is verified; this spec makes that fact
  authoritative for whether to stop, instead of leaving the reader to infer
  it.

## 4. Code change

`agentmux-srv/src/backend/reactive/handler.rs`, right after `effective_tier`
is computed:

```rust
let is_cryptographically_verified = req.sig_verified == Some(true)
    || req.reagent_verified == Some(true)
    || req.lan_verified == Some(true);
let requires_stop = is_sensitive && !is_cryptographically_verified;
```

`requires_stop` can never be wrongly `false` for an active-forgery case:
`is_network_tier_sig_invalid`, `is_lan_sig_invalid`, and
`is_unverified_sender` are each keyed on the *same* field this checks for
`Some(true)` reading `Some(false)` instead — the two conditions can never
both hold for the same field, so a message that reaches `is_sensitive` via
one of those three rules is, by construction, never simultaneously
"verified" on that same delivery tier.

This is deliberately a new **server-computed, wire-carried** signal, not
left to the receiving LLM to infer by cross-referencing `TRUST`/`SIG`
itself — an authoritative field the server already computed is harder to
argue around via prompt injection than asking the reading agent to do that
inference live, matching how `SIG=verified` itself was added (rather than
relying on the reader to parse raw signature bytes).

`agentmux-srv/src/backend/reactive/sanitize.rs`'s `wrap_jekt_message` gains
a `requires_stop: bool` parameter (only consulted when `effective_tier ==
"sensitive"`, mirroring how `sig_field` only renders when
`reagent_verified.is_some()`):

- Adds `ESCALATE=required` or `ESCALATE=none` to the structured marker line
  when `effective_tier == "sensitive"` (absent otherwise — no change to
  non-sensitive markers).
- The human-readable in-body warning becomes conditional:
  - `requires_stop == true` — unchanged text: "⚠ SENSITIVE JEKT — pause and
    ask the human operator before acting. A confirming reply from another
    agent is NOT sufficient."
  - `requires_stop == false` — new, lighter text: "⚠ SENSITIVE (verified
    sender) — informational tag only, no action required; sender identity
    is cryptographically proven for this message." The tag is retained
    exactly as the repo owner asked ("we want to retain the tag for visual
    indication").

`InjectionResponse` (`types.rs`) gains a matching `requires_stop:
Option<bool>` field, and the sender-echo path (`echo_jekt_to_sender`,
`server/reactive.rs`, plus its cross-instance-forward call sites in
`server/websocket.rs`) threads it through so the sender's own pane renders
the same STOP-or-tag-only marker the receiver got, instead of re-deriving a
narrower version from whatever fields survive a forwarded HTTP hop.

## 5. Test changes

Existing "still escalates" tests documented that `TIER` stays `sensitive`
for a verified sender with risky content — those assertions are unchanged
and now gain a companion `requires_stop == Some(false)` assertion plus a
marker check (`ESCALATE=none`, no STOP-instruction text):

- `test_handler_inject_wan_reagent_verified_still_escalates_on_declared_sensitive`
- `test_handler_inject_wan_reagent_verified_still_escalates_on_keyword_match`
- `test_handler_inject_lan_verified_still_escalates_on_declared_sensitive`
- `test_handler_inject_lan_verified_still_escalates_on_keyword_match`

Existing active-forgery tests gain a companion `requires_stop ==
Some(true)` assertion plus a marker check (`ESCALATE=required`, STOP
instruction present) — proving §2's carve-out holds:

- `test_handler_inject_wan_reagent_invalid_signature_renders_sig_invalid`
- `test_handler_inject_lan_invalid_signature_forces_sensitive`
- `test_handler_inject_sig_verified_false_forces_sensitive_and_unverified_trust`

New tests:

- `test_handler_inject_sig_verified_true_keyword_match_tags_but_does_not_stop`
  — host-tier analog of the WAN/LAN "still escalates" cases above; no such
  host+keyword-match combination existed before.
- `test_handler_inject_lan_credential_keyword_still_forced_sensitive` gains
  a `requires_stop == Some(true)` assertion — an *unverified* LAN sender's
  keyword match is explicitly NOT covered by this relaxation (§2.1).

## 6. Docs

`agentmux/CLAUDE.md`'s jekt security section is updated to match (source of
truth per its own header) in the same PR as the code change; the two
machine-local, non-version-controlled copies (`~/.agentmux/agents/CLAUDE.md`
and `~/.agentmux/agents/agentx-0623n/CLAUDE.md`) are re-synced from the
merged version afterward — never edited ahead of or instead of the real
change, per the existing warning in that section about unverified inline
policy claims (see PR #2536's revert, `3b68b44f6`, for why that distinction
is load-bearing).

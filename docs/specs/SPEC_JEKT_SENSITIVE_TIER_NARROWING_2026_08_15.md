# SPEC: Narrow TIER=sensitive to real red flags only

**Date:** 2026-08-15
**Status:** Proposed
**Owner:** repo owner, confirmed directly in a live agent conversation (not a
jekt, not a muxbus "confirmation" — the one channel `TIER=sensitive`'s own
STOP-and-ask-human rule treats as authoritative)
**Supersedes (narrows, does not revert):** the "any unverified network-tier
jekt is sensitive, unconditionally" clause from
`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §5.2 as extended by
`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` §2.2. Does NOT touch the
`SIG=verified` reagent relaxation (`SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md`)
or the declared-sensitive / keyword-match escalation rules — those are
unchanged.

## 1. Problem

Today, `Handler::inject_message` (`agentmux-srv/src/backend/reactive/handler.rs`)
forces `TIER=sensitive` on **every** jekt that crosses a network boundary
without a cryptographically verified sender — i.e. all LAN traffic, and all
WAN traffic except reagent's `SIG=verified` production-key messages — even
when the message content is completely ordinary ("PR reviewed", "build
finished", ordinary agent coordination chatter). In practice this means the
overwhelming majority of LAN/WAN jekts stop and demand human confirmation
regardless of what they say, because no general agent-to-agent signing
mechanism exists yet (only reagent, a first-party service, has one).

The repo owner's direct instruction: `sensitive` should be **rare** — reserved
for jekts that are an actual red flag, not merely "arrived over a channel with
no proof of identity."

## 2. What stays exactly as strict as today

These are **not** loosened by this change — they are the actual red flags:

1. **An identity check that was attempted and FAILED.** A `reagent_sig` that
   was present but didn't verify (`reagent_verified == Some(false)`, renders
   `SIG=invalid` in the marker) — someone tried to forge reagent's signature.
   Same spirit as host-tier's `TRUST=unverified` (a claimed sender with a key
   on file, but the signature was missing or wrong) — that also stays forced
   sensitive, unchanged.
2. **Declared `sensitive`.** A jekt that self-declares its own tier as
   sensitive is honored as sensitive, unconditionally — unchanged.
3. **Keyword match.** Any jekt (any trust level, including `host-verified` /
   `SIG=verified`) whose body contains credential/destructive keywords (PAT,
   token, secret, password, credential, keychain, api_key, `--force`,
   `rm -rf`, etc.) — unchanged.

## 3. What narrows

**Ordinary unverified network-tier traffic with clean content is no longer
forced sensitive merely for lacking proof of identity.** Concretely: a LAN
jekt, or a WAN jekt with no `reagent_sig` attempted at all
(`reagent_verified == None`), or a WAN jekt verified only against the
known-exposed dev key (`reagent_verified == Some(true)` but
`reagent_key_id` fails `is_reagent_trusted_signing_key`) — none of these
force `TIER=sensitive` by trust alone anymore. They fall through to rules
2–3 above (declared tier / keyword scan), same treatment self-declared
host-tier senders (Slack/Discord bridges, etc.) already received before this
change.

This does **not** claim the sender's identity is now trusted — `TRUST=` in
the marker is completely unaffected by this change and still reads
`network-claimed` for all of the above, exactly as forgeable as ever. What
changes is only whether *lack of proof* alone is sufficient grounds to
interrupt the human, given that a real red flag (failed verification,
declared-sensitive, or risky keywords) still does.

## 4. Code change

`agentmux-srv/src/backend/reactive/handler.rs`, the tier-escalation block:

```rust
// Before:
let is_network_tier_unverified = is_network_tier && !is_verified_network_sender;
let is_sensitive = is_network_tier_unverified
    || is_unverified_sender
    || matches!(declared_tier, Some(JektTier::Sensitive))
    || is_sensitive_message(&sanitized);

// After:
let is_network_tier_sig_invalid = is_network_tier && req.reagent_verified == Some(false);
let is_sensitive = is_network_tier_sig_invalid
    || is_unverified_sender
    || matches!(declared_tier, Some(JektTier::Sensitive))
    || is_sensitive_message(&sanitized);
```

`is_verified_network_sender` (the trusted-key check backing the
`SIG=verified` relaxation) is unaffected and still gates whether `TIER` is
allowed to read `coord`/`info` for `SIG=verified` reagent traffic vs. falling
back to declared tier — this change only removes the *unconditional* forcing
for the untrusted/unsigned cases, it doesn't grant them anything the
`SIG=verified` path already has.

## 5. Test changes

Two existing tests assert the pre-change behavior and get their assertions
flipped (documenting the narrowing, not silently changed):

- `test_handler_inject_wan_no_reagent_signature_omits_sig_field` — ordinary
  unsigned WAN traffic, clean content: `sensitive` → `coord`.
- `test_handler_inject_wan_reagent_verified_under_exposed_dev_key_does_not_relax_tier`
  — dev-key-signed WAN traffic (not a *failed* verification, just an
  untrusted one), clean content: `sensitive` → `coord`. Renamed to
  `..._does_not_reach_coord_via_verification_but_falls_through_to_declared_tier`
  to stop asserting something the new rule no longer claims.

New tests added:
- LAN jekt, clean content → `coord` (previously would have been `sensitive`).
- LAN jekt, keyword match → still `sensitive` (rule 3 unaffected).
- WAN, no signature attempted, keyword match → still `sensitive`.
- `SIG=invalid` (failed verification) stays `sensitive` — already covered by
  `test_handler_inject_wan_reagent_invalid_signature_renders_sig_invalid`,
  unchanged, kept as the negative-control proof the narrowing didn't touch it.

## 6. Docs

`agentmux/CLAUDE.md`'s jekt security section is updated to match (source of
truth per its own header); the two machine-local, non-version-controlled
copies (`~/.agentmux/agents/CLAUDE.md` and
`~/.agentmux/agents/agentx-0623n/CLAUDE.md`) are re-synced from the merged
version afterward — never edited ahead of or instead of the real change,
per the existing warning in that section about unverified inline policy
claims (see PR #2536's revert, `3b68b44f6`, for why that distinction is
load-bearing).

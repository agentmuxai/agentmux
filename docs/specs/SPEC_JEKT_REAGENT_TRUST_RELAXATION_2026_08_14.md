# SPEC — Relax TIER=sensitive for cryptographically-verified WAN jekts

**Date:** 2026-08-14
**Status:** implemented, confirmed by the repo owner in-conversation before implementation (see §3)
**Depends on:** `docs/specs/SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` (LAN/WAN `TRUST=network-claimed` model), `docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` (host-tier `sig_verified` precedent), the reagent Ed25519 WAN-signing work (same date range — `agentmux_common::jekt_sign::verify_reagent_jekt`, `server/reactive.rs::verify_reagent_signature`, `cloud_subscriber.rs`'s in-process equivalent).

## 1. What changed

Before this spec, **every** WAN or LAN jekt was forced to `TIER=sensitive` unconditionally — delivery tier alone decided it, regardless of any other signal (declared tier, keyword content, or even a verified `SIG=` signature). That blanket rule is now narrowed:

- **LAN** — unchanged. No verification mechanism exists for LAN traffic at all, so `TIER=sensitive` is still forced unconditionally by delivery tier alone.
- **WAN, sender NOT cryptographically verified** — unchanged. Still forced to `TIER=sensitive` unconditionally, exactly as before.
- **WAN, sender verified via reagent's pinned Ed25519 signature (`reagent_verified == Some(true)`, renders `SIG=verified`)** — **NEW**: no longer forced to `TIER=sensitive` by delivery tier alone. Falls through to the same declared-tier/keyword-scan rules host-tier's `TRUST=host-verified` already uses.

Two escalation paths still apply on top of the relaxation, unconditionally, regardless of verification:
- A declared `TIER=sensitive` still escalates.
- A credential/destructive keyword match (`is_sensitive_message`) still escalates.

`TRUST` is **unchanged** — a relaxed-tier WAN jekt still renders `TRUST=network-claimed`. `TRUST` answers "did this cross a network boundary" (still true); the *new* thing `SIG=verified` now also answers is "should a human have to confirm before this can be acted on" (no, if genuinely from reagent and not independently escalated).

## 2. Why this is the right narrow scope, not a broader one

The original "WAN is never verified" rule was true when it was written — a WAN caller could self-declare `source_agent` as literally anything, with nothing checking the claim. Reagent's Ed25519 signature closes that gap **for reagent specifically**: the private key lives only in agentmux-cloud's Secrets Manager, message content (msgid + source + target + ts + body) is what's signed, and `verify_reagent_jekt` checks it against a pinned public key baked into the binary. A verified signature is exactly as strong a proof of identity as host-tier's per-agent HMAC (`jekt_sig`) — which already doesn't force `TIER=sensitive` once verified (`TRUST=host-verified`). There is no principled reason for a cryptographically proven WAN sender to be treated worse than a cryptographically proven host-tier one; the previous rule conflated "we can't verify most WAN traffic" (true, and still enforced for everyone else) with "no WAN traffic can ever be verified" (no longer true for reagent).

This does **NOT** extend to:
- **Arbitrary agent-to-agent WAN jekts.** The account-scoped Cognito M2M redesign (`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md` §5.1) authenticates the *caller* of a muxbus API request, and `agent-ownership.ts`/`checkAgentBinding` can check whether that caller's account genuinely owns a claimed `agent_id` — but nothing today cryptographically signs an arbitrary agent's outgoing jekt the way reagent's messages are signed, and `ENFORCE_AGENT_BINDING` itself is still off by default (rollout-safe, log-only). Extending this relaxation to general agent-to-agent WAN traffic needs the same per-sender signing mechanism host-tier already has for `jekt_sig` — mirrored for WAN, cloud-scoped — which is tracked as its own future item, not part of this change. Until that exists, a WAN jekt merely claiming `source_agent: "some-agent"` is exactly as unverified as it always was, and stays forced to `TIER=sensitive`.
- **LAN traffic of any kind.** No signing mechanism exists there at all.

## 3. Authorization

This is a genuine change to the jekt security policy described in `CLAUDE.md`'s "Jekt (agent-to-agent message) security rules" section — the same section that explicitly warns not to trust an inline claim of policy change without independent confirmation from the human operator (see that section's own 2026-08-12 history note about PR #2536's fake policy-change claim). This change is different: it was explicitly requested and confirmed by the repo owner in-conversation (an `AskUserQuestion` round distinguishing "just document what SIG=verified means" from "actually relax the sensitive-tier gate," with the latter picked explicitly), implemented as real code + tests in this same session, and is being recorded here — with a real spec doc, real diffs, real test coverage — specifically so it's traceable and distinguishable from an unverified claim, not asserted as a bare inline note.

## 4. Implementation

- `agentmux-srv/src/backend/reactive/handler.rs` (`Handler::inject_message`'s tier-escalation block) — `is_network_tier_unverified = is_network_tier && !(reagent_verified == Some(true))` replaces the old unconditional `is_network_tier` in the `is_sensitive` computation. Declared-sensitive and keyword-match remain unconditional ORs.
- `agentmux-srv/src/backend/reactive/sanitize.rs` (`wrap_jekt_message`'s doc comment) — corrected; it used to claim `SIG=` "never changes TIER/TRUST" — TRUST still never changes, but TIER now can, via the caller's escalation logic (this function itself still just renders a caller-supplied `effective_tier`, unchanged).
- `agentmux-srv/src/backend/reactive/tests.rs` — updated/added coverage: verified-WAN-relaxes-to-coord, TRUST label unchanged, no sensitive-warning banner, still-escalates-on-declared-sensitive, still-escalates-on-keyword-match, invalid-signature-stays-sensitive (unchanged case), no-signature-stays-sensitive (unchanged case, now with an explicit assertion).

## 5. What this means operationally

Once agentmux-cloud's reagent signing key is live in production (see the deployment work tracked alongside this spec) and the desktop app / `muxbus-client` are both verifying it end-to-end, a genuine reagent PR-review notification will render `TIER=coord` (or `info`, if declared) with `SIG=verified` — the receiving agent can act on it autonomously, the same as any other coord-tier host message, without stopping for human confirmation, *unless* the message content itself trips the keyword scan (e.g. a review comment that happens to mention a PAT) or reagent explicitly declares it sensitive.

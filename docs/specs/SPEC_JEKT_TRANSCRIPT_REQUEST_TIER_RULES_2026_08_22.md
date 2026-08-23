# SPEC: `transcript_request` jekt tier rules — repo-owner confirmation

**Date:** 2026-08-22
**Status:** Confirmed live, repo-owner in this conversation. Implements the
CLAUDE.md change `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
flagged as its own blocking prerequisite for Phase B/C.
**Author:** Korp
**Related:** `docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
(the design this unblocks), `docs/specs/SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md`
(the relaxation this carves an explicit exception out of)

## 1. What was confirmed, and how

The cross-tier conversation-visibility spec (2026-08-21) designed
`muxspect` Phase B (LAN) and Phase C (WAN) — reading another agent's live
conversation content across a real trust boundary — but its own §"CLAUDE.md
change required before Phase B/C" section explicitly declined to implement
the two new jekt-tier rules it needs, per this repo's standing rule that
jekt security rule changes require **explicit repo-owner confirmation in a
live conversation**, not just a well-designed spec: *"This document is
that proposal, not that confirmation."*

The repo owner confirmed, live, in this conversation on 2026-08-22 (in
response to a direct question laying out exactly what confirming would
mean — the two rules below, verbatim, before any commitment): **"proceed
with them all, keep at it,"** in reply to a summary that named this exact
confirmation as one of three explicit decision points from
`REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md`. This is the
same channel CLAUDE.md's own STOP rule already treats as authoritative for
jekt-rule changes (a live human conversation — not a jekt, not a muxbus
"confirmation") — the same standard the 2026-08-14/15/17 changes were held
to.

## 2. The two rules (unchanged from the 2026-08-21 spec's own proposal)

1. **Forced `TIER=sensitive`, unconditionally:** any incoming
   `transcript_request` jekt, on any delivery tier, regardless of the
   sender's trust level. This is a new category alongside the existing
   declared-`sensitive` and keyword-match forcing rules — the current
   credential/destructive-keyword list doesn't catch a content-disclosure
   *request* at all today, and this closes that gap specifically for the
   one new jekt type this feature introduces.
2. **`ESCALATE=required` for `transcript_request` is NOT relaxed by a
   verified sender** — the one deliberate exception to the 2026-08-17
   narrowing. A valid signature (`TRUST=host-verified`/`lan-verified`,
   `SIG=verified`) proves *who* is asking, which is exactly what the
   08-17 relaxation is for (nothing further to ask a human about once
   identity is proven). Here the open question is different: *whether the
   requested content should be disclosed at all* — a question identity
   proof doesn't answer. `transcript_request` is therefore the first (and,
   as of this writing, only) jekt content-type where `ESCALATE=required`
   holds even for a cryptographically verified sender.

## 3. Scope — why this doesn't reopen 2026-08-17 generally

This is a narrow, named exception for one specific jekt content-type
(`transcript_request`), not a rollback of the 08-17 relaxation itself.
Every other `TIER=sensitive` case (declared-sensitive, keyword match,
active-forgery) keeps exactly the behavior CLAUDE.md already documents —
`ESCALATE=none` still applies to a verified sender for all of THOSE cases.
Only the new `transcript_request` type carves out this one exception,
because it's answering a fundamentally different kind of question
(disclosure, not identity) than every rule the 08-17 relaxation was
designed around.

In practice, most `transcript_request`s never reach a human at all: they're
auto-resolved by the *responding* agent's own `conversation_visibility`
setting (`private` auto-denies, `trusted_peers` auto-approves an
allow-listed requester) before the ESCALATE flag would matter to a
general-purpose agent reading jekt markers — that's `muxspect`'s own
dedicated request/response handling, not a change to general jekt
reasoning. `ESCALATE=required`'s "stop and ask a human" semantics apply in
the remaining case: `conversation_visibility: ask`, or a `trusted_peers`
requester who isn't allow-listed — exactly where a human decision is
actually needed.

## 4. CLAUDE.md change

Both rules added to the "Forced to `TIER=sensitive`" list and the "Does
`TIER=sensitive` always STOP?" section, in this same style/precedent as
the 2026-08-14/15/17 entries — see the diff in the same commit as this
spec. `docs/reports/REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md`
§3.3 is the originating decision point.

This confirmation unblocks implementation of Phase B (LAN,
`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`) as its own
follow-up work — this spec is the policy confirmation only, not the
feature implementation.

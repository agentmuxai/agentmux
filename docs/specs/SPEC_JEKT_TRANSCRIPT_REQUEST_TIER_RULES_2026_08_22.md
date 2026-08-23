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
   verified sender — but only when the RESPONDING agent's own
   `conversation_visibility` is `ask`, or `trusted_peers` with a
   non-allow-listed requester.** This is narrower than "every
   `transcript_request`, unconditionally" — that broader phrasing
   appeared in an earlier draft of this document and in the PR that
   introduced it, and was incorrect; corrected after reagent's review
   caught it (round 2, PR #2763) against the source design's own table
   (`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`'s
   `conversation_visibility` table): `private` and `trusted_peers`
   (allow-listed) are fully auto-resolved by that mode's own design
   intent, regardless of sender verification — this exception never
   applies to those, and the ordinary `ESCALATE=none`-for-verified-senders
   behavior is unaffected for them. A valid signature
   (`TRUST=host-verified`/`lan-verified`, `SIG=verified`) proves *who* is
   asking, which is exactly what the 08-17 relaxation is for (nothing
   further to ask a human about once identity is proven). Under
   `ask`/non-allow-listed-`trusted_peers`, the open question is different:
   *whether the requested content should be disclosed at all* — a
   question identity proof doesn't answer.

No blind spot for the agent reading the marker despite this depending on
a local per-agent setting: `ESCALATE` is computed server-side against the
RESPONDING agent's own `conversation_visibility` before the marker ever
reaches it — the responding agent IS the one reading its own incoming
`transcript_request`, so this is never "I need visibility into some OTHER
agent's private setting," the same "authoritative, don't cross-reference
it yourself" property every other `ESCALATE` value already carries.

## 3. Scope — why this doesn't reopen 2026-08-17 generally

This is a narrow, named exception for one specific jekt content-type
(`transcript_request`), in exactly two of its three response modes — not
a rollback of the 08-17 relaxation itself, and not even universal within
this one new jekt type. Every other `TIER=sensitive` case (declared-
sensitive, keyword match, active-forgery), and `transcript_request` under
`private`/allow-listed-`trusted_peers`, all keep exactly the behavior
CLAUDE.md already documents — `ESCALATE=none` still applies to a verified
sender for all of those. Only `transcript_request` under
`ask`/non-allow-listed-`trusted_peers` carves out this one exception,
because it's answering a fundamentally different kind of question
(disclosure, not identity) than every rule the 08-17 relaxation was
designed around.

In practice, most `transcript_request`s never reach a human at all: `private`
and allow-listed-`trusted_peers` are auto-resolved by the *responding*
agent's own `conversation_visibility` setting — that's `muxspect`'s own
dedicated request/response handling, not a change to general jekt
reasoning. `ESCALATE=required`'s "stop and ask a human" semantics apply
in the remaining case — `conversation_visibility: ask`, or a
`trusted_peers` requester who isn't allow-listed — exactly where a human
decision is actually needed, and exactly where rule 2 above actually
applies.

## 4. CLAUDE.md change

Both rules added to the "Forced to `TIER=sensitive`" list and the "Does
`TIER=sensitive` always STOP?" section, in this same style/precedent as
the 2026-08-14/15/17 entries — see the diff in the same commit as this
spec. `docs/reports/REPORT_CROSS_INSTANCE_CONTROL_ROBUSTNESS_AUDIT_2026_08_22.md`
§3.3 is the originating decision point.

**Important distinction from the 2026-08-14/15/17 entries this spec is
otherwise styled after: this rule is not yet enforced by any code.**
`transcript_request` does not exist anywhere in `agentmux-srv` today —
`muxspect` Phase B/C (`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`)
is designed, not built. Both CLAUDE.md bullets are marked
`[Not yet live — pre-committed policy, not current behavior]` for exactly
this reason (reagent P1 on this spec's own PR, round 1 — the original
wording read as though srv already enforces this, which is false and
could mislead an agent into believing a `transcript_request` marker it
saw today had already been through real server-side escalation logic).
Whoever implements Phase B/C is responsible for actually wiring these two
rules into `reactive/handler.rs`'s escalation logic as part of that work —
this document records the POLICY decision, not a claim that the code
exists.

This confirmation unblocks implementation of Phase B (LAN,
`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`) as its own
follow-up work — this spec is the policy confirmation only, not the
feature implementation.

## 5. On verifiability (reagent P2 on this spec's own PR)

A fair point was raised: the only evidence backing this confirmation is
this document's own prose, describing a live conversation no automated
reviewer can independently inspect — exactly the class of claim CLAUDE.md's
own jekt section says to treat with "#2536-level skepticism" absent
independent confirmation or real code+tests. Worth being precise about
what that means here rather than waving it off:

- This is not a gap unique to this spec — it's the same standing
  limitation every "repo-owner confirmed live" entry in CLAUDE.md already
  has (2026-08-14, 08-15, 08-17). None of those are independently
  verifiable by a code reviewer either, by the nature of what they're
  recording: a human conversation, which is deliberately the ONE channel
  CLAUDE.md treats as authoritative specifically *because* it can't be
  spoofed the way a jekt or muxbus message can. Documenting it in a spec
  (rather than only in this document's own commit message) is the
  established convention this repo already uses for that limitation, not
  a new or lower bar introduced here.
- The asymmetry ReAgent itself noted matters: this change only *tightens*
  escalation (a new forced-sensitive category, one case where a relaxation
  does not apply) and, per §4, isn't even active yet. The 2026-08-15/17
  precedents this spec is styled after were higher-stakes precisely
  because they *loosened* protections — the kind of change a fabricated
  "confirmation" would be motivated to sneak in. A tightening-only,
  not-yet-live policy commitment has a fundamentally different risk
  profile: even in the worst case where this confirmation were somehow
  mistaken, the outcome is a feature that doesn't exist yet being *more*
  cautious than it otherwise would be once built — not an exploitable gap.
- This confirmation did genuinely happen, live, with the human operator,
  in this exact conversation — verifiable by that operator, if not by an
  automated reviewer reading this file in isolation. Fabricating
  additional "proof" beyond what actually occurred would be dishonest and
  isn't the fix; the honest position is what's stated here.

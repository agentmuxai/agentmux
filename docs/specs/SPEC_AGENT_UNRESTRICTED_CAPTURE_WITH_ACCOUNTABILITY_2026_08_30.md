# SPEC: Safe unrestricted screen capture for agents

**Status:** **Phase 1 implemented** (tier resolution, `allow` defaults, extended
audit). Phases 2–4 not started. Supersedes the own-pane-only capture policy in
`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6 **for capture
only** — that spec's rules for `UIClick`/`UIQuery` are untouched (see §8).
**Date:** 2026-08-30
**Author:** AgentA
**Directive:** repo owner, this session — *"agents need to be able to screenshot
anything, including other agents"* and *"we need a safe way for agents to be
able to screenshot anything."*

---

## 0. Why the current policy is being replaced

The existing restriction was an **agent-authored recommendation that was never
ratified**. `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6 says
cross-agent targeting, *"if it's ever wanted at all,"* should be a distinct
higher-sensitivity capability — the author did not know whether it was wanted,
defaulted closed, and noted in the same breath that **no capability-flag
mechanism existed to express anything finer**. `CaptureWindow`'s `!is_self`
filter (PR #2709 round 3) is downstream of that: it exists only to stop
`CaptureWindow` end-running the `UIScreenshot` boundary, with no independent
justification.

Two corrections to the record, because they change the risk calculus:

1. **The PR #2662 P0s do not validate the boundary.** Both (client-supplied
   `block_id`; the DOM-ownership check missing descendants) were *bypasses of
   the chosen policy*. They show the boundary was implemented sloppily at
   first, not that it should exist. Under this spec's policy neither would
   have been a vulnerability.
2. **"Credentials in a form" is largely already handled.** Every credential
   input in the repo is `type="password"`, which renders masked — a screenshot
   leaks nothing. The real exposure is *plaintext* secrets in transcripts and
   terminals, which the old boundary addressed only incidentally and which §4
   below targets directly.

What the old policy got right and this spec keeps: **identity must be proven,
never claimed.** That machinery (§1) is load-bearing and untouched.

## 1. What does not change

`UIScreenshot`/`UIClick`/`UIQuery` authenticate via `sign_ui_automation_auth()`
— an HMAC over a fixed identity tuple using the per-agent `AGENTMUX_JEKT_KEY`,
verified server-side by `verified_block_id`, which then resolves the caller's
identity from a **server-owned registry**. No client-supplied pane id is ever
trusted. `ui_click_rejects_a_forged_agent_identity` locks this in.

This spec changes **what a proven identity is permitted to do**. It does not
weaken how identity is proven. Any implementation that starts trusting a
client-supplied target id has misread this document.

## 2. The model: accountability instead of prevention

Blocking was chosen because there was no way to be selective. The replacement
is three independent dials per tier, not one on/off switch:

| Dial | Meaning |
|---|---|
| **allow** | is the capture permitted at all |
| **audit** | is it recorded, attributably and durably |
| **notify** | does the observed party find out |

The safety property that replaces blocking is **non-silence**: an agent may
look at anything it is allowed to look at, but it cannot look *quietly*. That
is what makes an open default defensible — surveillance, not observation, is
the thing worth preventing.

## 3. Tiers

Keyed on **what is being captured**, not on who asks.

| Tier | Target | allow | audit | notify |
|---|---|---|---|---|
| **T0** | The caller's own pane | yes | no | no |
| **T1** | Another pane, same instance | yes | yes | yes |
| **T2** | Another AgentMux instance, same OS user | yes | yes | yes |
| **T3** | Any window owned by a **different OS user** | **no** (opt-in) | yes | n/a |
| **T4** | Non-AgentMux window, same OS user | yes | yes | no |

Rationale for the two non-obvious rows:

- **T3 stays closed by default.** This is the one boundary that is not
  agent-to-agent but human-to-human. The existing code already treats it as
  special — `DiscoverWindows`' own audit rationale calls out that `exe_path`
  "typically embed[s] the OS username." A shared machine's other user did not
  consent to this app's agents, and no in-app notification reaches them.
  Overridable by an explicit host-level setting, not by an agent.
- **T4 needs no notify** because there is no in-app party to notify. It is
  audited because this is where a password manager or a banking tab lives —
  PR #2709 round 2 caught the unscoped version capturing a KeePass window.

`T0` stays silent deliberately: auditing an agent looking at its own pane is
noise that would bury the entries that matter.

## 4. Redaction — where it is possible, and where it honestly is not

**Same-instance (T1) captures should route through CDP, not OS pixel grab.**
That is the only tier where AgentMux controls the renderer, and therefore the
only tier where regions can be masked before the frame is taken:

- A `data-amx-redact` attribute marks a subtree as sensitive.
- Before a T1 capture, the capture pipeline sets a document-level class that
  paints those subtrees over; it is cleared immediately after.
- T0 is exempt — your own pane is yours.
- Applied initially to the known plaintext-secret surfaces: identity account
  forms, token chips, OAuth code display, `run_cli_login` URL rendering.

**T2 and T4 cannot be redacted.** They are OS pixel captures of a process this
instance does not control. Stating that plainly rather than implying uniform
protection: at those tiers, audit and notify are the entire control, and a
plaintext secret visible on screen will be captured. That is a real, accepted
residual risk of the directive, not something this design quietly solves.

## 5. Notification

For T1/T2, the observed party learns, through two channels:

1. **Transient pane indicator** — the captured pane shows a brief marker
   ("captured by <agent>"), the same treatment class as the existing
   `Reconnecting…`/`Compacting…` status affordances.
2. **Transcript event** — a durable line in the observed agent's own
   transcript, so it survives the indicator fading and is visible on scrollback.

Notification is **best-effort and must never gate the capture** — the same
discipline `audit_log_capture_window` already follows ("a logging failure must
never break the tool itself"). A capture that succeeds while its notification
fails is still audited; the audit log is the durable record, the notification
is the courtesy.

## 6. Audit

Extend the existing append-only NDJSON trail rather than inventing a second
one. Current entries record `timestamp`, `agent_id`, `tool`, `query`,
`outcome`. Add:

- `tier` (T0–T4)
- `target` — resolved pane id / pid / window title, not just the query string
- `image_sha256` — so a leaked screenshot can be traced back to the capture
  that produced it
- `redacted` — whether masking was applied, so an unredacted T2 capture is
  distinguishable at review time

**Correction (2026-08-30, during Phase 1 implementation):** an earlier draft of
this section said `UIScreenshot` must write to the same trail because it is
"the most privileged capture path." That was wrong twice over, and it
contradicted §3's own table. `UIScreenshot` is own-pane-only — the *least*
privileged path, and T0, which §3 deliberately leaves unaudited so the entries
that matter aren't buried in noise. It is therefore **not** audited in Phase 1.

That changes if `UIScreenshot` ever gains cross-pane reach: at that point it
stops being T0, and the tier it lands in carries its audit/notify obligations
automatically. No separate rule needed.

## 7. Containment

- **Rate limit** T1+ per agent (proposed: 30/min, burst 10). Not a security
  boundary — it makes continuous silent monitoring impractical and bounds disk
  growth.
- **Retention** — reuse `prune_old_captures`; ensure it covers the new paths.
- **Kill switch** — one host setting reverts every tier to T0-only. If this
  design proves wrong in practice there must be a single lever, not a revert.

## 8. Explicitly out of scope

**This does not prevent exfiltration, and cannot.** An agent that can capture a
screenshot can send it anywhere; it already has `Bash` and network access.
Every control here is about *attribution and consent*, not containment of the
resulting image. Claiming otherwise would be security theatre. This mirrors the
reasoning already accepted for cross-instance targeting in
`ANALYSIS_AGENT_UI_AUTOMATION_CROSS_PANE_AND_CROSS_INSTANCE_TARGETING_2026_08_21.md`
§1.2: the raw capability is already available to anything running on the
machine, so the question is whether AgentMux offers it *accountably*, not
whether it exists.

`UIClick` cross-agent targeting is **not** covered here. Reading and acting are
different risks — the original spec's strongest concrete objection was clicking
"Confirm" on another agent's destructive dialog, which no screenshot can do.
That deserves its own decision.

## 9. Phasing

| Phase | Contents | Unblocks |
|---|---|---|
| **1** | Tier resolution + `allow` defaults; extend audit with tier/target/hash | The immediate ask — agents can capture anything below T3 |
| **2** | Notification (indicator + transcript event) | The non-silence property |
| **3** | `data-amx-redact` + T1 CDP routing | Plaintext-secret masking where it is possible |
| **4** | Rate limit, retention, kill switch | Containment |

Phase 1 alone satisfies the directive and is roughly a day's work: the tier
predicate replaces `!is_self`, and the audit schema grows. Phases 2–4 are what
make it *safe* rather than merely permitted, and should not be skipped.

## 10. Open questions for the repo owner

1. **T3 default.** Closed here. It is the only tier crossing a human boundary;
   confirm that is the intent, since a shared build machine is a plausible
   AgentMux deployment.
2. **T4 notify.** Currently no. Capturing a user's own non-AgentMux windows
   (their browser, their password manager) is silent. Audited, but silent.
3. **Retrofit.** Should `FocusWindow`'s existing unchecked `window_id` — the
   pre-existing hole §6 of the old spec cited as its own motivation — be
   brought under the same tier/audit model, or left alone?

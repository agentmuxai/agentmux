# Browser pane — camera (getUserMedia video) access

**Date:** 2026-09-01
**Status:** Proposal. Not implemented.
**Owner:** unassigned
**Issue:** #2871 (AgentX-asaf) — "Browser pane: camera (getUserMedia video)
access is unconditionally denied"
**Scope:** `agentmux-cef/src/client/handlers.rs` (permission handler +
`permission_handler()` getter), `agentmux-cef/src/client/mod.rs`
(`new_with_browser_pane`), `agentmux-cef/src/browser_pane/creation.rs`,
`agentmux-cef/src/browser_pane/creation_views.rs`, a new grant store in
`agentmux-srv`, and a prompt surface in the frontend.
**Related:** `SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md` §Phase 4 — specifically its
`4a. CEF mic-permission handler` (the handler this extends) and
`4b. OS-permission UX (frontend)` (the layer-B precedent in §3.6) — #1591/#1602, `SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md`,
`SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md` (per-pane surface
precedent)

---

## 1. Current behaviour (verified)

A browser pane cannot grant camera access to any page, ever. Two independent
reasons, both deliberate:

1. **No handler at all on browser panes.** `permission_handler()`
   (`handlers.rs:74`) returns `None` when `self.is_browser_pane`, so CEF's
   default Alloy-runtime behaviour applies — which is to **deny** media access.
   The inline comment is explicit about why: panes load arbitrary web content,
   and auto-granting would hand the mic to any site with no prompt.
2. **Video is never granted even where a handler exists.** On the main app
   client, `AgentMuxPermissionHandler::on_request_media_access_permission`
   (`handlers.rs:~685`) computes
   `allowed = requested_permissions & (DEVICE_AUDIO_CAPTURE | DESKTOP_AUDIO_CAPTURE)`.
   The comment at `handlers.rs:669` states the intent: *"we never want camera."*

**The forcing constraint.** Per the CEF contract quoted in that same comment,
for a `getUserMedia` request `allowed_permissions` must **match**
`required_permissions`. Granting a subset is not a partial grant — it is a
denial of the whole request. So a page asking for `{audio: true, video: true}`
is denied outright today, not silently downgraded to audio-only.

This is the single most important fact for the design below: **you cannot
decide audio and video independently for one request.** Any permission model
that presents them as separate toggles will produce a UI that lies.

## 2. Correcting the premise of #2871

The issue says the mic work "already solved the harder half of this problem for
audio: per-pane origin-scoped grants."

**It did not.** There is no origin scoping and no grant store anywhere in the
codebase today. The handler captures `requesting_origin` **only to log it**
(`tracing::info!(target: "voice", %origin, …)`) and then grants the audio bits
unconditionally for any origin. What Phase 4's §4a actually delivered is the
*handler plumbing* — a `wrap_permission_handler!` block wired to the main
client — not a policy layer.

This matters for scoping: the issue implies camera is "the same shape, for
video," i.e. mostly reuse. In fact the grant model, its storage, its prompt UI,
and its per-pane threading all have to be built from scratch. The reusable part
is roughly the twenty lines of CEF wrapper.

## 3. Design

### 3.1 The decision unit is the whole request, not "camera"

Because of §1's exact-match constraint, the handler must resolve one question:

> *May origin `O`, in pane `P`, capture `{the exact set of devices it asked
> for}`?*

Answer yes → `cb.cont(requested_permissions)` (exact echo). Answer no →
`cb.cont(0)`. There is no middle.

Consequence for the UI: the prompt must name what was actually requested
("**example.com** wants to use your **camera and microphone**"), and a stored
grant must be keyed by the requested bitmask, not by a generic "camera" flag.
A later `{video}`-only request from an origin previously granted
`{audio, video}` is a *subset* and may reuse the grant; a request for a
superset must re-prompt.

### 3.2 Grant identity: `(pane, origin, bits)`

- **Pane-scoped, not global.** Two browser panes on the same site are two
  independent trust decisions; a grant made in one must not silently apply to
  the other. This matches how the panes already behave for other per-pane
  state.
- **Origin-scoped, not per-URL.** Standard web permission granularity;
  anything finer re-prompts on every navigation within a site.
- **Not persisted across pane close, in v1.** A grant lives for the life of the
  pane. Persisting grants is a separate decision with a materially different
  threat model (see §4) and should not ride along in v1.

### 3.3 Threading pane identity into the client

Today `AgentMuxClient::new(handler, /* is_browser_pane */ true)`
(`browser_pane/creation.rs:164`, `creation_views.rs:124`) gives the client a
*boolean*. The handler therefore knows it is *a* browser pane but not *which*
one, so it cannot look up a per-pane grant.

`is_browser_pane: bool` needs to become something carrying the pane's
`block_id` (an `Option<String>`, or a small `PaneIdentity`). Every existing
`is_browser_pane` branch (`handlers.rs:50, 57, 67`) becomes an `is_some()`
check. This is mechanical but touches the client's construction signature in
three places.

### 3.4 Asynchronous decision

`on_request_media_access_permission` must return `1` (handled) and invoke
`cb.cont(...)` — but the prompt is asynchronous. The design depends on CEF
permitting the `MediaAccessCallback` to be **retained and called later**, off
the initial callback return.

**This must be verified before implementation** (see §7 Q1). If CEF requires a
synchronous decision, the whole design changes shape: the only options become a
pre-granted allowlist configured *before* navigation, or an in-page interstitial
that re-triggers `getUserMedia` after the user opts in. Do not begin building
§3.5 until this is settled — it is the load-bearing assumption.

Regardless, the handler must be robust to the pane closing while a prompt is
open: dropping the callback without calling it must not leak, and the pane's
teardown must resolve any outstanding request as denied.

### 3.5 Prompt surface

The existing `userinput` WPS event + `UserInputModal` (window-scoped, see
`initGlobalEventSubs` in `global.ts`) is the closest existing prompt mechanism
and is the natural first candidate, since it already handles a
backend-initiated, window-scoped, modal question.

Requirements the prompt must meet:

- Names the **origin** and the **exact devices** requested (§3.1).
- Is unmistakably attributed to the *page*, not to AgentMux — a permission
  prompt that looks like an app prompt trains users to accept them.
- Default action is deny; deny must be reachable by Escape and by clicking away.
- Must not be spoofable by page content. This is the reason to prefer a
  native/chrome-level surface over an in-pane DOM overlay, and is worth
  weighing against the pane-overlay clip machinery
  (`SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md`) which browser-pane overlays
  already have to negotiate.

### 3.6 The OS layer is a second, independent gate

Granting at the CEF layer is necessary but not sufficient: Windows Privacy
settings and macOS TCC can still deny, and that denial surfaces as a
`getUserMedia` rejection *inside the page*, which AgentMux cannot intercept.

The mic path handles the equivalent case by classifying the error
(`whisperVoiceEngine.ts:335` — `NotAllowedError`/`SecurityError` →
`"not-allowed"`) and surfacing it in its own UI. **That option does not exist
here**, because the failing `getUserMedia` call belongs to arbitrary third-party
page code, not to AgentMux's.

So the realistic contract is: AgentMux grants at the CEF layer, and if the OS
denies, the *page* shows whatever it shows for a denied camera. The most
AgentMux can do is detect the OS-level state proactively and warn before or
alongside the prompt. Scope that as a nicety, not a requirement.

### 3.7 Revoke must terminate an *active* capture — and only one mechanism can

This is the sharpest constraint after §1's exact-match rule, and it is easy to
miss: **the permission handler and the grant store only govern whether a
capture may _start_.** Deleting a `(pane, origin, bits)` grant does nothing to
`MediaStreamTrack`s CEF has already handed to the page. A "revoke" that only
clears the grant would leave the camera live while the UI claims it is off —
strictly worse than offering no revoke at all.

CEF exposes no API to terminate an in-flight media capture. The options, and
why only one is sound:

| Mechanism | Verdict |
|---|---|
| Delete the grant | **Insufficient alone** — governs future requests only. |
| Inject JS to call `track.stop()` | **Unsound.** Requires enumerating every track the page holds, which is not generally possible, and a hostile or merely complex page can retain references or immediately re-acquire. Never rely on page cooperation for a security control. |
| CDP (`Browser.resetPermissions` et al.) | **Unverified, and unused here** — there are no `ExecuteDevToolsMethod` call sites in the codebase today. CDP permission reset conventionally affects *subsequent* requests, so it plausibly has the same gap. Worth measuring, not assuming. |
| **Destroy the page context** — reload the pane | **The only guaranteed stop.** Navigation tears down the JS context and every stream it owns. |

Revoke is therefore specified as: **drop the grant, then reload the pane** via
the existing per-pane primitive (`browser_panes/navigation.rs:72`,
`reload(block_id)`), with the grant dropped *first* so the reloaded page's
re-request re-prompts instead of silently reacquiring.

**This has a real user-visible cost and the UI must own it.** Reloading discards
page state — a half-filled form, a call in progress, scroll position. The revoke
affordance must say so before acting ("Stop camera and reload the page?"), and
the §4 capture indicator must not imply a free, instant toggle.

If Phase 0's investigation finds a genuine track-termination path, this becomes
a softer default with reload as the fallback. Until then reload *is* the
specification, not an implementation detail — the §4 revoke guarantee depends
entirely on it, and Phase 3 cannot be built without this decision made.

## 4. Threat model — camera is not mic

Two asymmetries argue for camera being strictly more conservative than the
existing audio grant:

1. **Passive capture is more revealing.** A hot mic leaks a room's audio; a hot
   camera leaks the user's face, screen surroundings, and physical location
   cues, continuously and unambiguously.
2. **The blast radius is arbitrary web content.** The existing audio grant is
   on the *main app client* — first-party AgentMux UI. This spec proposes
   granting to pages the user browsed to, which is a categorically different
   trust boundary. The `permission_handler()` comment already names this exact
   reason for returning `None`.

Therefore, non-negotiable for v1:

- **No silent grants.** Every first grant for a `(pane, origin, bits)` triple is
  an explicit user action. No settings toggle that pre-grants all panes.
- **Visible while live.** An active capture must be indicated on the pane
  itself, persistently, and must be visible without focusing the pane.
- **Revocable.** One click to revoke, which must actually stop the stream, not
  merely prevent future grants.
- **Deny is the default and the failure mode.** Any error, ambiguity, or
  unresolvable state resolves to deny.

## 5. Desktop capture

`DESKTOP_AUDIO_CAPTURE` is already granted on the main client.
`DESKTOP_VIDEO_CAPTURE` (screen sharing) is **out of scope** for this spec and
must not be folded in as "the other video bit."

It is a different feature with a different consent model — the user picks a
*surface* (a window, a screen) as part of consenting, which camera does not
require, and Chromium normally provides a picker AgentMux does not currently
host. Scope it separately or not at all; do not let it ride along because the
bitmask is adjacent.

## 6. Phasing

**Phase 0 — verify the async callback contract** (§3.4). Blocking, and cheap: a
throwaway build that retains the callback and calls it a second later against a
test page settles the whole design's shape. **Do not proceed without this.**

**Phase 1 — pane identity.** Thread `block_id` into the browser-pane client
(§3.3). Mechanical, independently reviewable, no behaviour change.

**Phase 2 — handler + in-memory grants + prompt.** The v1 feature: session-only,
pane-scoped, origin-scoped, explicit-prompt grants (§3.1–3.5), with the §4
guarantees.

**Phase 3 — capture indicator + revoke** (§4). Deliberately *not* deferred past
v1 in spirit — if Phase 3 is not shipping, Phase 2 should not ship either. Split
only for review size.

**Later, separately:** persisted grants, a Settings surface for reviewing and
revoking them, and any desktop-capture work.

## 7. Open questions

1. **Can the `MediaAccessCallback` be retained across the handler's return?**
   Load-bearing (§3.4); answered by Phase 0.
2. **Is `DEVICE_VIDEO_CAPTURE` really `1 << 1`?** The existing code hardcodes
   `1 << 0` and `1 << 2` for the audio bits with a comment that the values are
   ABI-stable. The video bits are almost certainly `1 << 1` / `1 << 3`, but this
   spec deliberately does **not** assert them — confirm against the CEF headers
   rather than inferring from the pattern.
3. **What happens to an in-flight grant on navigation?** A same-origin
   navigation plausibly keeps it; a cross-origin navigation must drop it.
   Confirm which navigation callback is authoritative here.
4. **Does an active capture survive a pane being backgrounded or a tab switch?**
   Browser panes are native surfaces with their own lifecycle; a camera that
   keeps capturing while the pane is not visible is a surprise worth deciding
   deliberately rather than inheriting.
5. **Is there demand?** #2871 motivates this generically (video conferencing,
   QR/barcode scanners) rather than from a specific blocked workflow. Given §4's
   cost, one concrete user-facing use case should be identified before Phase 2
   is funded. Phases 0 and 1 are cheap enough to do regardless.

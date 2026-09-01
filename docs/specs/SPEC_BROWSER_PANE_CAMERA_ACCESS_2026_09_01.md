# Browser pane — camera (getUserMedia video) access

**Date:** 2026-09-01
**Status:** implemented — shipped 2026-09-01 across PRs #2893 (pane identity),
#2896 (identity TOCTOU fix), #2895 (grant store), #2897 (permission handler),
#2899 (prompt, live capture indicator, revoke). Phase 0 was answered from the
CEF headers in #2885. Out of scope and NOT shipped: persisted grants (§3.2 —
session-only by design) and desktop capture (§5). The prompt round-trip is
unit-tested and reasoned-about but has **not** been observed end-to-end — see
#2899's merge comment.
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

## 1. Behaviour BEFORE this spec shipped (verified at the time)

> **This section describes the state this spec set out to change, not current
> behaviour.** As of the PRs listed in the Status line, a browser pane installs
> `AgentMuxPanePermissionHandler` and consults the grant store, and camera
> access is obtainable with an explicit user grant. §§1-2 are kept as the
> problem statement — rewriting them to describe the fix would erase why the
> design is shaped the way it is.

A browser pane could not grant camera access to any page, ever. Two independent
reasons, both deliberate at the time:

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

**RESOLVED 2026-09-01 — yes, and the design is viable.** Verified against the
CEF source on this machine (`cef/include/cef_permission_handler.h`), so this is
no longer an assumption:

> *"Return true and call `CefMediaAccessCallback` methods **either in this
> method or at a later time** to continue or cancel the request. Return false
> to proceed with default handling. […] With Alloy style, default handling will
> deny the request."*

`CefMediaAccessCallback` is `public virtual CefBaseRefCounted` and its own
header describes it as *"used for asynchronous continuation of media access
permission requests."* Retaining the `CefRefPtr` across the handler's return
and continuing it after a prompt resolves is the API's intended usage, not a
workaround. The fallback designs contemplated here (pre-navigation allowlist,
in-page interstitial) are therefore **not needed**.

Two things this same paragraph of the header settles for free:

- **Returning `false` is the deny path under Alloy**, which is exactly what
  browser panes get today by having no handler at all — so the current
  behaviour is the header's documented default, not an accident.
- **`--enable-media-stream` bypasses this handler entirely** and grants all
  permissions: *"This method will not be called if the `--enable-media-stream`
  command-line switch is used."* See §3.8 — this is a **live ingress, not a
  hypothetical**, and correcting that is a prerequisite for §4 meaning
  anything.

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

**Checked 2026-09-01:** CEF's public headers expose no media-capture
termination API — no `MediaStream`/`StopMediaCapture` surface in
`cef_browser.h` or `cef_permission_handler.h`. So reload is not merely the
recommended option, it is the only one CEF offers today. If a CDP path is ever
shown to genuinely terminate live tracks (§7 Q6), this can become a softer
default with reload as the fallback; until then reload *is* the specification,
not an implementation detail — the §4 revoke guarantee depends entirely on it.

### 3.8 `--enable-media-stream` must be rejected at every ingress

An earlier revision of this spec said AgentMux "does not pass
`--enable-media-stream` today (verified: no occurrence in `agentmux-cef` or
`agentmux-launcher`)" and treated it as a standing constraint on future edits.
**That was true about hardcoded occurrences and misleading about the actual
risk.** The switch does not need to appear in our source to reach CEF.

Verified ingress paths, both unfiltered today:

1. **`AGENTMUX_CEF_EXTRA_FLAGS`** (`agentmux-cef/src/app/mod.rs:756-770`) —
   splits an env var on whitespace and appends **every token verbatim** as a
   Chromium switch. It exists to A/B GPU flags without a recompile, and has no
   allowlist, denylist, or validation of any kind. `AGENTMUX_CEF_EXTRA_FLAGS=--enable-media-stream`
   is sufficient.
2. **Launcher → host argument forwarding** — `agentmux-launcher/src/main.rs:309`
   collects the launcher's own CLI arguments verbatim
   (`std::env::args().skip(1).collect()`) and they reach the host process
   through `.args(args)` in `host_spawn.rs` (`:41` supervised, `:138` unix),
   with no filtering on what may pass. So anything on the launcher's command
   line arrives at CEF.

   (An earlier revision cited `host_spawn.rs:79` here. That is wrong and worth
   correcting explicitly, since a reader auditing this would have gone to the
   wrong place and concluded the claim was unfounded: `:79` is a *conditional,
   internally-controlled* `--disable-gpu` added from retry state, not arbitrary
   pass-through.)

So an environment variable — settable by a user, a shell profile, a CI config,
or **an agent with shell access** — silently grants camera and microphone to
**every browser pane, for every origin, with no prompt**, and does so by
disabling the handler rather than by going through it. Nothing in §3's design
observes it, and nothing in §4 survives it.

**Requirement:** before Phase 2 ships, `--enable-media-stream` (and its
`enable-media-stream=…` spelling) must be **stripped at every ingress**, with a
loud warning when rejected. The check belongs at the switch-assembly site in
`app/mod.rs`, since that is where both paths converge into `CefCommandLine`.

Two judgement calls for whoever implements it:

- **Reject the exact switch, or allowlist `AGENTMUX_CEF_EXTRA_FLAGS` entirely?**
  A denylist of one is easy to defeat by accident as Chromium adds aliases; an
  allowlist is stricter but breaks the flag's diagnostic purpose. A denylist
  covering the media switches specifically, with the warning, is the
  proportionate answer — but note this is a **diagnostics escape hatch being
  relied on as a security boundary**, which it was never designed to be.
- **Is this a pre-existing bug, independent of camera?** Partly yes: the same
  env var can already grant **microphone** to every browser pane today, without
  any of this spec being implemented. That is arguably worth fixing on its own
  schedule rather than waiting for Phase 2 — see §7 Q7.

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

**~~Phase 0 — verify the async callback contract~~ — DONE (2026-09-01).**
Answered from the CEF headers without needing a build; see §3.4 and §7 Q1/Q2.
The async design is viable and the bitmask values are confirmed. Phase 1 is now
the entry point.

**~~Phase 1 — pane identity~~ — DONE (#2893, TOCTOU fix #2896).** Shipped
*differently from §3.3's design*: threading `block_id` into the client at
construction would have silently reclassified prewarmed pool panes, which are
created with no identity and only become a specific block's pane at promote. It
resolves identity at request time instead (`AppState::block_id_for_browser`),
which needs no construction-site changes and works the moment a pane registers.
§3.3 is kept as written for the reasoning; the implementation note here is the
correction.

**~~Phase 2 — handler + in-memory grants + prompt~~ — DONE.** Split for review:
the grant store (#2895), the handler wired to it (#2897, behaviour-neutral —
nothing could create a grant yet), then the prompt (#2899).

**~~Phase 3 — capture indicator + revoke~~ — DONE (#2899), shipped with Phase 2**
as this section required. The indicator is driven by CEF's own
`OnMediaAccessChange` rather than by grants, so it means "is capturing now"
rather than "may capture".

**Still not built, deliberately:** persisted grants (§3.2 — a materially
different threat model), a Settings surface for reviewing and revoking grants
across panes, and any desktop-capture work (§5).

**Not verified end-to-end.** The prompt round-trip — prompt appears, user
allows, capture starts, indicator lights, Stop reloads — is unit-tested and
reasoned-about but has never been observed in a running build; that needs a
person driving a `getUserMedia` page in a pane. The `pane-media` log target
traces every decision step if it misbehaves.

## 7. Open questions

1. ~~**Can the `MediaAccessCallback` be retained across the handler's return?**~~
   **Answered — yes.** See §3.4; the header states the callback may be invoked
   *"either in this method or at a later time."* This was the design's
   load-bearing assumption and it holds.
2. ~~**Is `DEVICE_VIDEO_CAPTURE` really `1 << 1`?**~~ **Answered — yes**,
   confirmed against `cef/include/internal/cef_types.h`:
   `DEVICE_AUDIO_CAPTURE = 1 << 0`, `DEVICE_VIDEO_CAPTURE = 1 << 1`,
   `DESKTOP_AUDIO_CAPTURE = 1 << 2`, `DESKTOP_VIDEO_CAPTURE = 1 << 3`.
   The inferred pattern was right, but it is now checked rather than assumed —
   which is the standard a permission bitmask deserves.
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
   is funded. Phase 1 (threading pane identity) is cheap enough to do regardless,
   and Phase 0 is already done.

6. **Can CDP terminate a live capture?** §3.7 rules out everything except
   reloading the pane, on the basis that CEF exposes no termination API
   (confirmed) and that CDP permission resets conventionally affect only
   subsequent requests (assumed). If a CDP method genuinely stops in-flight
   tracks, revoke could stop being destructive. Worth one experiment, but note
   the codebase has **no `ExecuteDevToolsMethod` call sites at all** today, so
   this would introduce a new dependency surface for one feature — weigh that
   against simply accepting the reload.

7. **Is the `--enable-media-stream` ingress (§3.8) a pre-existing bug?** It can
   already grant microphone to every browser pane today, with no part of this
   spec implemented — the existing main-client audio grant is not the only way
   media reaches a pane. If so it should be fixed on its own schedule rather
   than as Phase 2 scaffolding, and this spec should depend on that fix rather
   than own it.

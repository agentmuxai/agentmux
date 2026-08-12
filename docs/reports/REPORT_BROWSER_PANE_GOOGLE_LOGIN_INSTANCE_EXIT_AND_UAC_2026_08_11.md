# REPORT — Browser-pane "Continue with Google" exits the whole AgentMux instance (+ a Windows UAC prompt)

**Date:** 2026-08-11
**Author:** Lark
**Status:** Investigation / root-cause. No code changed — this is a written report per request.
**Trigger (verbatim):** "the browser pane will close the entire agentmux instance if I login to anthropic via login with google. the entire agentmux instance will exit … plus I got a UAC from windows … we do not want ANY uacs … take a look, consider the previous work, write a report to file on why it is happening."

**Repro:** Open `claude.ai` in a **browser pane** → click **Continue with Google** → complete the Google sign-in. The pane vanishes and the entire AgentMux window/process exits; separately, Windows shows a UAC elevation prompt during the flow.

---

## TL;DR

There are **two independent bugs** here, chained by one design decision. Both are reproduced in the host log for this exact flow (`~/.agentmux/dev/naki-copy-button-false-positive-fix/…/agentmux-host-v0.55.3.log.2026-08-11`, 13:59:38 → 14:00:52).

1. **Why the pane closes (and login never completes):** Google sign-in from `claude.ai` opens a **GIS (Google Identity Services) popup** via `window.open(...)` with `display=popup&response_mode=form_post`. AgentMux's `on_before_popup` (`agentmux-cef/src/client/lifecycle.rs:524`) intercepts every popup and, **for browser panes, navigates the pane's own main frame to the popup URL** instead of opening a real popup. The GIS flow completes on `accounts.google.com/gsi/transform` and then calls **`window.close()`** as the normal last step of its popup handshake. Because the "popup" is actually the pane's own top-level browser, CEF honors `window.close()` and **destroys the entire browser pane** (`on_before_close` → "browser pane drained"). The credential is also never delivered to `claude.ai` — the opener↔popup `postMessage` handshake can't work when the frame *is* the opener, so the login silently fails too.

2. **Why closing that pane takes down the whole app:** destroying the pane arms the **last-window quit watchdog** (`agentmux-cef/src/wrr/win_event.rs`). The watchdog uses OS window visibility as its tiebreaker. At that moment the **main AgentMux window is hidden/parked offscreen** (`visible=false`, rect `-32768,-32768`) — a state left over from the browser-pane **airspace-hide** mechanism (`ShowWindow(SW_HIDE)`, `browser_pane/wrapper.rs:321`) whose *restore* was tied to the pane that just got destroyed. The watchdog sees "0 visible windows" for its full grace budget, **disagrees with the reducer** (which still says `registered=1`), exhausts its 3 lag-retries, and then **quits the message loop "on OS signal alone (reducer desync, investigate)"** (`win_event.rs:542`) — killing the process. The code literally logs this as a bug-report-to-self.

3. **Why the UAC:** AgentMux never requests elevation itself (both host and launcher manifests are `asInvoker` — `agentmux-cef/build.rs:24`, `agentmux-launcher/build.rs:23`). The UAC is **Chromium's default external-protocol handling firing inside the embedded pane**: `AgentMuxRequestHandler` implements only `on_render_process_terminated` and `auth_credentials` — there is **no `on_before_browse` and no external-protocol / `on_protocol_execution` guard anywhere in `agentmux-cef`** (confirmed by grep). So when the in-pane Google/Anthropic flow triggers a navigation to a non-http external protocol (an OS-registered app handler), CEF hands it straight to the Windows shell, and if that handler is an elevated target, Windows shows UAC. This is a direct consequence of the same decision in bug 1: the OAuth flow runs *inside* the embedded browser instead of the system browser, so the pane inherits every native-handoff the real browser would have taken — without the guardrails a real browser has. (See §4 for the evidence I have and the one piece I can't get from logs alone.)

**One decision underlies all three:** browser panes run third-party auth flows (popups, external protocols) *in-frame, inside the embedded CEF pane*, rather than routing them to the system browser the way the app UI's external links already are (`lifecycle.rs:549-559`).

---

## 1. Evidence — the exact sequence from the host log

Instance: `dev:naki-copy-button-false-positive-fix`, host v0.55.3, pid 62368. Timestamps below are from that log.

| Time | Log line (abridged) | What it means |
|---|---|---|
| 13:59:38.313 | `popup intercepted — deferred navigation of current frame … is_browser_pane=true url=https://accounts.google.com/o/oauth2/v2/auth?…&origin=https%3A%2F%2Fclaude.ai&display=popup&response_mode=form_post` | claude.ai's GIS popup was cancelled; the pane's **own frame** was navigated to the Google OAuth URL. (`lifecycle.rs:569`) |
| 13:59:38 – 14:00:40 | repeated `[pane-airspace] applied overlay clip to pane HWNDs`, `giveFocus closed=false` | user completing Google login inside the pane; airspace/overlay management active throughout |
| 14:00:40.765 | `state-write key=url value="https://accounts.google.com/gsi/transform"` | GIS reached its transform/close page — the step that self-`window.close()`s |
| 14:00:40.768 | `Unregistered browser: label=browser-pane-e686038f-… (remaining: 4)` then `browser pane drained via on_before_close` | CEF honored `window.close()` and destroyed the **whole pane browser** |
| 14:00:40.775 | `[wrr] arming 3000ms quit watchdog (reducer counts 1 live) — will re-check visibility on fire` | pane close armed the **last-window** quit watchdog |
| 14:00:43 / :46 / :49 | `quit watchdog: 0 visible for 3000ms but registered=1 draining=false … re-arming (1/3 … 3/3)` | for 9s straight, **0 top-level windows visible**, but the reducer still says a window is registered |
| 14:00:52.780 | `diag hwnd=… class=Chrome_WidgetWin_1 title="AgentMux" visible=false … rect Rect{left:-32768,top:-32768,…}` (×2) | the main AgentMux window(s) are **hidden and parked offscreen** |
| 14:00:52.780 | `quit watchdog fired: 0 visible for 3000ms (retries=3) but reducer disagrees (registered=1 draining=false) — quitting on OS signal alone (reducer desync, investigate)` | watchdog gives up and **quits the whole process** |
| 14:00:52 | `CEF message loop exited, shutting down` → `Killing backend sidecar` → `AgentMux host shutdown complete (fast exit)` | instance exit |
| 14:01:40 | `AgentMux host starting …` | launcher relaunches a fresh host |

The `display=popup&response_mode=form_post&origin=https://claude.ai` querystring is the signature of a **Google Identity Services popup sign-in** — a `window.open` popup that communicates its result back to the opener and then closes itself. That self-close is what lands on the pane.

---

## 2. Bug 1 — `on_before_popup` navigates a browser pane's own frame into third-party auth popups

`agentmux-cef/src/client/lifecycle.rs:524-575`. The current logic:

- **App UI → external site** (Help pane links, etc.): open in the **system browser**, cancel the popup (`is_external_http_url` branch, lines 549-559). Correct — this is exactly the safe path.
- **Browser pane, or internal URL**: `post_task` a **deferred `load_url` of the current frame** to the popup's target URL (lines 561-568), then cancel the popup.

The comment's rationale — *"in a browser pane following a link IS the point"* — is right for ordinary `target="_blank"` links. It is **wrong for OAuth/GIS popups**, which are not "links to follow" but a separate short-lived window that:

1. must stay **separate** from the opener (claude.ai) so the two can `postMessage` credentials across the window boundary, and
2. **self-closes** (`window.close()`) when done.

By collapsing that popup into the pane's own frame, AgentMux breaks (1) — so the login result never reaches claude.ai — and turns (2) into "destroy the pane." Both symptoms the user saw ("login doesn't work" is implicit; "the pane closes" is explicit) fall out of this one substitution.

**Note this is pane-specific.** The main app UI already does the right thing (system browser). Only browser panes take the in-frame path, and only because the same handler serves both.

---

## 3. Bug 2 — closing a pane can quit the app when the main window is in an airspace-hidden state

`agentmux-cef/src/wrr/win_event.rs:502-548`. The last-window quit watchdog exists to quit AgentMux when the final user window closes. It cross-checks two signals:

- **Reducer state**: `registered` (windows the app knows about) and `draining` (a close is in progress).
- **OS state**: how many app-class top-level windows `EnumWindows` reports as *visible*.

When these disagree — `registered=1, draining=false` (reducer says a live window exists, nobody asked to close it) but OS says `0 visible` — the watchdog grants a **bounded lag-retry budget** (3 × 3s) to ride out transient `EnumWindows` misreads from window-pool churn (per `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` and `PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md`). If visibility never recovers, it **trusts the OS signal and quits** (line 548).

Here the disagreement was **not** a transient misread — the main window was **genuinely hidden** for the entire budget. The `diag_dump_app_windows` output at quit-fire shows every `Chrome_WidgetWin_1 title="AgentMux"` at `visible=false`, rect `-32768,-32768`. That offscreen-park + `SW_HIDE` is the **browser-pane airspace-hide** state (`browser_pane/wrapper.rs:321` `ShowWindow(wrapper_hwnd, SW_HIDE)`; `browser_panes/mod.rs:158`; `browser_panes/clip.rs:603` references "the frontend freeze-frame that compensated for it"). The airspace mechanism hides/clips window HWNDs while a browser pane's native surface is composited, and **restores them afterward**. When the pane was destroyed mid-flight by the rogue `window.close()` (bug 1), the restore that was supposed to bring the main window back **was tied to the very pane that just vanished** — so the main window was left hidden, the watchdog saw 0 visible, and quit.

So bug 2 is a real latent defect independent of Google: **a browser pane closing while the main window is in an airspace-hidden state can strand the main window hidden, which the quit watchdog then reads as "last window gone."** Bug 1 is just the most reliable way to trigger it, because GIS's `window.close()` closes the pane at an arbitrary moment the airspace system didn't initiate.

---

## 4. Bug 3 — the UAC: no external-protocol guard on the embedded pane

**What's certain from the code:**

- AgentMux does not request elevation. Host and launcher both ship `<requestedExecutionLevel level="asInvoker" uiAccess="false"/>` (`agentmux-cef/build.rs:24`, `agentmux-launcher/build.rs:23`), and there is no `runas` verb, `ShellExecuteEx`, `SEE_MASK_*`, or elevation call anywhere in `agentmux-cef` / `agentmux-launcher` / `agentmux-common` (grep clean).
- `AgentMuxRequestHandler` (`agentmux-cef/src/client/handlers.rs:401-439`) implements **only** `on_render_process_terminated` and `auth_credentials`. There is **no `on_before_browse`, no `on_open_url_from_tab`, and no external-protocol / `on_protocol_execution` / `allow_os_execution` handler** anywhere in the crate (grep clean). Browser panes therefore run under **CEF's default external-protocol behavior**, which on Windows delegates unrecognized schemes to the OS shell.
- The app's *own* safe URL dispatcher (`open_url_in_default_browser`, `platform.rs:503-528`) is **not** on the pane path — it's only reached for app-UI external links and the `open_external` IPC, and it hard-allowlists `http/https/devtools/vscode`. Panes bypass it entirely.

**The mechanism:** running the Google→Anthropic auth flow *inside* the embedded pane (bug 1's consequence) means the pane can be navigated to a non-http external protocol — a native-app deep link or OS handler that these flows sometimes invoke (e.g. a passkey/security-key path, a `ms-…`/vendor scheme, or another registered protocol app). With no `on_before_browse` guard, CEF hands that scheme to the Windows shell; if the registered handler is an elevated target, Windows raises **UAC**. A real external browser either sandboxes these or prompts differently; the embedded pane, lacking the guard, does not.

**What I could not confirm from logs alone (stated honestly):** the specific scheme/handler that produced the UAC is **not** in the host log — a UAC is an OS-side `consent.exe` dialog, and CEF's default external-protocol dispatch doesn't emit an AgentMux log line (precisely because nothing in our code handles it). So while the *absence of any external-protocol guard* is a certain, code-verified defect and the most probable UAC vector, the exact triggering handler is a hypothesis, not a logged fact. **To capture it deterministically:** reproduce with **Process Monitor** filtered to `Process Name is consent.exe` (or parent = `agentmux-cef.exe`), or check **Event Viewer → Security** for the `consent.exe` elevation request around the repro time — that will name the exact executable Windows tried to elevate. Adding an `on_before_browse` that logs-and-blocks non-http schemes (see §5) would also surface it in our own logs going forward.

---

## 5. Fix direction (for a follow-up spec/PR — not implemented here)

Ordered by leverage. Each is independently shippable.

1. **Don't collapse auth popups into a pane's frame — route them to the system browser (or a real child popup).** In `on_before_popup`, detect the popup case for browser panes and either (a) open it in the system browser via the existing allowlisted `open_url_in_default_browser`, matching what the app UI already does, or (b) let CEF create an actual popup browser so the opener↔popup handshake and `window.close()` both land on a throwaway window, not the pane. Option (a) is simpler and closes bug 1 and removes the pane's exposure to bug 3's external-protocol handoff in one move. This is the highest-leverage fix — it addresses all three symptoms at their shared source.
2. **Add an `on_before_browse` / external-protocol guard to the pane RequestHandler.** Block (or explicitly, logged, route to the system browser) any non-http(s) scheme a pane tries to navigate to. This is the defense-in-depth that stops *any* embedded content — not just Google — from reaching an OS handler that can UAC, and it makes the offending scheme visible in our logs. Independently valuable even after fix 1.
3. **Harden bug 2 so a pane close can never strand the main window hidden.** Two parts: (a) ensure the airspace-hide **restore** is not owned by an individual pane's lifecycle — on any pane destroy, unconditionally restore main-window visibility (un-`SW_HIDE`, move back on-screen) before the quit watchdog can sample; and (b) in the watchdog's `registered>0 && !draining` branch, treat "a registered window exists but is only *hidden* (not closed)" as **do-not-quit** rather than falling through to "trust the OS signal" — an offscreen/`SW_HIDE` window is still a window. The watchdog already flags this path "reducer desync, investigate"; this is that investigation.

Fix 1 stops the user-visible incident. Fixes 2 and 3 make the two latent defects (any-scheme UAC surface; pane-close-strands-main-window) safe regardless of what content a pane loads.

---

## 6. Relationship to previous work (as requested)

- `SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17.md` / `lifecycle.rs:537-559` already established the correct pattern — **app-UI external links go to the system browser** — precisely to avoid navigating an owned window to a foreign origin and stranding it on "Can't reconnect." Bug 1 is the same class of problem one layer over: the browser-pane branch was deliberately *exempted* from that pattern ("following a link IS the point"), and OAuth popups are the case where that exemption bites.
- `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` and `PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md` built the lag-retry budget specifically to stop the watchdog quitting on transient `EnumWindows` misreads. This incident is the **non-transient** version they didn't cover: the main window was really hidden for the whole budget, so retries just delayed the wrongful quit by 9s rather than preventing it. Fix 3(b) extends their intent to the "hidden, not closed" case.
- The browser-pane **airspace/overlay** system (`browser_panes/clip.rs`, `pane-overlay*`, and the `data-pane-overlay` work in the recently-merged submenu PR #2525) is the same subsystem whose `SW_HIDE`/restore ownership is implicated in bug 2 — worth a look together, since both are about "a native pane's lifecycle transiently controlling main-window/overlay visibility."

---

## 7. Reproduction & verification notes

- **Repro logs:** `~/.agentmux/dev/naki-copy-button-false-positive-fix/3eaacaa32634b401/logs/agentmux-host-v0.55.3.log.2026-08-11`, lines ~7685 (popup) → ~8449 (quit fire). `muxlog host -i naki-copy-button-false-positive-fix` will tail the live instance.
- **To capture the UAC target deterministically:** Process Monitor filtered to `consent.exe` (or parent `agentmux-cef.exe`), or Event Viewer → Windows Logs → Security around the repro timestamp.
- **This report changed no code.** It documents root cause only, per the request.

---

*End of report.*

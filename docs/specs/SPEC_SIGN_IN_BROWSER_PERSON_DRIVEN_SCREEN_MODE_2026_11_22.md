# Spec: a sign-in browser a person drives (the Browser pane's screen mode), production build

**Status:** active - the first pull requests (the core, then windows and the pane) are in review; the later ones follow this list. Public requirements only.

## 1. What this is, and the decision behind it

The Browser pane's streamed browser is built so that agents can drive it, through a control channel attached to the browser. Some sign-ins need the other kind: an ordinary visible Chrome on a virtual screen, **driven only by a person**, with nothing attached to it. The product needs both kinds in the same pane.

Decision (the operator, 2026-11-22): a sign-in browser that only a person can drive is what is wanted. Agent driving of this browser is **not wanted and is not built**. This is a requirement of the design, not a missing feature:

- R1. No debugging attach of any kind (no pipe, no port, no automation or headless switch) and no script put into a page. A test pins the launch arguments.
- R2. No agent-facing way to read or act on this browser: no snapshot, no click or fill by reference, no window list for agents, no profile sharing with an agent's browser. Its profile is its own and is never handed to a browser an agent drives.
- R3. Nothing hides, removes, overrides or injects anything to pass a site's automated-browser check.
- R4. A person types their own credentials and codes. Agents never enter or see them.

The prototype (two draft pull requests) proved the capture, input and policy parts on Linux; this spec turns it into small, mergeable pull requests behind a setting that is off by default.

## 2. Name and how a person reaches it

- It is **the Browser pane in screen mode**; the person-facing label is "Full browser (you drive)". No new product name. In AgentMux Web (the web frontend reached through AgentMux Cloud) and in the desktop app's Browser pane that talks to a server that offers it.
- It is reached from the streamed Browser pane: when a page refuses the streamed browser, or by the pane's menu, the person chooses "Open in a full browser you drive". The pane reconnects with `mode=screen`. The streamed (agent-driven) browser keeps its own profile; the two never share one.
- The setting is off by default: server side `AGENTMUX_STREAMED_BROWSER_SCREEN=1`, and the pane offers the choice only when the server answers that it has the mode. (A later change may move this to a settings key; the default stays off until the unproven items in section 11 are proven.)

## 3. The browser's launch

A plain visible Chrome (or Chromium) on a virtual X screen with a window manager, started the way a person starts it: its own profile folder, the address proxy's network rules (the one way out, as for every pane), a window size, and the page to open. The sandbox switch is only for a demo setup that cannot use the browser's own sandbox, and is only ever asked for by the operator through an environment setting. A browser installed as a snap or flatpak is refused with words (its policy is read from where the check cannot see).

## 4. What it may open: policy per browser instance, never machine-wide

The streamed pane blocks local files, the browser's own pages and developer tools through its debugging channel. Screen mode has none, so the block is Chrome's managed policy: blocked addresses (file, the browser's own pages, developer tools, view-source, javascript, ftp), developer tools off, extension install blocked, browser sign-in off, **no file dialogs, no downloads**.

- R5. The policy is **per browser container, never machine-wide on a machine that also runs a streamed browser**. A policy file at the browser's fixed place binds every Chrome of that kind on the machine; the streamed browser needs its control channel, which the policy's developer-tools setting refuses, so the two cannot share a file. Therefore screen mode runs only where the container or dedicated machine holds **one screen-mode browser and no streamed browser**: the policy folder is baked into the image (or written inside the container before the browser starts), the server never writes a machine-wide file, and a server with screen mode on **serves no streamed browser** in the same process.
- R6. Start-up refusals, each with words: no policy file that holds the required content (the words say what to install and its content); another browser already running in the container; a snap or flatpak browser (its policy is read from where the check cannot see it). A hosted task holds one screen session; a test host may allow a few by a setting.
- R7. There is no shared-host form that wraps the browser in a mount namespace: the browser's own sandbox needs user namespaces, and a host may allow them only to programs with their own profile, which would force the sandbox off (see R7a). A Linux desktop that wants the mode runs the same image in a container.
- R7a. **The browser's own sandbox is on in every form.** No `--no-sandbox` in any production setup. A hosted image that only works with the sandbox switch is not acceptable. Check (on a local container started with the hosted runtime's restrictions, then on the hosted task): no browser command line carries `--no-sandbox`; a renderer process has its own process namespace and a seccomp filter (`Seccomp: 2` in its status). If the runtime's default profile refuses the sandbox's namespaces, the finding goes to the leader at once and the image does not ship.
- R13. **No flag the browser itself calls unsupported.** The browser starts without any switch that makes it show its warning bar (the prototype's name-lookup switch is the known one): the proxy, no-QUIC and no-DNS-over-HTTPS settings come from the container's policy (so the image fixes the address proxy's port), and a test shows the warning bar absent and no name lookup made by the browser itself.
- R8. The window manager runs with settings that bind nothing (no menu, no shortcut, no mouse action, no window frame), so a client's keys and mouse reach the browser and nothing else.
- Tests: unit (the policy's content and its check, the window manager's settings, the start-up refusals of R6, the streamed route closed when screen mode is on); real, on a Linux host with a dedicated policy folder: the policy is shown as applied, a typed local file does not open, Alt-dragging does not move the window, the browser's sandbox is on (R7a), the warning bar is absent and no local name lookup is made (R13).

## 5. Frames and input

As built: a frame is the whole screen as a JPEG (the pane's existing frame message), sent only when the screen changed and at most 15 a second, a frame the pane cannot take is dropped and made again from a newer screen; mouse, wheel and keys reach the screen as a person's pointer and keyboard (the X test extension); a typed address is the checked, normalised address, typed after the address bar has the focus. A US keyboard and a fixed screen size are limits of the first release; a resize reconnects at the new size.

- R14. **Idle cost.** The screen is read only when the screen server reports a change (its damage events), through shared memory, never on a timer; a static page costs at most 5% of one core per session in the whole stack at both sizes, and sends no frame unless pixels changed.
- R15. **Typing latency.** A frame is made at once when an input event arrives, and every ~16 ms for about 300 ms after it, then back to the steady cap (15 a second); the steady rate does not rise. Target: key to changed picture under about 40 ms at the smaller size and under about 60 ms at the larger, on a real text box.

## 6. Windows, the new-window notice and the key gap

A sign-in often opens a second window. The server follows the browser's top-level windows by **X events** (a window mapped or unmapped on the screen), not by polling, so that:

- R9. When a window opens the pane gets one notice, rate-limited ("A new window opened in this browser"), and keys sent after the open are held for half a second so they do not land in a window the person has not seen. The hold begins at the moment the window is mapped, not at the next sample (the prototype polled and could let typing through for a short moment).
- R10. Closing the last window ends the session; the pane says so.

## 7. Clipboard

- Paste into the browser: the person's pasted text is typed into the screen key by key, capped in length (as built).
- Copy out: the selection in the browser can be fetched by the person on request (a "copy from browser" action), returned to the pane as text and placed on the person's own clipboard by the person's action. Nothing is read from the browser unless the person asks. Images and files are not carried.

## 8. Downloads, uploads, file dialogs

Denied, as in the streamed pane (R5's policy). A sign-in does not need them. If a later feature needs a safe path, it gets its own spec.

## 9. The kept profile, sign-out and removal

- The browser's profile is kept per person (the kept profile) so a sign-in survives a closed pane, and is archived on a clean close (the profile folder is closed, then archived; a browser that is still writing is never archived).
- R11. Sign-out and removal are explicit actions in the pane: "Sign out and remove this browser's data". It closes the browser, deletes the profile and its archive, and tells the person in words that signing out of the provider's own account device list is separate and theirs to do if they want it. A browser session never ends silently with its sign-in left on the server.
- R12. The profile is never read by any agent or by another person's session.

## 10. Limits, metering, and where it runs

- One screen session per person, a small number per server (four in the prototype; the figure is set from the measured cost per active session), an idle limit and a longest session, each ending with a notice the person sees first. Screen sessions are metered separately from streamed ones (they cost more per active hour).
- **A hosted workspace or container** gets the Linux image with the virtual screen, the window manager and the browser; the server there owns start, limits and the kept profile. **A desktop** (Windows, macOS): the mode is Linux-only, so the setting stays off and the pane says "not available on this computer"; the desktop's own Browser pane is already an ordinary browser the person drives. A Linux desktop with the tools installed may turn it on.
- Linux tests run on a Linux host; the Windows and macOS builds compile the mode out and keep their tests (protocol, pane and frontend).

## 11. What stays unproven, and how each will be proven

1. The **pop-up flow on a real third-party site** (an account chooser or consent window that opens a second window): proven with a person on a site the operator names, with the window notice and key gap observed. Until then the pane does not promise it.
2. **Durability of a kept profile** over days (a sign-in that survives a close, archive and restore): proven by a person returning after a day and after a week, on the test account, no challenge expected or forced.
3. **Volume**: the cost and the limits at several sessions at once: proven by the measurement tables (CPU, memory, bandwidth, key-to-picture delay at two sizes) and a soak at the chosen limit on a Linux host.
4. **Other accounts and providers**: not claimed; each provider is tested by a person, one attempt, stopping at any challenge.

## 12. The pull requests (each mergeable on its own, behind the off-by-default setting)

1. **Screen mode core** (the first prototype pull request, made real): streamer, screen-level input, locked window manager, **the container's own policy and its start-up refusals** (R5 to R8), no unsupported flag (R13), launch-argument tests, the sandbox-on check (R7a) on a local container.
2. **Windows and pane** (the second prototype pull request, made real): event-driven window tracking, notice, key gap (R9, R10), the pane's opt-in and the "Full browser (you drive)" choice.
2b. **Capture cost and typing latency** (R14, R15): damage-driven capture, shared-memory reads, input-triggered bursts, a faster encode of the changed rectangle; proven by the idle and key-to-picture measurements of the prototype, re-run.
3. **Clipboard** (section 7).
4. **Kept profile, sign-out and removal** (section 9), with the server-side archive; depends on the kept-profile design.
5. **Limits and metering** (section 10), after the measured numbers.
6. **Hosted image and docs**: the Linux image with the screen, window manager and browser, and user docs.

PRs 1 to 3 (and 2b) do not depend on the cost numbers. The two draft prototype pull requests are turned into PRs 1 and 2 (their review history stays), not closed.

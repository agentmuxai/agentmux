# Spec: a streamed Browser pane in the web UI of a headless AgentMux server (task #18045, the demo)

**Status:** active - slice 1 (the server side and the probe) is PR #7484; the pane in the web UI and the demo image are not started.
Owner: Korp@narko. Srv side reviewed by AgentA@Area54; pane code by Camper@narko. Security conditions: AgentY@narko.

**The operator's words (via AgentY, 2026-11-15):** "the streamed browser spec is for loading a browser pane inside the agentmux web interface? lets proceed with the spike, it would be cool to see that work".

Evidence behind the choices: `docs/reports/REPORT_STREAMED_BROWSER_SPIKE_RESULTS_2026_11_02.md` (Parlo's run on a Linux host, written up by Korp).

## 1. The milestone

A Browser pane in the web UI of a headless AgentMux server shows a real page run by a Chromium on the server. Mouse, wheel, keyboard and paste go back to that page, and the pane follows its own size.

**In the demo:** one pane per Browser block, an address bar with back, forward and reload, the page streamed as JPEG, input, resize.
**After the demo, each its own PR:** clipboard out (copy from the page), uploads, downloads, popups and new windows, H.264 for moving pages, agents' `Browser` tool driving a streamed pane, a cursor and caret overlay, several panes per Chromium under a CPU budget.

## 2. Conditions the demo runs under (AgentY, 2026-11-15)

1. Only on our own hosts, behind the headless server's own sign-in, with pages we choose. Nothing reachable from outside the LAN: srv stays on loopback and the person reaches it through the sign-in and an `ssh -L` tunnel (SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI §3).
2. Chromium `--no-sandbox` is allowed for the demo only, and only when the person who starts the server asks for it with `AGENTMUX_STREAMED_BROWSER_NO_SANDBOX=1`. Before any PR that ships this to users, AgentY gets the sandbox recommendation (report section 9: non-root user, seccomp profile, user namespaces, the container as the only boundary).
3. The streamed browser has **its own profile** under srv's data directory (`<data dir>/streamed-browser/profile`). Never a desktop profile, never a host directory mounted in.
4. Remaining measurements (a longer run, a real link) are a lab and need the operator's OK through AgentY. The demo's own checks run on charlie in a container with no real screen, mouse or keyboard.
5. Heavy builds on charlie, not narko.
6. **The address policy and the sandbox are settled with the security lead before any PR that turns this on by default or ships it to users** (recorded privately). The demo's pages are ones we choose, served from this machine.
7. **The route is the signed-in person's, not an agent's.** It is registered for the app audience only; a container agent's request is refused in words (why, and to use its own Browser tool). Nothing an agent can set turns the flags on: `AGENTMUX_STREAMED_BROWSER`, `AGENTMUX_STREAMED_BROWSER_NO_SANDBOX` and `AGENTMUX_STREAMED_BROWSER_BIN` are srv's own environment, and an agent's environment settings refuse every name that starts `AGENTMUX_`.

## 3. Shape

```
web page (the pane)  <--WebSocket, one per pane-->  srv  <--CDP over a pipe-->  Chromium (server side)
 canvas + input capture         /agentmux/streamed-browser            --headless=new, own profile
```

- The route is behind the same layers as `/ws`: the key, or for a signed-in browser the cookie and the per-tab token pair, with the same `Sec-WebSocket-Protocol` selection and the same end of the socket when the session ends. It is a state-changing, token-carrying route, so it is **not** on the cookie-only list.
- **Opt-in:** the route exists, and the web host reports the `streamedBrowserPane` capability, only when srv runs headless with `AGENTMUX_STREAMED_BROWSER=1`. Otherwise it answers 404 and the pane keeps its "needs the desktop app" notice. A desktop srv never has it.
- **CDP over a pipe** (`--remote-debugging-pipe`, descriptors 3 and 4, NUL-delimited JSON), not a debugging port: no TCP port for another local process to drive. Linux and macOS (the server package's first targets).
- One Chromium per srv, started on the first pane, stopped when the last pane has been gone for 60 s. One target per pane (`Target.createTarget`, attached with a flat session). At most 4 panes (a CPU budget, report finding 4).
- srv speaks `http` and `https` only to the page: `Page.navigate` is refused for any other scheme (`file:`, `chrome:`, `devtools:`, `view-source:`, `javascript:`).

## 4. The wire protocol (v0)

Server to client:
- Binary: byte 0 = kind. `1` = a JPEG frame (bytes 1.. are the image). Other kinds are ignored by a client.
- Text, JSON with a `t` field: `nav` (`url`, `title`, `canGoBack`, `canGoForward`, `loading`), `size` (`w`, `h`: the viewport the next frames have), `error` (`message`, plain words), `closed`.

Client to server (text JSON):
- `{"t":"mouse","type":"down|up|move","x","y","button":"left|middle|right|none","buttons","clicks","mods"}` (`x`, `y` in the pane's CSS pixels, which are the page's, since the viewport equals the pane and dpr is 1).
- `{"t":"wheel","x","y","dx","dy","mods"}`
- `{"t":"key","type":"down|up","key","code","text","vk","mods"}`
- `{"t":"paste","text"}` becomes `Input.insertText`.
- `{"t":"resize","w","h"}`
- `{"t":"nav","go":"url|back|forward|reload|stop","url"}`

Limits: a text message at most 64 KiB, `paste` at most 1 MiB, `w` and `h` between 100 and 4096, anything malformed is dropped with an `error`, never a panic.

## 5. Frames and flow control

`Page.startScreencast` with `format: jpeg`, `quality: 70`, `everyNthFrame: 1`, `maxWidth`/`maxHeight` = the viewport, at **device pixel ratio 1** (dpr 2 costs 2.6x the bytes; a dense screen gets a softer picture until a later setting). srv acknowledges a frame (`Page.screencastFrameAck`) **only after the frame was handed to the client's socket**, so a slow link slows the page's own capture (the ack-delay throttle of report finding 3) instead of building a queue. A hidden pane could be driven down by delaying the ack: not in the demo.

## 6. Resize (the open case of report section 8)

The spike's resize cells failed with no first-frame time; the likely cause is that no new frame arrives when the page does not change. srv therefore, on `resize`: `Emulation.setDeviceMetricsOverride` (`width`, `height`, `deviceScaleFactor: 1`, `mobile: false`), then `Page.stopScreencast` and `Page.startScreencast` again, which delivers a frame of the new size even for a still page, then tells the client with `size`. The client draws a frame at its own size and never stretches one. The probe (section 9) measures resize to first new-size frame at three sizes and records it in the report's section 8; the figure in the report is the answer, not this paragraph.

## 7. The pane in the page

- `BrowserViewComponent` chooses: `nativeBrowserPane` gives the native pane as today; else `streamedBrowserPane` gives the streamed one; else the notice. Files are under `frontend/app/view/browser/streamed/`: the socket client and reconnect (`streamed-client.ts`), input mapping (`input-map.ts`), and the pane (`StreamedBrowserPane.tsx`) with its own small bar. The native pane's bar is not reused: it is built around host calls the web host does not have; unifying them is later work.
- A frame is decoded with `createImageBitmap` and drawn on a `<canvas>` at its natural size. The pane is a focusable element; keyboard and paste are captured while it has focus and **only then**.
- Resize: a `ResizeObserver`, sent at most every 100 ms, and once more when it settles.
- The pane's URL is the block's `url` meta, as for the native pane, so a reloaded page returns to it.
- Lifecycle (Camper's pointers, to check on the PR): moving a pane between tabs or windows or tearing it off remounts it, so the socket is opened on mount and closed on cleanup, never kept in a module-level cache by block id. A hidden pane (a background tab, magnified away) may stay mounted, so the stream stops while the pane is not visible and resumes when it is. The sleep state is the one the native pane reads (`use-pane-sleep.ts`, `hibernation.ts`), not one of its own; whether a sleeping streamed pane drops its Chromium target or only its stream is decided in slice 2 (the demo only stops the stream).
- Plain-words states: connecting, "the page could not be reached", "the server's browser is busy (4 panes are open)", "the server was started without the streamed browser".
- All user-facing text goes through the i18n ratchet's rules (#7181).

## 8. Security notes for this slice

- No secret in a URL. The browser never holds the key; the session pair is the one S1 gives it.
- The streamed Chromium has no extensions, no sign-ins of ours, and a profile that is deleted with the data directory. Cookies the person creates in it stay in that profile.
- A page can open dialogs, new windows and downloads; in the demo srv denies them (`Browser.setDownloadBehavior: deny`, new targets closed as they appear, JavaScript dialogs dismissed) and says so in the pane. Handling them properly is a later PR.
- A page's file chooser is intercepted and never answered: a click on a file input offers nothing on the server's disks and reads nothing (the probe checks it).
- `--no-sandbox` appears only with the explicit variable and is logged by srv at start with a warning line.
- The pages for the demo are ones we choose (a local test page set and a short list of public pages). Nothing that holds a real account sign-in.

## 9. Proof, in order

1. **Rust unit tests:** CDP framing over a pipe, the message parser and limits, the URL gate, the opt-in gate, pane count limit, ack-after-send ordering (with a fake CDP peer).
2. **A probe, `scripts/streamed-browser-probe.mjs`:** signs in the way a script can (the key file and the loopback proof), opens a pane over the WebSocket, and measures: first frame time, a click that changes a test page, a key, a wheel scroll, a paste into an input, and resize at 800x600, 1024x640 and 1600x900 with the time to the first frame at the new size. Run on charlie in a container (no screen, mouse or keyboard), with Parlo.
3. **Vitest:** input mapping (key, button, modifiers), the client's reconnect and size handling, the pane's states.
4. **Look check on the pane's chrome** (Windows from the demo on narko, macOS Clare, Linux Maricon), because it is a visual change.
5. **The demo:** a container image of the headless server with the web build and Chromium, a run script, and the line AgentY gives the operator (tunnel, `agentmux open`, the pane).

## 10. Slices (one PR each, each through muxreview)

1. This spec, the srv module (Chromium, CDP over a pipe, the route, the protocol), its tests and the probe. Runs without any UI.
2. The pane in the web UI, the capability, the web host's check, its tests.
3. The demo image, the run script and the short how-to; the probe's numbers into the report.

## 11. Questions for the owners

- AgentA: a new module `crates/srv/src/server/streamed_browser/` plus one `mod` line and one route in `routes.rs`: acceptable? Anything that route-class test or the Host/Origin checks need that the `/ws` pattern does not already give?
- Camper: the one branch in `browser-view.tsx` and the new folder under `view/browser/streamed/`: anything in the pane's block lifecycle (close, move between tabs, the `url` meta) that a socket-backed pane must handle?
- Parlo: a container on charlie with Chromium and no screen, Docker's default network (no published port), for the probe and the demo.

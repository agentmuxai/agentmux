# AgentMux Version History

## 0.46.1 — 2026-06-16

- fix(cef): low-memory pause page Resume button was inert (double-quoted JS string inside a double-quoted onclick attribute) — wire it via a <script> + addEventListener so OOM-paused windows can actually resume
- fix(agent-pane): replace remaining <Index> with <Key> in virt list and <For> with <Index> in DiffViewer to close replaceChild crash class (#1326)
- fix(host): eliminate ghost taskbar icon from pre-warmed pool window — apply WS_EX_TOOLWINDOW at on_window_created (reliable HWND) instead of on_after_created where BrowserHost::window_handle() can return null after page load
- fix(browser-pane): hand keyboard focus back when a pane HWND is destroyed — redocking/closing a browser pane no longer locks typing app-wide (no more "open another window to get typing back")
- feat(muxbus): delivery hierarchy P1+P2 — agent ID injection, URL fix, cloud push subscriber
- fix(startup): wait for re-injected IPC creds before bridge init; retry on stale token; stop the recovery-reload loop
- diag(browser-pane): log the create register-result (Fresh/Closing/AlreadyLive), requested window, and rect on the redock path — surfaces the previously-silent AlreadyLive race so the black-page-on-redock cause is conclusive in a repro
- feat(agent-pane): session-digest accessory data model — pure projection of the digest into the Pane Accessories row model (status incl. a new "stale" state), mirroring the fork-set derivation
- feat(agent): deliver AskUserQuestion answers for one-shot/container agents as a follow-up turn
- feat(agent-pane): render the session digest as a PaneRow accessory — single status-accented row (fresh/stale/generating/failed) with age + "+N new" stale hint, replacing the bespoke banner
- fix(agent): answered AskUserQuestion no longer re-surfaces its panel on history reload
- refactor(agent-pane): remove the per-row hover strip (timestamp + expand button). Expand/collapse now lives on each surface's own header + the row keyboard handler; section headers and activity-log lines became click-to-toggle. Per-line hover timestamps are dropped.
- fix(agent-pane): persistent shell hung on inherited stdin — npm/vite dev servers now start (null stdin)
- feat(agent): answer AskUserQuestion via the Agent SDK control protocol
- feat(muxbus): LAN tier — mDNS auth_key, agent cache, tier-3 inject forwarding
- fix(ui): scrollbars use the default arrow cursor, not the link hand
- fix(toolchain): GUI-launched AgentMux can find nvm/Homebrew node, npm & git — enrich the srv's PATH from the user's login shell (+ well-known toolchain dirs) so `npm install`/agent CLIs resolve when launched from Finder/Dock/DMG (was failing with "npm: command not found"). Additive, login-shell-sourced, no-op on Windows. P0 of SPEC_TOOLCHAIN_MANAGER.
- chore(ui): add cursor design tokens + utilities and a stylelint guard against cursor on scrollbars
- fix(activity-dock): higher-contrast pinned rows + larger stop/dismiss buttons
- feat(toolchain): Toolchain manager in the hamburger menu — a new "Toolchain" item opens a modal showing the effective PATH (and how it was derived), OS/arch, and the detected version + path + status of node, npm, git, docker and every provider CLI, with install links for anything missing. P1 of SPEC_TOOLCHAIN_MANAGER (read-only; install-in-place is P2).
- fix(widgets): hug content width in the More dropdown so short widget labels no longer leave a wide empty gap on the right (was min-width:170px, now width:max-content)
- fix(cef): add `.0` before the `HWND -> *mut c_void` cast in app.rs (unbreak the Windows build; windows 0.57 `HWND` is a non-primitive struct).
- feat(mcp): add SendMessage tool for agent-to-agent messaging
- feat(agent): shared PaneRow auxiliary-pin primitive (forks Phase 1)
- feat(muxlog): cross-instance log discovery, NDJSON rendering, filters and recipes
- feat(agent-pane): surface the classified failure cause when an agent exits non-zero (SPEC_AGENT_FAILURE_DIAGNOSTICS Phase 2 — pane path). The `SubprocessController` now captures a stderr tail, runs it through `failure::classify`, and emits an `agentfailure` event; the pane shows the real reason (auth, rate-limit, OOM, context, …) + stderr tail instead of a bare "exited with code N".
- feat(agent): fork-set derivation — the data model for the fork bar (forks Phase 2)
- feat(statusbar): click CPU to open an adaptive per-core usage panel
- fix(agent-pane): cross-channel history empty — global snapshot stored a channel-local sourceBlockId
- fix(widgets): give the More dropdown a small min-width floor (120px) so it isn't razor-tight while still hugging content
- feat(menu): rename the hamburger 'Toolchain' entry to 'Toolchain Manager'
- feat(agent): PaneRegions declarative region container (forks Phase 1, aux-pins)
- feat(reactive): controller-aware delivery — steer persistent & ACP agents mid-turn instead of dropping PTY keystrokes (Agent Control Protocol Phase 3)
- feat(agent): ForkBar UI — the bottom-of-pane fork switcher (forks Phase 2)
- feat(trust-center): wire the service-OAuth connect flow in the Accounts UI (Phase 3 frontend)
- fix(agent-pane): backfill registry session_id so a cross-channel open resumes the original conversation
- feat(agent): useForkSet — reactive fork-set hook feeding the fork bar (forks Phase 2)
- feat(trust-center): brand-tile Accounts gallery with connected counts and OAuth/Key chooser
- fix(dev): reap a stale Vite squatting the dev port instead of erroring on relaunch
- feat(agent): send-now queue — instant panel, hold-until-next-tool-call delivery, ArrowUp recall, no 30s drop
- feat(agent-pane): per-error-class failure recovery row with real re-auth + 5s auto-retry
- fix(agent-pane): cross-channel restore uses stale highWaterMark — re-derive from global zone
- fix(agent-session): read global snapshot first so cross-channel opens never get a stale per-channel sourceBlockId
- fix(agent-session): G1 invariant — enforce sourceBlockId="" in global snapshot via debug_assert; schemaVersion 3 migration deferred to a follow-up PR
- fix(dev): reap guard checks CWD so another clone's live Vite is never killed on port collision
- fix(agent-pane): accept schemaVersion >= 2 so v3 snapshots are not treated as schema-mismatch
- fix(term): give terminal I/O priority over perf telemetry on the WebSocket egress
- chore(release): add --as <type> override to force a specific version bump (e.g. patch even when minor changesets are queued)
- feat(agent): tool blocks stay expanded until scrolled off the top (replaces 3s collapse timer)
- fix(persistent-agent): emit agent-message-accepted, persist user msgs to NDJSON, prefer global zone for history
- feat(launcher): memory-aware host relaunch — wait out system OOM instead of crash-looping
- feat(host): debounced memory-pressure detection + observability (mem_pressure)
- fix(agent-pane): clip persistent-shell panel so collapsed build log doesn't paint behind the conversation
- feat(host): proactive --disable-gpu at startup when commit is critically low
- feat(pane.open): OpenEditor collapse_tree + floating-pane support
- fix(muxbus): register persistent agents for Tier-1 delivery so inter-agent messages reach no-PTY panes (#1470)
- feat(muxbus): unified agent-discovery endpoint + DiscoverAgents tool (host/LAN/WAN, addressable flag)
- feat(host+ui): low-memory warning banner driven by mem_pressure
- feat(muxlog): mem/doctor command — commit-free + pressure + live AgentMux footprint
- fix(agent): apply model/effort/permission changes to running persistent (Claude) agents via resume-preserving restart
- feat(trust-center): add AgentMux Cloud as a virtual first-class account in the Accounts gallery (brain-alternate logo, reuses the existing muxbus PKCE sign-in)


## 0.46.0 — 2026-06-15

- feat(agent): backfill pre-existing agent conversations into the global transcript store
- docs(macos): document release-artifact verification (maps/symbols/channel stripped) + build-from-tag and non-Desktop output-dir gotchas
- fix(build): slim the Linux AppImage — strip frontend source maps (~28MB) and use max-level zstd compression
- feat(agent-pane): add `!cmd` shell execution prefix to run commands in the agent working directory
- fix(build): commit canonical CEF GN args + configure script so libcef rebuilds are slim by default; stop resolve-cef-runtime crying 'unpatched upstream' on the correct unstripped official build
- feat(editor): Save As inline path entry for scratch buffers (Ctrl+S / Ctrl+Shift+S)
- feat(editor): context menu polish — confirm dialog, Collapse Folder, F2 rename shortcut
- feat(agent): default Claude runtime to Opus + xhigh effort, add xhigh effort level — `xhigh` (between high and max) is the coding/agentic sweet spot and Claude Code's own default; the default model now resolves to the current Opus (4.8)
- feat(windows): native-CEF Inno Setup .exe installer (`task package:installer`) — per-user install with the AgentMux icon, replacing the removed Tauri-era installer.
- fix(agent-pane): normalize MSYS /c/ cwd for persistent Shell so task dev / vite servers spawn on Windows (os error 267)
- feat(editor): scratch buffers, file-tree context menu, widget default UX
- fix(agent): bump codex default model to gpt-5.5 and stop sending `--effort` to Claude Haiku (it 400s on Haiku 4.5) — first slice of the per-provider models/effort generalization (SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION)
- fix(srv): unsubscribe WPS broker on WebSocket disconnect — stops ~1000/min handle leak (closes #1125)
- feat(tabs): content-aware tab sizing — measure label text, size each tab to fit (VS Code shrink model)
- feat(build): add release-time BeginWindowDrag-patch gate for Linux libcef.so (verify-cef-patch.sh) — advisory in bundle:linux, hard gate in build-appimage-linux.sh
- feat(agent): per-provider model lists + provider-aware `/model` — the `/model` picker now shows the active provider's models (Claude opus/sonnet/haiku, Codex gpt-5.5/gpt-5.4/gpt-5.1-codex-max/gpt-5.3-codex) and Codex honors the picked model; runtime `model` is now provider-scoped (P2 + model-side of P3 from SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION)
- feat(agent-pane): persistent shell node Phase 3 — ShellStop tool + UI stop button (tree-kill, no more taskkill roulette)
- fix(widgets): repair phantom drop indicator after right-click unpin — replaces brittle ref/signal dual-state with a single DragState machine, adds primary-button guard and global OS-interrupt cleanup (closes #1432)
- fix(startup): self-heal + recovery UI for host-bridge init failure (auto-reload guard, Reload button, Ctrl+R/F5 keybinding)
- fix(agent-pane): scope subagent ⚡ panels to the owning pane so terminal-spawned subagents stop leaking into unrelated agent panes
- feat(editor): bind CodeMirror colors to the global theme and tighten the line-number gutter
- feat(agent-pane): pinned activity dock (Phase 1) — long-running shells pinned atop the pane, click to expand live log
- feat(trust-center): rename the app-wide "Identity & Memory" hub to "Trust Center" and add an Accounts tab for managing service credentials alongside Identity and Memory bundles
- feat(trust-center): secure API-key storage backend — validate keys against the live service, store them in the OS keychain (never plaintext in the DB), and expose them via the new account.key.verify RPC
- feat(trust-center): secure API-key entry UI — paste a key, Validate (one user-initiated probe with an inline egress notice) or save without validating; keys are stored in the OS keychain and shown masked + non-recoverable with a Replace action
- feat(trust-center): add OpenAI as a key-based account provider (validated against api.openai.com/v1/models), with icon and per-agent assignment support
- feat(agent): OpenEditor MCP tool + /api/v1/pane/open route so agents can open editor panes
- feat(trust-center): service OAuth 2.0 client scaffold — Authorization Code + PKCE (loopback) and Device Flow per RFC 8252/7636/8628/9700, with account.oauth.* RPCs; gated on per-provider client ids (inert until provisioned or supplied as BYO)
- fix(tabs): increase tab padding to match VS Code feel; resize live while typing during rename
- feat(muxbus): delivery hierarchy P1+P2 — agent ID injection, URL fix, cloud push subscriber
- fix(agent): tool live-tail skips system chunks — shows last stdout/stderr or elapsed timer while waiting
- fix(release): use only the changeset title line in VERSION_HISTORY, not every body line
- feat(agent): render AskUserQuestion as an interactive panel and deliver the answer to the agent
- fix(markdown): make the table-of-contents reactive to streaming text


## 0.45.0 — 2026-06-14

- fix(container): thread global_output_zone through the container-exec output path (publish_line) so main compiles — semantic merge conflict between #1399 and #1357 (#1401)
- fix(linux): floater drag uses JS-driven positioning (mirrors macOS) so redock hover + drop work
- feat(agent-pane): responsive aux info tiers + color system
- fix(css): pointer cursor on all scrollbar thumbs — global WebKit, xterm, OverlayScrollbars, Monaco
- fix(scripts): import-agents rehydrates Claude sessions into the isolated CLAUDE_CONFIG_DIR home so resume actually replays the conversation (history store P0)
- fix(history): scan AgentMux-isolated Claude homes so the history browse surfaces agent conversations, not just global ~/.claude (history P1a)
- feat(history): clear/delete past sessions — history.Delete + history.Clear RPCs remove native transcripts (history P1b)
- diag(agent-pane): log parent container + detached child + ancestry the instant replaceChild would throw, to finally name the reconcileArrays crash component
- fix(agent-pane): disable per-block OverlayScrollbars on streaming markdown — the actual cause of the replaceChild crash (#1326), found via the new diagnostic
- fix(term): reclaim dead space to the right of terminal text at all zoom levels
- feat(statusbar): GPU status indicator — enabled/disabled + driver info
- feat(agent-pane): persistent shell node — ShellNode type, reducer, PersistentShellBlock component and styles (Phase 1 frontend skeleton)
- fix(agent-pane): idle-send messages no longer flash in the queued zone
- feat(agent): fork prompt for parallel sessions — detect active panes and offer named fork instead of silent collision
- fix(layout): resize border no longer triggers pane tear-off
- **Root cause:** The resize handle (6 px, centered at the pane boundary) overlaps
- 1.5 px with the top of the adjacent pane's header. In that overlap zone the
- resize handle wins via `z-index: 3` vs the header's `z-index: auto`, but under
- WebView2 timing edge cases (layout recomputing during a reactive update,
- subpixel rounding at the boundary) the header occasionally received the
- `mousedown` instead — starting a pragmatic-dnd drag that set
- `_currentDragPayload`, which the `CrossWindowDragMonitor` then interpreted as a
- tear-off request.
- **Fixes applied (both in `TileLayout.win32.tsx`):**
- 1. `ResizeHandle.onPointerDown` now calls `event.preventDefault()` before
-    `setPointerCapture`. `preventDefault` on `pointerdown` suppresses the
-    subsequent `mousedown` event; since HTML5 drag-and-drop requires `mousedown`
-    to start, no drag can initiate from a press on the resize handle even if an
-    underlying element would otherwise react.
- 2. `DisplayNode.canDrag` now rejects drags whose initial pointer position
-    (`input.clientX/Y`) falls within the ±halfSize zone of any resize handle's
-    `centerPx` (converted to display-container-local coordinates). This is
-    defense-in-depth for the race: if the pointer barely misses the handle
-    element and lands on a header pixel near the border, the drag is cancelled
-    before `onDragStart` ever sets `_currentDragPayload`.
- Tear-off remains fully functional when dragging from inside the pane header
- away from the border zone.
- fix(gpu): bundle SwiftShader software-GL fallback on Windows
- fix(package): copy SwiftShader DLLs into portable runtime (follow-up to #1344)
- fix(bashwrap): collapse lone CR sequences so spinner animations don't render as one line per frame
- feat(container): add container_image, container_volumes, container_name to AgentDefinition schema and RPC types (Phase 0)
- fix(agents): surface the real cause of a failed agent run (rate-limit/auth/OOM/crash) instead of an opaque exit code
- fix(gpu): embed supportedOS manifest so Windows reports the true OS version — fixes GPU process crash
- feat(container): add Dockerfile and CI workflow for agent-claude image
- feat(agent-pane): context window fill bar in composer strip
- feat(agent-pane): persistent shell node Phase 2 — backend + MCP tool
- feat(container): Phase 2 — wire agentinput/agent.send to docker exec for container agents
- fix(agent): remove bookmark feature; right-click now shows standard tile context menu
- **What changed:**
- The bookmark feature has been removed from the agent pane:
- - **Right-click on agent feed body** now shows the standard split-right / split-left /
-   float / close tile context menu — matching terminal and other pane types.
-   Previously `DocumentRow.handleContextMenu` intercepted every right-click on the
-   feed body and replaced the tile menu with a bookmark-only menu, making the standard
-   tile actions unreachable from the agent pane body.
- - **Tool expansion overlay** no longer shows a Bookmark button in the action bar.
- - **Node hover strip** no longer shows a bookmark icon on row hover.
- - **Ctrl+B** no longer opens a bookmarks panel in the agent view.
- - **'b' key** on a focused row no longer triggers a bookmark action.
- **Files deleted:**
- - `frontend/app/view/agent/hooks/useBookmarks.ts`
- - `frontend/app/view/agent/components/BookmarksPanel.tsx`
- - `frontend/app/view/agent/styles/_bookmarks.scss`
- **Files modified:** `agent-view.tsx`, `agent-view.scss`, `DocumentRow.tsx`,
- `NodeHoverStrip.tsx`, `ToolOverlayActions.tsx`, `ToolBlock.tsx`,
- `ToolBlockOverlay.tsx`, `AgentDocumentView.tsx`, `AgentDocumentVirtualList.tsx`,
- `useAgentKeyboard.ts`, `types.ts`, `gotypes.d.ts`.
- fix(agent): remove keyboard hint line; soften input focus border
- Removes the "Enter to send • Shift+Enter for newline • Esc to clear / stop"
- hint line from the agent pane composer footer — it consumes vertical space
- without adding value for returning users.
- Changes the textarea focus border from full `--accent-color` to
- `color-mix(in srgb, var(--accent-color) 40%, transparent)` — a lighter
- variation of the pane-selected border color that stays visually connected
- to the theme without competing with the pane focus ring.
- fix(term): make scrollbar thumb always visible and clickable
- fix(term): fix xterm paste truncation — chunked send, larger input buffer, BPM on by default
- fix(macos): window drag and right-click context menus now coexist on the title bar
- The macOS title bar used `-webkit-app-region: drag` so the OS could move the
- window, but Chromium swallows every event (including `contextmenu`) on those
- regions — so right-click context menus never fired on empty title-bar space,
- and the only workaround broke dragging. Switch macOS to the JS-driven drag
- model already used on Linux: the header stays HTCLIENT (right-click works
- everywhere) and a left-button-only drag is handed to the host, which runs a
- manual move loop — pumping the drag events and repositioning the window until
- the button is released — with no patched libcef.
- Dragging the window and right-clicking the title bar now both work on the same
- surface.
- fix(agent): inject Bash(agentmux-bashwrap *) into permissions.allow so bashwrap exec is not blocked
- fix(agent): write-state schema v2 — replace nodes[] snapshot with a lightweight overlay + NDJSON restore to eliminate the renderer OOM crash. v2 restore is scoped to same-block reopen (the OOM-critical path); cross-block "structural continuation" falls back to NDJSON replay until a unified per-agent log lands (follow-up).
- feat(blockfile): output.idx byte-offset index for O(1) line seek
- Adds a lazily-built, self-validating byte-offset index (`output.idx`) so
- `blockfile:read_range` can seek directly to a requested line range instead of
- loading the whole `output` file and slicing by line number.
- The index is a pure cache of `output` with no incremental mutation: an 8-byte
- header records the output size it was built for, and the read path rebuilds it
- (one streaming scan) only when the output size changes. Because it is always
- derived from the current output in one shot, it cannot desync, mishandle
- chunk-split lines, or miscount blank lines. It indexes non-blank lines to match
- the reader's addressing, is gated to non-circular files, and falls back to the
- previous full-scan path on any error.
- fix(scroll): stickToBottom can now disengage on short conversations
- fix(bashwrap): keep CONIN writer alive until after child.wait() — prevents CTRL_C_EVENT exit 130 on Windows
- fix(term): restore URL hover in terminal pane
- fix(term): file path links now open in Explorer/Finder on click
- fix(term): clicking a directory path opens it; file paths reveal in parent
- fix(block): focused ring falls back to accent when agent color is unusable; 'Terminal' is not an agent id
- fix(bashwrap): wrap commands in a /dev/null brace-group redirect instead of exec </dev/null — the exec form closed the child's ConPTY console input and ConPTY killed every streamed bash command with exit 130 before it ran; the group redirect gives stdin-readers EOF without firing ctrl-c (#1368)
- fix(term): xterm-6 terminal scrollbar is clickable and shows the default cursor — lift the overlay scrollbar above the link-layer canvas (z-index) and force cursor:default; retire the dead xterm-5 fit/reservation + native-scrollbar code and add a CDP hit-test smoke guard (#1369, #1370)
- fix(term): remove the 5px term-connectelem margin that framed every terminal pane with block-background gaps on all four sides — the terminal now fills the block body edge-to-edge
- fix(security): strip ipc_token from renderer URL and denylist secret keys in get_env
- fix(term): terminal scrollbar sits flush against the pane's right edge — grow .xterm to fill the flex connect width so the overlay scrollbar (anchored right:0) no longer floats a zoom-varying sub-cell remainder away from the edge
- chore(srv): remove dead Go-port modules, unused subtle dep, and Tauri event constants
- chore(cruft): remove dead chat view, orphaned assets, Go-era configs
- fix(tear-off): keep the pulsing-brain splash covering the whole window bootstrap until content has settled, then cross-fade — instead of removing it mid-mount, which exposed the bare-chrome/empty/piecemeal-mount flashes (most visible on tear-off)
- chore: adopt Node 24 / npm 11 toolchain; drop unused color dep
- fix(scripts): import-agents.sh skips bad/old source DBs instead of aborting; strip CRLF from slug
- fix(dev-badge): show DEV only under task dev, not on portable/release builds — self-identify build type by exe path, not the leaked AGENTMUX_RUNTIME_MODE env
- fix(build): agents always run the app's own bashwrap, not a stale system-PATH copy — dev build now bundles agentmux-bashwrap into the runtime tools/bin, and the sidecar PREPENDS the bundled (version-locked) tools dir to the agent PATH instead of appending it (was the exit-130 root cause: a stale Downloads-portable bashwrap on the system PATH shadowed the fixed bundled one)
- feat(registry): add session_id to named-agent record (schema v2, lazy-bump)
- feat(registry): global agent-definition store (cross-channel P0.2a)
- feat(registry): cross-channel agent definitions write-mirror + read-first (P0.2b+c)
- feat(registry): backfill existing agents into global store (cross-channel P0.2d)
- refactor(registry): decouple instance working-dir base from registry root (cross-channel P0.3a)
- chore(diag): log which agentmux-bashwrap an agent will run at spawn — info when the bundled (version-locked) binary is used, WARN when it's missing and the agent will fall through to a possibly-stale system-PATH copy. Cross-check with agentmux-bashwrap --version (already supported). Cheap guardrail for the stale-binary trap (RETRO_BASHWRAP_STALE_BUNDLE_2026_06_13)
- feat(registry): re-root instance registry to global shared dir + scan all channels (cross-channel P0.3b)
- feat(registry): per-record source agents base for cross-channel workdir reconstruction (P0.4)
- fix(registry): cross-channel agent backfill now captures ALL existing agents — the one-shot definition migration was skipping whole DBs on a missing column (older schemas lack container_*), never scanned dev/ branches, and wrote an unconditional one-shot marker after that incomplete pass. Now: schema-resilient column handling (PRAGMA-introspect, default missing), scans channels AND dev, and a versioned marker that re-runs once so existing users recover their agents (Qooma, etc.) cross-channel+version. See ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13
- docs(architecture): add canonical code-anchored agent-data + cross-channel overview; mark 5 overtaken specs SUPERSEDED in place (banner → canonical doc) instead of deleting, since live code comments + kept specs cite them as design rationale
- fix(registry): anchor instance migration on the global agents root so "My Agents" repopulates cross-channel (instances; complements #1391's definitions fix)
- fix(linux): capability-probed ANGLE backend precedence (hardware Vulkan → hardware GL → SwiftShader) — fixes burst-paint terminals and enables hardware WebGL on VMware/SVGA3D (and any no-Vulkan-but-has-GL) guests, with no vendor gate
- fix(launcher): isolate each local build as its own AgentMux instance — bake a per-build BUILD_ID into the data-dir channel (data dir + cef-cache + pipe now all per-build) and make a nested portable ignore the leaked ambient AGENTMUX_CHANNEL. Completes #1315 (which fixed only the pipe; cef-cache stayed per-branch). Safe now that agents + auth are global (#1387-#1393).
- fix(agents): surface cross-channel agents in "My Agents". Fix the live registry mirror to anchor on the GLOBAL workspace root (the live-write twin of #1393 — newly-created agents were silently dropped as "not representable"), and source the My-Agents list from the global registry (deduped by definition+name, enriched with local running state, local-only agents appended) so agents created in any build/channel/version appear.
- feat(agent): globalize agent transcript so cross-channel agents load their conversation history


## 0.44.1 — 2026-06-10

- fix(cef): low-memory pause page Resume button was inert (double-quoted JS string inside a double-quoted onclick attribute) — wire it via a <script> + addEventListener so OOM-paused windows can actually resume
- fix(agent-pane): replace remaining <Index> with <Key> in virt list and <For> with <Index> in DiffViewer to close replaceChild crash class (#1326)


## 0.44.0 — 2026-06-10

- perf(pane-focus): skip updateTree for FocusNode + drop diag console.logs
- Two small, all-cross-platform wins on the click → focused-border-paint
- chain (issue #1136, full analysis in
- `docs/analyses/ANALYSIS_PANE_FOCUS_PAINT_LATENCY_2026-05-28.md`):
- - **Skip `updateTree()` for `FocusNode` actions**
-   (`frontend/layout/lib/layoutModel.ts`). The reducer previously ran
-   a full rebalance + per-leaf transform recompute after every action.
-   `FocusNode` only mutates `treeState.focusedNodeId` — topology, sizes,
-   and per-leaf transforms are unchanged. The reactive `isFocused` memos
-   still get notified via the `localTreeStateAtom._set` immediately after.
-   Savings scale with #panes-per-tab; the dominant synchronous cost on
-   the click path is gone.
- - **Drop the two diagnostic `console.log`s in `handleChildFocus`**
-   (`frontend/app/block/block.tsx`). `getElemAsStr(event.target)` walked
-   the DOM on every focusin event; the unused `getElemAsStr` import is
-   removed too.
- No platform `cfg` / `*.linux.tsx` gates; benefits every platform
- uniformly.
- fix(term): predictive-echo — short stall cooldown so a slow rAF doesn't lock predictions out for 1.2 s
- The user-visible symptom on Linux: type a sustained burst, see ~10
- chars, then ~1 s of nothing, then a huge burst of all the accumulated
- chars. Holds even with `--ozone-platform=x11` (PR #1241) because rAF
- *occasionally* still stalls past 600 ms on broken Mutter/Chromium GPU
- handoff.
- The state machine in `predictive-echo.ts` had two rollback paths
- sharing one cooldown:
- - **`reconcile()` rollback** (line ~182): PTY echoed bytes that didn't
-   match the prediction — a real divergence. Penalising with the full
-   `cooldownMs` (1200 ms default) makes sense; it rides out a mode
-   change without thrash.
- - **`sweep()` rollback** (line ~197): no echo arrived within
-   `predictTimeoutMs` (600 ms default) — *not* divergence, just a slow
-   echo. Penalising this with the same 1200 ms was wrong.
- The Linux symptom is exactly the second case looping:
- 1. User holds key, ~12 chars get predicted+painted in the first 600 ms
-    (~20 Hz key-repeat × 50 ms/key = 600 ms).
- 2. The next rAF stalls past 600 ms (we measure occasional rAF gaps to
-    ~1 s on Linux even after #1241).
- 3. `sweep()` fires → `rollback()` erases the painted chars +
-    `enterCooldown()` sets a **1200 ms** dead zone.
- 4. For the next 1.2 s, every keystroke goes through `observe()`
-    instead of `paint()` — nothing visible — while chars accumulate.
- 5. Cooldown ends → re-arms → backlog pours out at once.
- This PR splits the cooldown into two:
- - `cooldownMs` (divergence) — unchanged default 1200 ms.
- - `stallCooldownMs` (sweep timeout) — new default **100 ms**.
- Sweep now calls `enterStallCooldown(now)`. The next rAF cycle resumes
- painting almost immediately. The 100 ms default is hardcoded; a
- per-platform `stallCooldownMs` constructor option exists for tests.
- Tests: convergence + the 17 existing predict-echo tests still pass;
- added one new test specifically covering the stall path (sweep
- rollback should re-paint within ~150 ms, not be stuck for ~1.2 s).
- Spec: docs/specs/SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md
- (§7.3 — cooldown split into divergence vs stall).
- fix(agent-pane): fix virtualized layout under zoom != 1 (Phase 4 - single zoom-normalize at the measure boundary)
- feat(notify): sound notifications subsystem with turn-complete sound
- feat(notify): per-tool-call subliminal tone voice
- fix session-digest shorten summary to 10 words
- fix(virt): accumulate tool chunks in RAF buffer to prevent replaceChild crash
- feat(topbar): 3-tier progressive collapse — drop labels before overflow
- fix(virt): advance streaming-buffer frontier to cap <Index> at 50 — prevent replaceChild crash across turns
- fix(virt): defer SessionEnd, batch HistoryLoaded, Show-guard streaming buffer
- fix(scripts): import-agents.sh — scan version dirs for channel instead of deriving from git branch
- docs(linux): catch up BUILD.md, README, cef-build guide, and new linux.md operator guide
- feat(app-api): agent.define HTTP+WebSocket endpoint to create/upsert agent definitions with system_prompt, env, and model support
- docs: prominent early-alpha warning at top of README and in MSIX manifest
- Adds a callout to the top of README.md and updates the AppxManifest
- Description so the Microsoft Store listing carries the same disclaimer.
- Canonical wording lives in
- docs/specs/SPEC_EARLY_ALPHA_WARNING_2026_06_05.md.
- fix(floating-pane): restore JS-driven drag on macOS (BeginWindowDrag not available in dev)
- fix(floating-pane): dwell + velocity gate prevents accidental redock on fast transit
- feat(providers): register muxcode as a first-party provider
- fix(topbar): implement tier-3 overflow — clipped pinned icons now route to …more dropdown
- fix(my-agents): repair missing definitions in db_agents; add Created/Last Launch/Last Active timestamps to My Agents cards
- fix(topbar): reliable moreBtnW via always-mounted probe; remove dead tooIconOnly signal
- fix(topbar): restore labeledW===0 guard to prevent transient false-positive on first measure
- fix(topbar): pin/unpin context menu for clipped widgets; observe more probe; remove dead import
- fix(linux/macos): floater & secondary windows honor window:transparent (were opaque-black by construction)
- fix(launcher): rename dev-portable channel to local; use build label in pipe hash so successive task package runs start independent windows
- feat(agent-pane): colorize tool names, bash commands, streaming chunks, section headings, thinking blocks
- fix(agent-pane): Send Now panel always visible during stream stall + replaceChild crash on consecutive cap-advances
- fix(agent-pane): virtual head Key→Index — eliminates replaceChild crash on rapid window shifts
- fix(agent-pane): hold expanded tool on hover; expand Glob results by default
- feat(scripts): import-agents wires real Claude sessions so imported agents resume their conversation
- fix(agent-pane): replace For with Index in ChunkList to close the last replaceChild crash site
- fix(build): bake data channel via rustc-env so a channel change forces a rebake instead of serving a stale incremental cache


## 0.43.1 — 2026-06-06

- feat(window-drag): smooth floater header drag via host-side Win32 native move loop (#1280)

## 0.43.0 — 2026-06-05

- feat(macos): package:macos — signed .app/.dmg with CEF Helper app
- feat(macos): patched CEF 148 framework + per-type helpers fix the renderer on macOS 26
- fix(macos): lean notarized DMG (ULMO/LZMA + strip + locale trim) — 167MB
- feat(macos): launcher-owned instant splash + launcher as packaged entry point
- fix(cef): guard CreateWindowTask against null client (CrBrowserMain SIGABRT on multi-window tear-off)
- fix(macos): stop SIGABRT on window close (try_close_browser vs window.close); bundle schema; drop dead per-type helpers
- fix(macos): gate host.set_focus calls to Windows/Linux — stops SIGABRT on pane drag
- perf(linux): default to XWayland (X11 ozone) — 5–8× fewer frame stalls
- Linux CEF 146 (Chromium 146) on Mutter (GNOME) has broken native-Wayland
- GPU buffer negotiation — Chromium logs
- `WaylandZwpLinuxDmabuf::OnTrancheFlags Not implemented` at startup and
- then responds `LayerTreeHostImpl::DidNotProduceFrame` to ~89 % of
- Mutter's `BeginFrame` requests. The renderer's `requestAnimationFrame`
- callbacks (including predictive local echo's render path, #1223) are
- gated on those frames, so typing visibly hangs and pumps out on key
- release.
- Setting `--ozone-platform=x11` routes the renderer through XWayland's
- X11 present path — the wire-format Linux Chromium has shipped
- reliably for years. Measured locally on the same host
- (`scripts/capture-trace-ipv6.cjs` + CDP `Profiler.start`, 10 panes,
- sustained held key):
- |                 | Wayland (native) | XWayland (this PR) |
- | --------------- | ---------------- | ------------------ |
- | rAF firing rate | 2.5 Hz           | **6.4 Hz**         |
- | rAF gap p50     | 138 ms           | 136 ms             |
- | rAF gap p95     | 1182 ms          | **224 ms**         |
- | rAF gap max     | 8280 ms          | **1024 ms**        |
- p50 is unchanged (the 136 ms median is the residual per-frame Blink/CC
- compositor cost — separate work). p95 and worst-case drop **5×** and
- **8×**: no more "hold a key, nothing happens, release, dump." VSCode
- on the same machine (Electron 39 / Chromium 142) sits on the XWayland
- path by default and runs smoothly here — this PR brings the AgentMux
- runtime onto the same well-trodden path until native Wayland is fixed
- upstream.
- `AGENTMUX_OZONE_PLATFORM=wayland` opt-out remains for regression
- testing the native-Wayland path (which will be revisited once the CEF
- 148 binary distribution lands for Linux — the source bump is already
- in main, #1221, but the patched libcef.so needs a rebuild).
- Not a complete fix for full VSCode parity — the residual 136 ms median
- is per-frame compositor work that still needs CSS layer-tree audit
- follow-ups — but this is the largest single Linux user-visible win
- since predictive echo (#1223) and removes the worst pathological
- stalls.
- feat(providers): add Qwen Code provider
- feat(macos): notarized CEF 148 DMG — patched renderer, launcher splash, DCHECK-off build
- fix(codex): functional codex provider (launch args, gpt-5.4 model, turn-boundary session_end)
- fix(agent-pane): remove open/close pane animations, thicken high-contrast scrollbars, drop tool ok label
- fix(auth): auto-focus OAuth code box + strip OSC-8 escapes so Claude login code lands
- fix(gemini): stop Claude model leak + map turn-end to session_end
- fix(macos): move hamburger menu to far-right of title bar; mirror right-anchored menus
- fix(build): guard bundled CEF version against linked cef crate; surface a user-facing dialog on CEF init failure instead of a silent splash
- fix(agent): status-bar token stats — record claude usage (cache-inclusive)
- fix(agent-pane): HMR-safe useProcessCount RPC guard
- feat(launcher): log instance_claim isolation telemetry and key the splash window class per instance; document isolation invariants (I1-I6)
- fix(agent-pane): restore scroll-to-bottom when a pane returns from hidden
- docs(arch): replace ASCII architecture diagram with SVG (clean alignment + cardinality + brand colors); sweep stale CEF 146 -> 148 across README, host metadata, .cargo/config.toml example, CEF_ARCHITECTURE.md
- fix(macos): survive external accessibility queries (Magnet/Synergy) without crashing
- fix(window): collapse widget labels then shrink tabs when the title bar is crowded
- feat(macos): native menu bar (File/Edit/View/Window/Help) + name dev instance 'AgentMux DEV'
- docs(arch): SVG readability + accuracy — light surface for cross-theme rendering, 2-line legend (no overflow), launcher cardinality bumped from per-channel to per-(channel, version) per the I1 isolation invariant
- fix(auth): run Claude login under a PTY so the CLI survives to receive the OAuth code
- revert(arch): restore the original architecture SVG from PR #1258 (the one that renders dark on GitHub dark theme) — undoes the readability fixes from #1264 that made it permanently light
- fix(macos): window close now exits the instance instead of hanging hidden
- feat(tab): VSCode-style flush tabs + folder content surface
- refactor(agent-pane): retire the TanStack virtualizer; render the agent pane from the layout-reducer slice (Phase 3)
- fix(auth): clearer 2-step Claude login box with paste-to-submit
- fix(auth): single-flight login CLI lifecycle — supersede-kill + timeout reaper
- fix(agent): stop and drop subagent file watcher on agent unregister
- fix(linux): patch cef-dll-sys to AgentU-asaf/cef-rs fork to restore --features patched-libcef on CEF 148
- fix(agent): own tool_chunk sub at body scope; drop dead detached login spawn
- fix(linux): bundle CEF 148 Vulkan SwiftShader + headless resources so renderer process can start
- fix(macos): native close button on the DevTools window
- fix(dev): frontend-load error Retry navigates to the real URL + auto-retry
- refactor(drone): store-backed draft graph for fine-grained canvas reactivity
- feat(drone): pan/zoom canvas, zoom-aware node drag, zoom/fit controls
- feat(drone): top emoji node-type bar + drag-from-bar onto full-bleed canvas
- feat(linux): launcher in the AppImage launch path + PR_SET_PDEATHSIG reap parity (A0)
- fix(auth): macOS Claude login completes — detect Keychain-stored creds
- fix(drone): inspector title tracks selected node (was stale)
- feat(linux): Unix-domain-socket IPC + reducer + saga coordinator on Linux (A1)
- feat(drone): port-based drag-to-connect wiring with typed/acyclic validation
- fix(auth): shared provider auth dir; restore the validate-the-dir-you-run-in invariant
- fix(a11y): keep loading spinners spinning under reduced-motion (macOS)
- fix(virt): batch StreamFlush dispatches to prevent replaceChild crash
- feat(drone): inline in-node parameter editing — remove right-side inspector
- feat(error-pane): copy-on-highlight with cursor tooltip
- fix(tabbar): move fill inside scroll so empty tab-bar space is draggable
- fix(error-pane): copy-on-highlight with tooltip and full-height stack
- docs: prominent early-alpha warning at top of README and in MSIX manifest
- Adds a callout to the top of README.md and updates the AppxManifest
- Description so the Microsoft Store listing carries the same disclaimer.
- Canonical wording lives in
- docs/specs/SPEC_EARLY_ALPHA_WARNING_2026_06_05.md.


## 0.42.0 — 2026-06-02

- feat(macos): package:macos — signed .app/.dmg with CEF Helper app
- feat(macos): patched CEF 148 framework + per-type helpers fix the renderer on macOS 26
- fix(macos): lean notarized DMG (ULMO/LZMA + strip + locale trim) — 167MB
- feat(macos): launcher-owned instant splash + launcher as packaged entry point
- fix(cef): guard CreateWindowTask against null client (CrBrowserMain SIGABRT on multi-window tear-off)
- fix(macos): stop SIGABRT on window close (try_close_browser vs window.close); bundle schema; drop dead per-type helpers
- perf(package): strip source maps from release portables (~28 MB saved)
- fix(host): gate renderer OOM recovery on system memory — pause instead of false give-up
- fix(host): match dev localhost origin in crash-recovery URL reuse
- fix(agent-pane): normalize zoom in virtualizer measurement — fixes history overlap
- fix(build): bundle CEF runtime from target/release — deterministic, version-matched
- fix(agent-pane): set data-index before measureElement so all rows are observed (fixes virtualization overlap)
- perf(agent-pane): skip layout of collapsed tool overlays via content-visibility (cuts zoom/scroll relayout ~10x); guard virtualizer against undefined virtual item
- feat(agent-pane): add layout state-machine slice (Phase 0 — pure reducer + store + tests; no wiring)
- fix(agent-pane): keep tool body visible during collapse animation (content-visibility allow-discrete)
- feat(agent-pane): Phase 1a — pure currentExpansion mapper + via:default for the layout slice
- feat(agent-pane): Phase 1b — populate layout slice expansion from the live document (dev shadow)


## 0.41.1 — 2026-06-01

- fix(launcher): include build version in pipe name hash — two releases no longer share single-instance domain


## 0.41.0 — 2026-06-01

- fix(window): canonical label-based window resolution (P1)
- Route minimize / maximize / drag / browser-pane-parent through the canonical
- `resolve_window_hwnd(label)` instead of `find_own_top_level_window`, which
- returns the process's first-visible top-level — the floater when one exists,
- so those actions hit the wrong window.
- Also fixes redock-onto-main silently failing (no landing ghost, no dock): a
- warm-pool window promoted on-screen to serve as main keeps its `window-pool-*`
- label in the HWND cache while `main` is left with no live HWND, so
- `resolve_window_at_cursor` handed back the stale pool label and it never matched
- the target window's frontend `main` identity. It now resolves the
- cache-independent main frame (`find_main_window`) as `main` even when the
- reverse-map label is a lingering pool label. Adds permanent `redock-resolve`
- and `browser_pane` lifecycle instrumentation (kept, per the regression history).
- docs: spec for integrating the launcher into macOS/Linux task dev (Phase 7)
- feat(linux): floating-pane tear-off — chromeless floater (Phase A, mirrors macOS #1182)
- On Linux, tearing a pane off used to produce a full workspace window
- (tab bar + widget bar). Now it produces a chromeless floating window —
- "just the pane" — matching Windows and macOS (the latter shipped this
- in #1182).
- One-file frontend change in
- `frontend/app/drag/CrossWindowDragMonitor.linux.tsx`:
- - Pane branch in `performTearOff` now calls
-   `open_floating_pane_window` (chromeless), mirroring the win32 and
-   darwin siblings. Imports `measureSourcePaneSize` from the shared
-   helper and uses it to size the floater at the source pane's
-   rendered size (not the parent window's outer size). IPC-first /
-   mutate-on-success ordering matches the win32 reference (Reagent
-   P1 on #1073).
- - Tab branch unchanged. Tab tear-off still spawns a full top-level
-   instance with its own taskbar entry.
- No backend work: #1182 already widened the
- `agentmux-cef/src/commands/floating_pane.rs` non-Windows branch
- from "not yet implemented" to a real implementation that runs
- identically on Linux and macOS. Secondary windows on Linux are
- already frameless CEF Views windows (`window_create_top_level
- frameless=true`), and the chromeless renderer
- (`<FloatingPaneWorkspace>`) is purely a function of the
- `?floatingPaneId=` URL param — both platform-agnostic.
- Phase A scope only. Owned-window lifecycle (Gtk `transient-for` +
- `skip-taskbar-hint` + `destroy-with-parent`), JS-driven header drag,
- and floater redock are Phase B+ (tracked separately).
- Spec: docs/specs/SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md
- docs(analysis): blur audit — backdrop-filter is not an input-first hotspot (input-first 0.3, #1161)
- feat(launcher): drive srv and host via agentmux-launcher on macOS/Linux task dev (Phase 1)
- fix(linux): floating-pane header drag — use compositor IPC instead of polling
- After the Phase A chromeless floater landed, the floater appeared at the
- drop point but its header could not be dragged. The existing handler in
- `floating-pane-workspace.tsx` is polling-based: on `mousedown` it reads
- `get_window_position`, then on each `mousemove` it calls
- `set_window_position`. That round-trip is correct on Windows
- (SetWindowPos in physical px) and on macOS (CEF Views `set_bounds`),
- but on Wayland the compositor forbids client-driven top-level
- repositioning, so `set_bounds` is a no-op for position. IPCs fire at
- ~20ms cadence — the window never moves.
- Fix: branch on Linux at the top of the header `mousedown` handler and
- fire a single `start_window_drag` IPC, mirroring the main window's
- `useWindowDrag.linux.ts`. That routes through the patched CEF
- `BeginWindowDrag` → `WmMoveResizeHandler::DispatchHostWindowDragMovement`
- → `xdg_toplevel.move`, handing the drag to Mutter until mouseup. No
- polling, no `set_bounds`. Same IPC the main title bar already uses — no
- new backend surface.
- Windows and macOS branches unchanged.
- fix(linux): tear-off — delete docked layout node after IPC succeeds
- Phase A left a P1 from reagent's review on PR #1188: the win32 and
- darwin tear-off paths both call `treeReducer(DeleteNode)` after a
- successful `open_floating_pane_window` so the pane doesn't render
- twice (once in the source tab, once in the floater), and the linux
- file omitted that step. `TearOffBlock` moves the block server-side
- but the source layout's local tree still references the layout node
- until SolidJS reconciles a new tree — so the docked pane stays
- visible.
- Fix: port the same `getLayoutModelForStaticTab` → `DeleteNode` block
- from `.darwin.tsx` (lines 226-236) into the linux file's success
- branch, and change the error branch to `return` instead of falling
- through (matching darwin/win32 — if the IPC failed there is nothing
- to clean up locally, and we don't want to delete the layout node when
- the user can still see the docked pane).
- Also adds the matching imports
- (`getLayoutModelForStaticTab`, `LayoutTreeActionType`,
- `LayoutTreeDeleteNodeAction`).
- docs(spec): terminal flow control (PTY backpressure) design — input-first (#1161)
- fix(linux): tear-off — use DOM screen coords instead of get_cursor_point
- reagent CHANGES_REQUESTED on PR #1188 caught a P1: the
- `agentmux-cef` host command `get_cursor_point` is a Windows-only
- `GetCursorPos` wrapper — on non-Windows builds (Linux, macOS) it
- returns `{x:0,y:0}` (drag.rs:211-212). The linux dragend handler was
- threading those zeros into `open_floating_pane_window` as the
- floater's top-left, so every Linux pane tear-off opened pinned to the
- screen's top-left corner instead of the drop point.
- The `.darwin.tsx` sibling already solved this by reading
- `DragEvent.screenX/screenY` straight out of the DOM event (top-left
- origin, CSS px = DIP — exactly what CEF Views positioning expects).
- Port that fix verbatim to the linux file:
- - `handleDragEnd` captures `dropX = e.screenX`, `dropY = e.screenY`
-   before the 50ms settle.
- - `handleCrossWindowDragEnd` signature grows `dropX, dropY` params;
-   the `get_cursor_point` invoke is gone.
- - `cursorPoint` is now `{x: dropX, y: dropY}` — same shape, correct
-   source.
- Also clears the stale P2 doc-comment at `floating-pane-workspace.tsx`
- header: clarified that on Linux the JS-driven drag fires
- `start_window_drag` (compositor-driven) rather than the
- `get/set_window_position` polling used on Windows/macOS.
- After this commit the only remaining `get_cursor_point` invoke is in
- the win32 sibling, where it is correct.
- fix(linux): tab tear-off anchor — drop DPR scale now that screenX is DIP
- reagent P2 against `edda8911` on PR #1188: the tab-anchor in
- `.linux.tsx` does `screenX - grabOffset.x * dpr`, which was correct
- when `screenX` came from `get_cursor_point` (Windows-style
- `GetCursorPos` returning physical px). The prior commit on this PR
- switched `screenX/Y` to DOM `e.screenX/Y` (CSS px = DIP) to fix the
- floater-at-screen-origin bug — but the tab-anchor multiplier still
- assumed physical px. On HiDPI Linux that double-scales the grab
- offset, placing the tab tear-off window off by the grab-offset
- amount; harmless only at dpr=1.
- Fix: drop the `* dpr` on both axes. Both `screenX/Y` and
- `grabOffset.x/y` are now DIP, and CEF Views positions in DIP on
- Linux, so plain subtraction is correct. The `* dpr` survives in the
- win32 sibling, where it's still right (`get_cursor_point` returns
- physical px there).
- Updated the inline comment to spell out the DIP arithmetic and why
- the win32 sibling diverges.
- fix(ui): canonical maximize/restore icons + tighter hamburger
- feat(dnd): file drop on terminal + agent panes (phase 1)
- fix(macos): show the AgentMux instance in the Dock (set Regular activation policy)
- test(bench): term key-repeat hiccup diagnostic (CDP frame/longtask/mark capture) — input-first (#1161)
- docs(analysis): agent app API + amux CLI for open-in-editor
- fix(cef): never request OS credential/keychain access in any runtime mode
- feat(packaging): Microsoft Store MSIX packaging for the CEF app (task package:msix)
- refactor(term): remove the Stage-1 RAF write-coalescer (double-rAF) — xterm RenderDebouncer is the only frame gate (input-first #1161)
- feat(agent-pane): new-message enter animation — swift fade+slide on streaming rows
- perf(agent-pane): incremental streaming-markdown render
- chore(infra): remove dead speculative CDK source for the never-deployed webhook stack
- fix(agent-pane): cap tool-output rendering to bound conversation DOM
- chore: delete dead db/migrations-* legacy SQL files (superseded by inline flat schema)
- fix(agent-pane): byte-cap single huge lines in tool output
- fix(agent-pane): gate send-now zone on an interruptible turn
- fix(agent-pane): hide tool-overlay header while running
- perf: drop sysinfo cascade-diagnostic instrumentation
- Commit ac70ff59 (2026-05-28) added temporary diagnostic instrumentation to
- the sysinfo broadcast hot path — two `tracing::warn!` calls in
- `Broker::publish` (one per missing-client branch, one per zero-routes
- sysinfo branch) and two per-tick `console.log("[fe] sysinfo:* handler …")`
- emits in the status-bar widgets (`SystemStats`, `BackendStatus`). All four
- were tagged "Remove once the cascade root cause lands."
- The cascade root cause has since landed. The diag is no longer needed and
- is shipping in production builds, where:
- - The two FE `console.log` calls fire every sysinfo tick (~1 Hz × 2
-   widgets) → routed through CEF's console→host bridge → synchronous
-   `tracing::info!` → disk. Locally measured at ~70k `[fe] sysinfo:*` lines
-   per day in the host log on Linux. Each emit also rides the main thread's
-   IPC bridge, adding noise to the input-first hot path.
- - The two backend `tracing::warn!` calls fire from inside the broker
-   mutex, with `event.scopes` formatted as `?` debug on every sysinfo
-   publish whose route lookup is empty.
- This commit removes all four diagnostics. The widget bodies revert to the
- shape they had pre-ac70ff59 (no diagnostic logging around the reactive
- setStats / setUptimeSecs calls). The broker's `client is None` arm
- becomes a plain early return; the zero-routes branch is deleted entirely.
- Net: 1 insertion / 29 deletions across 3 files.
- perf(block): replace `.block-mask` `backdrop-filter` with `will-change: transform`
- feat(term): predictive local echo for terminal input
- perf(cef): enable EarlyEstablishGpuChannel + EstablishGpuChannelAsync
- Adds `--enable-features=EarlyEstablishGpuChannel,EstablishGpuChannelAsync`
- to the CEF browser-process command line in
- `AgentMuxApp::on_before_command_line_processing`. Both features ship
- enabled in stable Chrome on Linux and are explicitly set by VSCode's
- Electron (confirmed via `/proc/<pid>/cmdline` on a running VSCode); CEF
- does not enable them by default.
- They (a) request the GPU process channel before the renderer's first
- paint instead of synchronously on first paint and (b) treat the channel
- establishment as non-blocking, which lets the compositor start producing
- frames against the GPU process sooner.
- ## Empirical impact (Linux Chromium-Ozone-Wayland, 10 panes, 12 s held key)
- Measured via `scripts/capture-trace-ipv6.cjs`:
- | | Before | After |
- |---|---|---|
- | `BeginMainFrame` avg | 39.1 ms | **35.7 ms** (-9 %) |
- | `BeginMainFrame` count | 12 | 12 (unchanged) |
- | Frame cadence | ~1 Hz | ~1 Hz (unchanged) |
- | `LayerTreeHostImpl::DidNotProduceFrame` | 66 | 60 |
- Small per-frame win (~3 ms / frame). The Wayland frame-production stall
- that holds the cadence at ~1 Hz under sysinfo invalidation is a separate
- problem (the renderer rejects 60+ Mutter `BeginFrame` requests as
- `DidNotProduceFrame` because xterm.js WebGL canvas writes don't surface
- as page-level dirtiness). Tracked separately; this PR is just the
- matching VSCode flag set so we're not unnecessarily off-default.


## 0.40.1 — 2026-05-30

- fix(cef): unbreak agentmux-cef compile on macOS with public cef-rs 146
- fix(macos): make task dev launchable on a fresh checkout — bundle CEF Framework + suppress keychain prompts
- fix(macos): port PR #444 traffic-light buttons — hide Win11 caption buttons on macOS, add platform-resolved window-controls
- fix(browser-pane): replay deferred create after close-drain so redocked panes always load
- fix(macos): patch NSApplication for macOS 26 Tahoe drag crash
- fix(macos): traffic-light scss stylelint comment + stale docstring on win32 move
- fix(floating-pane): maximize resizes the frontend browser, not the web-content child
- chore(perf): add input-handler sync-IPC CI guardrail (input-first I2, #1161)
- fix(dnd): suppress prohibited-cursor during cross-window tear-off on macOS+Linux
- test(perf): input-latency bench harness — multi-run baseline-delta + reporting (input-first 0.1, #1161)
- perf(airspace): skip redundant SetWindowRgn when a pane's clip is unchanged (input-first 0.3b, #1161)
- feat(floating-pane): resize floating panes by dragging edges/corners
- fix(macos): set Dock icon at runtime so task dev shows the AgentMux logo
- fix(linux): enable patched-libcef feature in build:host:linux so window drag works
- feat(window-drag): Windows title-bar drag via host-side native move loop (VS Code-smooth, zero per-move IPC)
- feat(macos): floating-pane tear-off — chromeless floater + working header drag
- docs: macOS floating-pane redock analysis + implementation plan
- docs(floating-pane): cross-platform field docs for OpenFloatingPaneArgs x/y/width/height
- feat(macos): floating-pane redock — drop a floater onto a window to merge the pane
- fix(dnd): suppress tear-off drag snapback on macOS/Linux for pane + tab
- feat(ui): fatten pane control + hamburger icons, raise pane body text default to 15px
- fix(window): gate macOS/Linux floating-pane redock resolver off Windows so the host compiles
- fix(macos): suppress AppKit drag slide-back animation on pane/tab tear-off
- fix(airspace): send overlay-clip rects in physical px so DOM overlays clip correctly over browser panes at display scale != 100%


## 0.40.0 — 2026-05-29

- feat(floating-pane): MVP re-dock — drop floater over agentmux window to redock its block
- feat(agent-pane): scrub orphan in-progress nodes on session reopen
- fix(tab-switch): fade-in opacity reveal eliminates FOUC flash + investigation spec
- fix(workspace-tabs): widen tab basis to 240px (VS Code-aligned), enable grow, floor at 100px, cap at 320px, cap tab-name input at 128 chars
- feat(agents): instance_get_by_name reads from db_agents (Phase 3b.2)
- feat(agents): instance_list no-status case reads from db_agents (Phase 3b.3a)
- feat(agents): instance_get_active_for_block resolves via block→agent reference (Phase 3b.4)
- fix(package-portable): refuse to wipe a running install
- refactor(storage): extract memory_bundles to its own module (R.3)
- docs(readme): document muxlog log helper in Quick Start
- fix(cef): resolve_frontend_base_url returns Result instead of silently emitting localhost:5173 in production
- fix(cef): bound on_render_process_terminated with a crash budget (no infinite recovery loop)
- fix(cef): auto-close crash-loop terminal page so the dead window releases its instance number
- fix(agent-pane): merge same-batch text deltas instead of dropping the update
- fix(launcher,cef): anchor asset lookup on AGENTMUX_HOME env from launcher instead of fragile current_exe()
- fix(cef): rate-limit renderer_terminated log on the crash target (100ms gap, suppressed_count rolled forward)
- refactor(storage): extract content/skills/history modules (R.4)
- refactor(storage): extract identities module (R.2)
- refactor(storage): extract Phase 3a dual_write helpers (R.5)
- refactor(storage): extract registry_mirror module (R.6)
- fix(cef): evict stale HWND from window_hwnds cache + on_before_close cleanup
- refactor(storage): extract agents module (R.1) — modularization complete
- fix(agent-pane): suppress Send-now flash by excluding Submitting from the predicate
- fix(agent-pane): collapse 3 hover events into one auto-expand panel
- build(package): ephemeral local build labels — stop committing version bumps for smoke builds
- diag(statusbar): sysinfo channel instrumentation to bisect agent-pane cascade freeze
- feat(statusbar): show local build label in the instance panel
- refactor(cef): extract window lifecycle handlers into commands/window/lifecycle.rs
- feat(agent-pane): IME composition handling + agent-keystroke perf marks + slash matcher TODO
- refactor(cef): extract window motion handlers into commands/window/motion.rs
- refactor(cef): extract window chrome handlers into commands/window/chrome.rs
- ci(perf): guardrail against layout reads in agent/term input handlers
- refactor(cef): extract window transparency/opacity handlers into commands/window/transparency.rs
- test(bench): agent composer keystroke-latency benchmark via CDP
- docs(spec): input responsiveness — terminal + agent pane structural rules + execution plan
- refactor(cef): extract window meta handlers into commands/window/meta.rs
- refactor(cef): extract window creation handlers into commands/window/creation.rs (window.rs modularization complete)
- feat(host-reducer): pane-state reducer Phase 0 — pane_window_states scaffolding
- fix(host-reducer): co-evict pane window-placement state on pane close (Phase 1)
- fix(layout): animate pane open/close/split so panes glide into place instead of snapping
- refactor(agents): partial-update API for updateagentinstance (drops fetch-and-merge)
- feat(floating-pane): reducer-backed maximize/restore for torn-off floating panes
- docs(analysis): typing-perf open-tracking inventory + consolidation
- fix(agents): db_agents.working_directory mirrors the def's configured cwd, not the instance's resolved workdir
- fix(window): cache the OUTER top-level frame (GA_ROOT) at capture, not the CEF inner WS_CHILD
- feat(storage): add db_agents.last_block_id column + dual-write maintenance (Phase 3c PR1)
- fix(cef): bind window label to promoted window, not off-screen warm-pool HWND (fixes window drag)
- fix(floating-pane): maximize to monitor work area + fixed maximize button
- fix(floating-pane): resize embedded CEF child to client area on WM_SIZE
- fix(floating-pane): redock onto main resolves via find_main_window when window_hwnds cache misses

## 0.39.3 — 2026-05-28

- (no semantic content — internal portable-build counter increment from `task package`; auto-appended by `scripts/bump-wrapper.sh` to satisfy the release-consistency invariant)


## 0.39.2 — 2026-05-28

- (no semantic content — internal portable-build counter increment from `task package`; auto-appended by `scripts/bump-wrapper.sh` to satisfy the release-consistency invariant)


## 0.39.1 — 2026-05-27

- (no semantic content — internal portable-build counter increment from `task package`; auto-appended by `scripts/bump-wrapper.sh` to satisfy the release-consistency invariant)


## 0.39.0 — 2026-05-27

- docs(menu): spec for menu positioning framework + paintable-area guard
- feat(menu): useMenuPosition framework primitive (Phase 1)
- feat(menu): route FlyoutMenu + Popover through useMenuPosition (Phase 2)
- feat(menu): migrate More dropdown + TokenBreakdownPopover to useMenuPosition (Phase 3)
- feat(menu): paintable-area guard — dev assertion + CI grep gate (Phase 4)
- fix(menu): preserve crossAxis/alignmentAxis offset semantics in Popover
- fix(menu): route the right-click context menu through computeMenuPosition
- feat(editor): file-tree explorer rooted at $HOME with header chevron + toolbar (show hidden / collapse all / refresh)
- fix(agent): show agent name in composer placeholder instead of UUID hash
- feat(editor): multi-root file tree (HOME + drives/mounts) and draggable resize handle between tree and editor
- feat(agent-pane-state): reducer state + commands for composer details panel
- feat(agent-pane): slim composer status strip replaces AgentStatusLine
- feat(editor): pane icon toggles the file-tree (replacing the separate chevron); title bar now shows the full file path
- fix(composer-strip): correct SCSS mixins path (build broke after #1069)
- feat(floating-pane): Phase 3 — pane tear-off routes to floating child window, not new instance
- feat(editor): LSP integration Phase 1 — TypeScript diagnostics via typescript-language-server, with install banner + status chip
- fix(agent-pane): wrap user input + smooth tool-block collapse + 3s hold
- feat(toggle): square thumbs + tracks (canonical Toggle + status-bar LAN discovery)
- feat(floating-pane): Phase 2 — real Block renderer via initApp + chromeless workspace
- feat(build): task package auto-appends VERSION_HISTORY entry to fix recurring reagent drift
- fix(dev): respect AGENTMUX_VITE_PORT in child windows so tab/pane tear-off works on non-5173 dev clones
- feat(floating-pane): native title-bar drag via WM_NCHITTEST + size matches source pane
- feat(editor): per-pane zoom + decouple icon-as-toggle from header dblclick
- feat(editor): slice #10 editor-pane-state — Phase 1A scaffolding
- feat(floating-pane): floater renders as a free-floating docked pane — no extra chrome, no OS controls
- feat(editor): tabs Phase 1B — view + tab strip
- feat(fe-log): runtime source-map resolver for piped error stacks
- fix(editor-pane-state): call eventSink whenever state changes, not only when events array is non-empty
- feat(editor): preview tabs + centered error panel + binary-file guard + memo reactivity fix
- feat(agents): dedupe continuation chains in My Agents picker (Phase 3b.1) + tracking spec
- perf(pane-overlay): short-circuit IPC + coalesce on rAF + drop class observation (Agent2 #1097)
- fix(ui): title-case widget bar and pane labels
- refactor(store): rename wstore → store (Wave-fork relic) + modularization spec (R.0)
- fix(agent-pane): render-gap (stream-parser id collision) + replaceChild crash (sticky-frontier partition)
- fix(floating-pane): suppress focus-ring border + route window-drag/close by label
- feat(widgets): reorder + slim default widget bar — pinned: Agent, Swarm, Drone, Warden; in More: Terminal, Editor, Browser, Sysinfo, Help
- feat(browser): slice #9 extension — Phase 1A multi-tab scaffolding
- feat(tabs): workspace tabs adopt editor-style basis + shrink + ellipsis (160/64/220 px)
- feat(empty-tab): show user@host + version on the empty-tab logo panel
- feat(agent-picker): install ribbon green instead of yellow (forward action, not warning)


## 0.38.14 — 2026-05-26

- (no semantic content — internal portable-build counter increment from `task package`; the build itself failed mid-Vite-transpile from an SCSS import error introduced by #1069, so the bump landed without a portable; #1072 fixes the import. Backfilled to satisfy the release-consistency invariant)

## 0.38.13 — 2026-05-26

- (no semantic content — internal portable-build counter increment from `task package`; backfilled to satisfy the release-consistency invariant)

## 0.38.12 — 2026-05-26

- feat(window): top-bar widget icons become theme-driven, monochrome by default (PR 1 of SPEC_WIDGET_ICON_COLORS)
- feat(layout): show WxH badge at pane corner during resize
- fix(modal): structural compact-variant redesign — Phase 1 of MODAL_COMPACT_VARIANT_ARCHITECTURE
- feat(dev): per-clone Vite port derivation so two task dev instances can run in parallel
- chore(modal): Phase 2 — replace JS class-toggle with CSS container queries


## 0.38.11 — 2026-05-26

- (no semantic content — internal portable-build counter increment from `task package`; backfilled to satisfy the release-consistency invariant)

## 0.38.10 — 2026-05-26

- (no semantic content — internal portable-build counter increment from `task package` to give each portable ZIP a unique version label; backfilled to satisfy the release-consistency invariant)

## 0.38.9 — 2026-05-26

- fix(modal): add compact-mode min-width override to launch/prereq/new-bundle/browser-auth modals


## 0.38.6 — 2026-05-26

- fix(lan): normalize mDNS hostname with .local. suffix so service registration succeeds on Windows / non-mDNS hosts
- docs(readme): refresh widget table — all-pinned default, add Warden, drop stale More tier
- feat(common): per-clone Dev isolation — RuntimeMode::Dev now carries clone_id for two-clones-same-branch parity
- fix(widgets): preserve Solid reactivity in ActionWidget so responsive collapse to icon-only works

## 0.38.5 — 2026-05-26

- chore: version bump (post-release rebuild to give the next portable a distinct version label from prior 0.38.4 portables on the build machine)

## 0.38.4 — 2026-05-26

- feat(statusbar): opacity slider % tracks the drag (reactive store)
- feat(activity): add global agent-activity aggregator (taskbar indicator step 1)
- tweak(tool-block): bump post-completion hold from 1s to 5s
- feat(srv): agent-anchored session zones (backend)
- feat(agent): default-continue from agent session zones (frontend)
- feat(focus): instrument LAST_FOCUSED_CHILD on intentional Win32 focus (window-reactivate step 1)
- feat(picker): two-tier — My Agents + templates
- feat(picker): hide templates
- feat(srv): db_agents schema + dual-write
- feat(srv): migrate read paths to db_agents (Phase 3b)
- fix(srv): drop parent_instance_id filter from instance_list_named so continuation rows appear in My Agents picker (Option E)
- fix(srv): make template_promote migration self-idempotent (data-invariant-gated, not marker-gated)
- fix(agent): pass prior session id through reattach so spawn includes --resume on the first turn
- fix(agent): failed tool panels collapse after the 5s post-completion hold like successes
- docs(spec): per-root focus tracking (window-reactivate step 1.5)
- fix(agent): high-contrast user-input color + no soft-wrap on typed messages
- fix(agent): mark startup-injection user messages via stream-parser flag
- fix(agent): UserMessageBlock collapses startup injection on hover, mirrors ToolBlock
- refactor(focus): per-root LAST_FOCUSED_BY_ROOT map (window-reactivate step 1.5)
- fix(agent): hover-expanded startup body uses absolute overlay so summary stays under cursor
- fix(agent): opaque overlay bg + extend hover-anchor to ToolBlock for upward expansion near pane bottom
- fix(test): exclude .claude/worktrees from vitest discovery to drop 82 duplicate failures
- fix(test): unstale browser-pane title and ACP openclaw OAuth assertions
- feat(focus): top-level WM_ACTIVATE handler restores focus on window re-activate (step 2)
- chore(test): load @testing-library/jest-dom/vitest types in tsconfig (29 tsc errors → 0)
- feat(common): channels — version-spanning data isolation (Increment A from SPEC_DATA_CHANNELS)
- fix(window): drop hand-link cursor on title-bar widgets + Windows window controls
- fix(term): await fonts.ready + NaN guard + post-paint refit to fix jumbled startup render
- feat(srv): safety lock — refuse to open schema written by newer binary (channels Increment B.1)
- chore(modal): drop modal-v2 references from comments — canonical modal is the only version
- feat(network): add LAN discovery toggle to HostPopover with live daemon start/stop
- feat(modal): parameterize ModalLayer over scope — pane-scope launch modal (SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25)
- diag(cef): instrument on_auth_credentials + on_load_error entry points (browser-pane auth diagnosis)
- feat(srv): pre-migration SQLite snapshots — Increment B.2 lean cut from SPEC_DATA_CHANNELS
- fix(cef): pass --disable-chrome-login-prompt so OnGetAuthCredentials fires (browser-pane HTTP auth modal)
- feat(browser-pane): wrap browser view in pane-scope ModalLayer (auth modal locks only the pane)
- feat(modal): compact variant for narrow lock regions (auto-trigger via ResizeObserver)
- fix(term): use document.fonts.load() to force-load Hack before first fit (#1030 follow-up)
- fix(term): PSReadLine cursor-desync thaw on terminal init (#1042)
- feat(warden): add widget shell with three-section (Host/LAN/Internet) scaffold
- feat(warden): host section renders live agent list from reactive registry
- feat(warden): host section shows recent jekt audit feed
- feat(warden): LAN section renders live mDNS peer list
- feat(warden): host section adds soft-deregister button per agent
- feat(theme): hard corners on buttons + modals; pills/avatars stay round


## 0.38.3 — 2026-05-23

- feat(widgets): responsive labels collapse to icons on narrow title bars; all widgets pinned by default
- fix(modal): defer singleton-modal label resolution past app boot
- fix(launch-modal): thread continueOfId through the +New bundle round-trip
- feat(identity): add SecretRef::OAuthConfigDir + per-bundle dir helpers (oauth bundles PR A)
- feat(identity): resolver provider-class + OAuth config-dir dispatch (oauth bundles PR B)
- build(release): self-verify version-file consistency after task release
- feat(oauth): bind OAuth credentials into identity bundles on Connect (oauth bundles PR C)
- feat(oauth): identity-binding status + reconnect UI (oauth bundles PR D)
- feat(oauth): seed Default identity bundle from ambient ~/.claude on startup (oauth bundles PR E)
- refactor(oauth): drop openclaw always-gate hack; bundle path handles all oauth providers (oauth bundles PR F)
- fix(agent-pane): PTY cols follow pane width + bump default to 200
- feat(agent-pane): add TurnPhase discriminated union (turn-phase PR A dual-write)
- fix(agent-pane): migrate throwing dispatch sites + capture uncaught DOM errors
- docs(recovery): recovered Maks conversation transcript from v0.38.3 cascade incident
- fix(tool-block): post-completion hold timer was cancelled by reactive self-loop
- feat(agent-pane): bounded interrupt timeout
- feat(agent-pane): view reads isWorking(state)
- feat(agent-pane): initPhase state machine
- feat(tool-block): UX polish — hover delay, collapse animation, post-completion hold, scroll isolation, a11y inert
- feat(agent-pane): bounded submit timeout
- feat(agent-pane): bounded streaming watchdog
- feat(agent-pane): Disconnected state + banner
- refactor(agent-pane): drop legacy turnActive/stopping/streaming.active
- feat(block): per-block error boundary + localized reload
- feat(agent-pane): unified per-pane registration
- feat(agent): recent sessions reattach UX
- ci: add release-consistency gate (closes retro action item 3)
- feat(agent-pane): model-level dispatchIfAlive helper


## 0.38.1 — 2026-05-22

- fix(magnify): single-instance pane render — fixes zoom + browser-pane black on restore
- fix(statusbar): show DEV badge in dev mode + add to instance panel
- feat(agent): launch modal defaults to Continue mode for known definitions
- chore(legal): add NOTICE, SECURITY.md, ACKNOWLEDGEMENTS.md, README disclaimer, and SPDX headers
- fix(security): correct shell integration component name in SECURITY.md scope
- refactor(bundles): extract IdentityManager/MemoryManager components
- feat(agent): OAuth Connect creates a named Identity bundle first
- feat(modal): app-wide singleton modal coordination layer
- feat(bundles): Identity & Memory manager modal + hamburger entry
- refactor(bundles): demote agent-settings Identity/Memory tabs to read-only


## 0.38.0 — 2026-05-22

- fix(term): remove writeInFlight guard from small-data fast path in scheduleRafWrite
- docs(bench): add terminal echo-latency benchmark and spec
- feat(voice): per-pane mic button in frame header (Phase 2)
- feat(voice): Phase 3 polish — tooltips, permission toast, settings toggle
- fix(launcher): relocate launcher-sagas.db into `<data-dir>/db/`
- Closes audit item §8.3. The launcher saga log previously lived
- directly in `<data-dir>/launcher-sagas.db` while srv put all its
- SQLite files under `<data-dir>/db/`. The new
- `data_dir::launcher_saga_log_path()` returns the canonical
- `<data-dir>/db/launcher-sagas.db` and performs a one-shot back-
- compat rename from the legacy location on first launch.
- Idempotent + safe to call repeatedly; +4 unit tests cover fresh
- install, legacy migration, both-files-present, repeated calls.
- Audit doc also corrected: §8.2 (duplicate saga tables) was a
- false alarm — retracted.
- refactor(storage): rename `db_identities` → `db_identity_bundles` and `db_memories` → `db_memory_bundles` (v11)
- Closes AUDIT_SQLITE_SYSTEMS §8.1 — the schema-naming drift where v7 created
- tables under the bare names but the rest of the codebase + UI vocabulary used
- "identity bundle" / "memory bundle." The "bundle" suffix conveys that each row
- carries multiple facets (provider bindings + display name for identities;
- instructions + context_files + mcp_servers + skills for memories).
- - New `run_forge_v11_migrations`: idempotent `ALTER TABLE … RENAME` for both
-   tables, plus DROP+CREATE on the `is_blank` indexes. SQLite ≥ 3.25
-   auto-updates the FK reference in `db_identity_bindings`, so the binding
-   cascade-delete still works through the rename (covered by a new test).
- - v7 now guards its legacy-name DDL/seed/shadow-migrate block on
-   `db_identity_bundles` not yet existing — prevents re-creating empty old
-   tables alongside the renamed ones on every subsequent startup.
- - v1 (the base `db_forge_agents` CREATE) extracted into its own
-   `run_forge_v1_migrations` so tests can stage a pre-v7 schema and exercise
-   the legacy path independently.
- - All `wstore.rs` queries + doc comments updated to the bundle names.
- - `db_identity_bindings` is intentionally NOT renamed — its name was already
-   bundle-consistent (a binding binds an identity bundle to a provider
-   account; the surrounding object IS the binding, not the bundle).
- refactor(storage): flatten objects.db schema, retire the "forge" vocabulary, add user_version tripwire
- Implements `docs/specs/SPEC_SCHEMA_FLATTENING_2026_05_19.md`. Closes
- AUDIT_SQLITE_SYSTEMS §8.5 (and absorbs the §8.1 / PR #933 v11 rename).
- **Flatten.** The 11-step incremental migration chain (`run_forge_v1` …
- `run_forge_v11`) is replaced by a single flat `run_object_schema` that defines
- the final `objects.db` schema directly. Per-version data dirs mean every new
- version was always born with a fresh DB and ran the whole chain in one shot
- anyway — the intermediate states were never reachable. ~1,400 lines of
- migration code + tests deleted.
- **De-forge.** "Forge" is dead vocabulary (replaced by Memory / Identity /
- agent-definition). Renamed Rust-side: `db_forge_agents` → `db_agent_definitions`,
- `db_forge_content` → `db_agent_content`, `db_forge_skills` → `db_agent_skills`,
- `db_forge_history` → `db_agent_history`, `db_forge_agent_identities` →
- `db_agent_identity_links`; structs `ForgeAgent`/`ForgeContent`/… →
- `AgentDefinition`/`AgentContent`/…; `forge_*` store methods → `agent_def_*` /
- `agent_content_*` / `agent_skill_*` / `agent_history_*`; files
- `forge_handlers.rs` → `agent_handlers.rs`, `forge_seed.rs` → `agent_seed.rs`.
- The RPC wire command strings are intentionally unchanged (decision A1) — the
- frontend is untouched and the wire contract is stable.
- **Dead tables dropped.** `db_workflow_definitions` / `db_workflow_runs` and the
- `db_v10_migrated_legacy_*` sentinels are no longer created; `adopt_legacy_table_names`
- drops them from any pre-flatten dev DB — their data had already been copied
- into `db_drone_*`.
- **Safety net.** `adopt_legacy_table_names` runs once per startup: it renames
- any pre-flatten table names found (the single surviving fragment of the old
- chain — it also subsumes the v11 bundle rename) so a developer's pre-flatten
- `objects.db` carries forward without data loss. SQLite ≥ 3.25 auto-updates FK
- references when a parent table is renamed.
- **user_version tripwire.** `stamp_and_check_version` stamps `PRAGMA user_version`
- on all four SQLite files (`objects.db`, `filestore.db`, `sagas.db`,
- `launcher-sagas.db`) and logs a loud warning if a file was written by a newer
- build (downgrade detection — the bug class behind PR #933's Codex P1). A
- tripwire, not a migration gate: idempotent DDL remains the schema mechanism.
- refactor(a2): retire the "forge" vocabulary from the IPC wire + frontend
- Follow-up A2 to the storage de-forge (PR #934). That PR renamed the Rust
- storage layer but deliberately left the IPC wire command strings and the
- whole frontend `forge` view untouched (decision A1). This completes the job
- — "forge" is now gone from every layer a developer reasons about.
- The IPC wire is internal (CEF frontend ↔ srv, shipped together), so the
- command strings are renamed outright with no compat shim; srv + frontend
- land atomically.
- - **Wire commands** — `listforgeagents` → `listagents`, `createforgeagent`
-   → `createagent`, `getforgecontent` → `getagentcontent`,
-   `listforgeskills` → `listagentskills`, `appendforgehistory` →
-   `appendagenthistory`, `importforgefromclaw` → `importagentfromclaw`,
-   `reseedforgeagents` → `reseedagents`, etc. (18 commands). The
-   `COMMAND_*_FORGE_*` constants rename to match.
- - **Frontend view** — `frontend/app/view/forge/` → `view/agent-def/`;
-   `forge-model.ts` → `agent-def-model.ts`, `forge-constants.ts` →
-   `agent-def-constants.ts`; `ForgeViewModel` → `AgentDefViewModel`; the 9
-   `Forge*` components → `AgentDef*` / `AgentSkill*` / `AgentContent*` /
-   `AgentHistory*`.
- - **Types** — `gotypes.d.ts` + `rpc-api.ts` updated: `ForgeAgent` →
-   `AgentDefinition`, `ForgeContent` → `AgentContent`, `ForgeSkill` →
-   `AgentSkill`, `ForgeHistory` → `AgentHistory`, and the `Command*Forge*Data`
-   types, matching the Rust struct names from #934.
- - **Settings/overlay tab** — the internal `"forge"` tab enum value →
-   `"agent"`. The `block.tsx` `view: "forge"` → `"agent"` migration shim is
-   kept (back-compat for already-persisted blocks).
- - `forge-seed.json` → `agent-seed.json`; `seed_forge_agents` →
-   `seed_agents`; `default_forge_icon` → `default_agent_icon`.
- Out of scope (follow-up): the `forge-*` CSS class names (~74) are an
- internal styling layer — renaming them risks silent visual regressions the
- compiler can't catch, so they're left for a dedicated cosmetic sweep.
- Verified: `agentmux-srv` builds clean; frontend vite build clean; 3,270
- frontend tests pass.
- feat(menu): hamburger menu tweaks — reorder, DevTools item, Documentation link
- Three tab-bar hamburger (≡) menu changes:
- - **Reorder.** "New Tab" is now the topmost item. "Command Palette" moves
-   from the top down to just below Settings.
- - **DevTools is no longer a widget.** Removed `defwidget@devtools` from
-   `widgets.json` (and the now-dead devtools special-case in
-   `handleWidgetSelect`). DevTools toggling moves into the hamburger menu as
-   a "DevTools" item between Settings and Command Palette — same
-   `toggleDevtools()` action. (The Command Palette's "Toggle DevTools"
-   command is unaffected.)
- - **"Help" → "Documentation".** The menu item is renamed and now opens
-   `https://docs.agentmux.ai` in the external browser via
-   `getApi().openExternal(...)`, instead of opening the in-app help pane.
-   The `help` widget / `view: "help"` pane are unchanged.
- New bottom-of-menu order: Documentation · Settings · DevTools · Command
- Palette · — · Exit.
- fix(agent): gear/cog opens the settings panel reliably
- Clicking the ⚙ gear in an agent pane header did nothing. The gear flips an
- overlay-tab signal, but the panel was rendered behind
- `<Show when={showOverlayTab() != null && currentAgent() != null}>`.
- `currentAgent()` resolves the pane's `agentId` block-meta against the
- `db_agent_definitions` list. That only matches for panes launched via
- `launchAgentDefinition` (the AgentPicker → launch-modal path). It is `null`
- for provider quick-launch panes (`launchAgent` writes a *provider* id), for
- a pane whose definition was deleted, and during the async window before /
- if `ListAgentDefinitionsCommand` resolves. In every one of those cases the
- gear silently no-op'd — click, nothing.
- Fix: gate the overlay only on `showOverlayTab() != null` and pass
- `currentAgent()` (possibly `undefined`) straight through. The panel chain
- already handles a missing definition — `AgentCardSettingsPanel.agent` is
- typed `AgentDefinition | undefined` (create-mode), and the Identity tab has
- a "save the agent first" fallback. `AgentFocusedPanel`'s `agent` prop is
- widened to `AgentDefinition | undefined` to match what it forwards.
- feat(agent): recency-sorted launch picker + updated_at on agent definitions
- **Recency-sorted picker.** `agent_def_list` previously ordered by
- `created_at ASC` (oldest first). It now orders **most-recently-used first**:
- a `LEFT JOIN` on `MAX(db_agent_instances.started_at)` per definition. Never-
- launched agents (no instance rows → NULL `last_used`) sort after the launched
- ones under `DESC`, ordered among themselves by `created_at ASC`. The
- AgentPicker reflects this order directly.
- **Default selection.** The AgentPicker now focuses the first card (the
- most-recently-used agent) on mount via a new `AgentCard.defaultFocus` prop —
- so the focus ring marks the default and Enter launches it immediately.
- **`updated_at` on agent definitions.** `db_agent_definitions` gains an
- `updated_at` column (schema v2 — `OBJECT_SCHEMA_VERSION` bumped, with an
- idempotent `ALTER TABLE ADD COLUMN` for existing dev databases). It is set to
- `created_at` on insert and stamped fresh on every `agent_def_update`. Surfaced
- on the `AgentDefinition` struct + `gotypes.d.ts` type. (`db_memory_bundles` /
- `db_identity_bundles` already had `updated_at`; agent definitions did not.)
- feat(menu): hamburger order — Settings, Command Palette, DevTools, Online Docs
- Follow-up tweak to PR #936's hamburger menu. The bottom group is reordered
- to **Settings · Command Palette · DevTools · Online Docs**, and the
- "Documentation" item is renamed to **"Online Docs"** (its action — open
- `https://docs.agentmux.ai` in the browser — is unchanged).
- Frontend-only; one block in `tabbar.tsx`.
- docs(readme): fix Widgets table — DevTools removed, Drone added
- The README's Widgets table drifted from `widgets.json`:
- - Removed the **DevTools** widget row — DevTools stopped being a widget in
-   PR #936; it's now a hamburger-menu item. Added a corresponding row to the
-   "Not widgets — opened from elsewhere" table (Hamburger ≡ → DevTools).
- - Added the **Drone** widget row (`diagram-project` icon, More tier) — it
-   was present in `widgets.json` but missing from the README table.
- feat(stability): suppress Windows crash modal (supervision Phase 0)
- feat(authfile): write authkey.dev for all runtime modes so portable builds are discoverable by the benchmark
- feat(stability): launcher auto-restarts the host on crash (supervision Phase 1)
- feat(stability): host retry ladder + crash classification (supervision Phase 1)
- build: task package auto-bumps the patch version every build
- fix(shell): seq-based input reorder buffer + threshold session reset — prevents terminal freeze from out-of-order or concurrent input RPCs
- fix(build): stamp srv binary with the live post-bump version
- feat(statusbar): DEV badge on the version chip
- fix(term): route keyboard input through the blockinput wscommand so fast typing stays in order
- feat(statusbar): Build row shows commit hash, new Time row shows build timestamp
- feat(modal): unified scope-based modal primitive (stage 1)
- refactor(modal): migrate window-scope callers to unified modal (stage 2)
- refactor(modal): migrate tab-scope agent modals to unified modal (stage 3)
- refactor(modal): delete legacy modal-v2 primitive (stage 5)
- refactor(modal): retire v1 modal registry (stage 4)


## 0.37.0 — 2026-05-19

- feat(voice): cherry-pick voice-input building blocks + per-pane spec
- feat(launch-flow-state): wire recordDispatch audit ring (closes §6.8)
- createLaunchFlowStore now appends every dispatch to the global
- audit ring (frontend/app/store/command-source.ts). Each entry tags:
- `slice: "launch-flow-state"`, `key: null`, the command + emitted
- events + source ("user" default, "system" override). The diag panel
- gets transition history for free.
- Closes the final unchecked acceptance criterion of
- docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md.


## 0.36.0 — 2026-05-19

- feat(launch-modal): lift auth controller out of conditional panel + remove blank Identity/Memory (Stage 1)
- Fixes the "memory change → forgot login" repro by lifting the
- AuthFlowController instance from PreLaunchAuthPanel up to
- AgentLaunchModal so its lifetime spans the whole modal mount.
- Conditionally re-rendering the Connect panel no longer destroys
- in-flight auth state.
- Also removes the "blank" Identity/Memory sentinel: both selections
- are now required at submit. Spec:
- docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md
- feat(launch-flow-state): additive reducer slice + tests (Stage 2a)
- Adds `frontend/app/store/launch-flow-state/` — types + pure reducer
- + selectors + 37 unit tests covering the form/identity/memory/
- bindings/submit cross-product. Modeled on browser-pane-state.
- Purely additive — no view migration yet. AgentLaunchModal still uses
- its existing local signals; Stage 2b migrates it. Spec:
- docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md
- feat(launch-modal): migrate form state to launch-flow-state slice (Stage 2b)
- AgentLaunchModal's form fields (name, runtime, image, identity,
- memory, continueOf) now route through the launch-flow-state
- reducer. Adds the Solid-store wrapper + tests; the existing
- accessor names (`name()`, `setName(v)`, …) remain as thin facades
- so call sites are unchanged.
- Stage 2c will migrate the resources (identities/memories/bindings)
- + wire backend push events for cross-tab reactivity.
- feat(launch-modal): migrate submit + error state to launch-flow-state slice (Stage 2c.1)
- AgentLaunchModal's `submitting` / `error` signals now live in the
- reducer. handleSubmit dispatches `SubmitClicked` / `SubmitFailed`
- directly instead of the legacy paired setters. The failure case now
- sets in-flight=false and error=msg in one atomic dispatch.
- Stage 2c.2 (resources) + 2c.3 (bindings + push events) land in
- follow-up PRs.
- feat(launch-modal): migrate identities + memories resources to slice (Stage 2c.2)
- AgentLaunchModal's `identities` and `memories` createResource calls
- are replaced with async wrappers that dispatch
- IdentitiesLoading/Loaded/Failed (and the Memory equivalents) into
- the reducer. `realIdentities` / `realMemories` selectors replace
- the inline `is_blank` filter memos.
- Stage 2c.3 (bindings migration + identitybundlebindings:changed
- subscription) lands next.
- feat(launch-modal): migrate bindings + subscribe to backend push events (Stage 2c.3)
- Final piece of the launch-modal state-machine hardening.
- - `selectedBundleBindings` createResource removed; the reducer
-   emits `FetchBindings` on identity selection and the view's
-   event sink runs the RPC + dispatches BindingsLoading/Loaded.
- - Subscribes to backend's `identitybundlebindings:changed:<id>`
-   event (already emitted on bundle_bind / unbind RPCs) so cross-
-   tab modifications via the Identity pane update this modal
-   without a manual refetch.
- - `bundleHasMatchingBinding` memo delegates to the slice's
-   `hasMatchingBinding(state, providerId)` selector.
- Closes the Stage 2 spec — all of form / submit / resources /
- bindings flow through the reducer slice now.
- feat(launch-flow-state): fold AuthState into the slice (Stage 2d.1)
- `LaunchFlowState.auth: AuthState` is now part of the slice. The
- reducer delegates `{ type: "Auth", cmd }` commands to auth-state's
- pure `update()` and wraps emitted events as
- `{ type: "Auth", event }` on the outer ReducerResult.
- Adds the §6.9 cross-product test suite — 8 tests asserting that
- form-field commands (Name/Memory/Runtime/Image/Identities/
- Bindings/Submit) never touch `state.auth`, and that auth commands
- never touch `state.form`. Pins the original memory-change-resets-
- auth bug as a pure-reducer regression.
- View migration to read `flow.state.auth.kind` is Stage 2d.2.
- feat(launch-modal): route auth state through the slice (Stage 2d.2)
- AuthFlowController accepts optional `externalGetState` +
- `externalDispatch` hooks. AgentLaunchModal wires them so the
- controller reads + writes auth state via the launch-flow-state
- slice. `flow.state.auth` is now the single source of truth;
- internal signal stays as a fallback for tests + standalone use.
- §6.7 satisfied: all Launch modal state is owned by the slice.
- feat(launch-modal): integration test pinning the memory-change regression (Stage 2g, closes spec)
- Adds Vitest + @solidjs/testing-library + jsdom integration test that
- mounts AgentLaunchModalPanel with mocked RPCs, drives the user flow
- programmatically, and asserts the auth panel doesn't reappear after a
- Memory dropdown change. Pins the §6.10 acceptance criterion with a
- fast (~360ms) jsdom-based test instead of standing up real
- Playwright/WebDriver against CEF.
- Approach + library choice rationale:
- docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md.
- Closes Stage 2 of the launch-modal state-machine hardening spec.


## 0.35.0 — 2026-05-19

- feat(browser-pane): HTTP Basic/Digest auth prompt (Phase α)
- feat(agent-picker): pre-launch system prereq check (git for Claude Code/OpenClaw)
- feat(launch-modal): Profile section + New buttons (Phase α)
- feat(launch-modal): New Identity bundle modal (Phase β)
- feat(launch-modal): New Memory bundle modal (Phase γ)
- refactor(drone): rename Workflows feature to Drone (SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18)
- docs(drone): annotate pre-rename specs with rename-pointer headers


## 0.34.0 — 2026-05-18

- devx(phase 2): adopt changesets workflow (RFC #857)
- fix(taskfile): guard dev:serve against orphan vite on :5173 (rebase of #839)
- fix(window-drag): scale CSS-pixel deltas by devicePixelRatio (Win11 125% fix) — rebase of #827
- feat: hamburger menu keyboard shortcuts + Command Palette + per-window opacity slider (rebase of #863, #858)
- feat(linux): enable CEF Wayland window transparency + iterative fixes (rebase of #797)
- fix(taskfile): quote desc strings containing colons (broke task --list + task package after RFC #857 Phase 2)
- fix(launcher): add UpdateWindowMeta + WindowMetaUpdated match arms (pre-existing main breakage)
- fix(linux): multi-window pane crash + new-window invisibility + tab-switch overlay bleed
- Three related Linux-only correctness fixes in the Views pane/window
- machinery:
- - **Pane crash in 2nd/3rd window** (`browser_pane/creation_views.rs`):
-   opening a browser pane in any non-first window FATAL-crashed the CEF host
-   with `observer_list.h:318 NOTREACHED — Observers can only be added once!`.
-   Root cause: every isolated `RequestContext` yields a different `Profile*`
-   pointer but they share one `ThemeService` instance (chrome's
-   `ThemeServiceFactory` redirects to the original profile). The pane passed
-   `None` for `request_context` → different `Profile*` than the parent
-   window's main browser → `CefWidgetImpl::AddAssociatedProfile` map miss →
-   re-`AddObserver` on the shared `ThemeService`. Fix: pane reuses the
-   parent window's `RequestContext` via
-   `state.get_browser(window_label).host().request_context()`.
- - **"New Window" creates a CEF browser but no OS window appears**
-   (`ui_tasks.rs`): with at least one pane open,
-   `state.first_browser()`'s HashMap iteration order could pick a pane
-   browser. The new window inherited its `is_browser_pane=true` client.
-   `on_load_end`'s pane early-return then skipped the `window.show()`
-   call. Fix: filter `list_browsers()` to a top-level (non-pane) browser
-   when picking the client to reuse.
- - **Inactive-tab browser pane bleeds into active tab**
-   (`browser_pane/creation_views.rs::resize_browser_pane_view`): switching
-   tabs left the previous tab's `OverlayController` drawing a borderless
-   residual quad on top of the new tab's DOM. Root cause: frontend sends
-   `browser_pane_resize(0,0,0,0)` when the placeholder goes `display:none`
-   (getBoundingClientRect returns all zeros), but the backend only called
-   `set_size(0,0) + set_position(0,0)` without `set_visible(0)`. On
-   Wayland a 0-sized OverlayController at `set_visible(1)` still
-   composites residual pixels. Fix: toggle `set_visible` based on rect
-   dimensions (`width>0 && height>0`).
- Spec: `docs/specs/multi-window-pane-and-newwindow-fixes-linux-2026-05-15.md`
- feat(browser-pane): live page title and favicon in pane header
- fix(#876): preserve window title on None + free CEF string-list buffer + favicon fallback (reagent P1+P1+P2 commandeer)
- feat(agent-pane): persist reducer state to disk so reopen restores full conversation (no more truncated tables / orphaned tool calls)
- fix(agent-pane): cascade detection + dispatchIfRegistered migration
- - Detect reactive cascades that dispose a pane mid-dispatch (`agent-pane-state-store.ts`, `agent-document-store.ts`); log a `CASCADE_DETECTED` warning identifying the projection setter that triggered the dispose.
- - Add `dispatchIfRegistered` soft-variant on both pane stores; migrate 22 async-context call sites (RAF, setTimeout, setInterval, subscription handlers, RPC `.catch()` continuations) across `useAgentStream.ts`, `useAgentCommands.ts`, `useHistoryPagination.ts`, and `agent-view.tsx` so they silently no-op instead of throwing when the pane disposed mid-dispatch.
- - Guard the `browser-model.reload()` RAF callback with `if (this.closed) return;` to match every other IPC handler in that file.
- Throwing `dispatch()` stays as the contract for synchronous-body register-order checks. Backed by new `agent-pane-state-store.test.ts` covering both contracts (5 tests, 306 total still pass).
- fix(launch-modal): defensive fallback for blank identity/memory labels
- fix(statusbar): show total window count instead of per-window ordinal
- feat(taskfile): add task dev:local mirror of package:local
- fix(dev): restore launcher IPC connection in task dev via parent-process check
- fix(live-log): plumb 'persist' field through WPS publish endpoint
- diag(live-log): temporary tracing for tool_chunk delivery path
- diag(live-log): log reducer tool-chunk drop+append
- diag(live-log): log overlay render-side chunks view
- fix(live-log): tool overlay must access props.node inline (not via createMemo) for live chunk rendering
- feat(live-log): PTY-backed tool streaming + auto-expand inline panel
- feat(agent): install stage Phase α — agent-pane install modal with live npm progress
- feat(openclaw): OAuth login via Codex harness + PTY-backed auth subprocess
- fix(install-modal): add missing SCSS, modal was collapsing to intrinsic size
- feat(errors): error-catalog scaffold (codes + frontend banner) — PR 1 of 3
- feat(errors): migrate CLI resolve + install paths to typed catalog — PR 2 of 3
- feat(errors): migrate auth paths to typed catalog — PR 3 of 3
- fix(install-modal): bind xterm to configured terminal theme
- fix(modal): crossfade chained-modal handoffs via tabModal.replace()
- feat(install-modal): verbose output toggle for npm install
- feat(install-modal): default verbose output to true
- fix(install-modal): copy/paste wiring + unified clipboard spec
- fix(modal): paint-gate entrance animations until content settles
- fix(install-modal): always verbose, drop checkbox
- diag(browser-pane): instrument title/favicon flow end-to-end
- diag(browser-pane): per-instance vmId to detect stale viewmodel refs
- fix(browser-pane): persist viewmodel memos in createRoot (favicon/title stuck on first load)
- fix(browser-pane): reset favicon override on cross-origin navigation


This document tracks the version history of AgentMux (forked from waveterm).

## Latest Version: 0.34.0

**Base:** Upstream waveterm v0.12.0 + extensive custom features

---

## Sizes (CEF portable, Windows x64)

Auto-appended by `scripts/package-cef-portable.sh`. Newest first.

| Version | Date | ZIP (compressed) | Folder (uncompressed) | Note |
|---------|------|------------------|-----------------------|------|
| 0.44.0 | 2026-06-10 | 163.9 MiB | 349.8 MiB | |
| 0.38.13 | 2026-05-26 | 164.8 MiB | 346.2 MiB | |
| 0.38.11 | 2026-05-26 | 164.8 MiB | 346.1 MiB | |
| 0.38.10 | 2026-05-26 | 164.8 MiB | 346.1 MiB | |
| 0.38.6 | 2026-05-26 | 164.8 MiB | 346.1 MiB | |
| 0.37.3 | 2026-05-21 | 164.7 MiB | 345.6 MiB | |
| 0.36.0 | 2026-05-19 | 164.6 MiB | 345.7 MiB | |
| 0.35.0 | 2026-05-19 | 164.6 MiB | 345.6 MiB | |
| 0.33.835 | 2026-05-13 | 164.2 MiB | 344.3 MiB | |
| 0.33.831 | 2026-05-13 | 164.1 MiB | 344.1 MiB | |
| 0.33.827 | 2026-05-12 | 164.1 MiB | 344.0 MiB | |
| 0.33.826 | 2026-05-12 | 164.1 MiB | 344.0 MiB | |
| 0.33.824 | 2026-05-12 | 164.1 MiB | 344.0 MiB | |
| 0.33.823 | 2026-05-12 | 164.1 MiB | 344.0 MiB | |
| 0.33.822 | 2026-05-12 | 164.1 MiB | 343.9 MiB | |
| 0.33.821 | 2026-05-12 | 164.1 MiB | 343.9 MiB | |
| 0.33.814 | 2026-05-12 | 164.1 MiB | 343.9 MiB | |
| 0.33.813 | 2026-05-12 | 164.1 MiB | 343.9 MiB | |
| 0.33.804 | 2026-05-11 | 164.1 MiB | 343.8 MiB | |
| 0.33.799 | 2026-05-11 | 162.4 MiB | 340.2 MiB | |
| 0.33.789 | 2026-05-11 | 162.4 MiB | 340.4 MiB | |
| 0.33.788 | 2026-05-11 | 162.4 MiB | 340.4 MiB | |
| 0.33.787 | 2026-05-11 | 162.4 MiB | 340.4 MiB | |
| 0.33.786 | 2026-05-11 | 162.4 MiB | 340.4 MiB | |
| 0.33.784 | 2026-05-11 | 162.4 MiB | 340.4 MiB | |
| 0.33.733 | 2026-05-08 | 162.3 MiB | 340.3 MiB | |
| 0.33.732 | 2026-05-08 | 162.4 MiB | 340.3 MiB | |
| 0.33.731 | 2026-05-08 | 162.4 MiB | 340.3 MiB | |
| 0.33.726 | 2026-05-08 | 162.3 MiB | 340.3 MiB | |
| 0.33.718 | 2026-05-08 | 162.3 MiB | 340.2 MiB | |
| 0.33.712 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.706 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.703 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.702 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.701 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.700 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.697 | 2026-05-07 | 162.2 MiB | 340.1 MiB | |
| 0.33.696 | 2026-05-06 | 162.2 MiB | 340.1 MiB | |
| 0.33.695 | 2026-05-06 | 162.2 MiB | 340.1 MiB | |
| 0.33.694 | 2026-05-06 | 162.2 MiB | 340.1 MiB | |
| 0.33.693 | 2026-05-06 | 162.2 MiB | 340.1 MiB | |
| 0.33.688 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.685 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.680 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.660 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.655 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.650 | 2026-05-06 | 162.2 MiB | 340.0 MiB | |
| 0.33.647 | 2026-05-05 | 162.2 MiB | 340.0 MiB | |
| 0.33.644 | 2026-05-05 | 162.2 MiB | 340.0 MiB | |
| 0.33.643 | 2026-05-05 | 162.2 MiB | 340.0 MiB | |
| 0.33.624 | 2026-05-03 | 161.9 MiB | 339.0 MiB | |
| 0.33.614 | 2026-05-03 | 161.9 MiB | 339.0 MiB | |
| 0.33.592 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.591 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.590 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.589 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.586 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.585 | 2026-05-02 | 161.9 MiB | 339.0 MiB | |
| 0.33.560 | 2026-05-01 | 161.0 MiB | 337.2 MiB | |
| 0.33.533 | 2026-04-30 | 160.8 MiB | 336.4 MiB | |
| 0.33.512 | 2026-04-29 | 160.5 MiB | 335.6 MiB | |
| 0.33.511 | 2026-04-29 | 160.5 MiB | 335.5 MiB | |
| 0.33.510 | 2026-04-29 | 160.5 MiB | 335.5 MiB | |
| 0.33.509 | 2026-04-29 | 160.4 MiB | 335.1 MiB | |
| 0.33.508 | 2026-04-29 | 160.4 MiB | 335.1 MiB | |
| 0.33.507 | 2026-04-29 | 160.3 MiB | 334.8 MiB | |
| 0.33.506 | 2026-04-29 | 160.3 MiB | 334.8 MiB | |
| 0.33.505 | 2026-04-29 | 160.3 MiB | 334.8 MiB | |
| 0.33.504 | 2026-04-29 | 160.2 MiB | 334.7 MiB | |
| 0.33.503 | 2026-04-29 | 160.2 MiB | 334.7 MiB | |
| 0.33.502 | 2026-04-29 | 160.2 MiB | 334.7 MiB | |
| 0.33.394 | 2026-04-25 | 159.8 MiB | 333.7 MiB | |
| 0.33.372 | 2026-04-24 | 159.8 MiB | 333.6 MiB | |
| 0.33.356 | 2026-04-24 | 159.8 MiB | 333.6 MiB | |
| 0.33.329 | 2026-04-23 | 159.8 MiB | 333.6 MiB | |
| 0.33.319 | 2026-04-22 | 159.8 MiB | 333.4 MiB | |
| 0.33.317 | 2026-04-22 | 159.8 MiB | 333.4 MiB | |
| 0.33.314 | 2026-04-22 | 159.8 MiB | 333.4 MiB | |
| 0.33.311 | 2026-04-21 | 159.8 MiB | 333.4 MiB | |
| 0.33.298 | 2026-04-20 | 159.7 MiB | 333.1 MiB | |
| 0.33.296 | 2026-04-20 | 159.7 MiB | 333.1 MiB | |
| 0.33.284 | 2026-04-19 | 159.7 MiB | 333.1 MiB | |
| 0.33.245 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.240 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.234 | 2026-04-17 | 158.6 MiB | 330.8 MiB | |
| 0.33.233 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.232 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.231 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.230 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.229 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.228 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.227 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.226 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.225 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.224 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.223 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.222 | 2026-04-17 | 158.6 MiB | 330.9 MiB | |
| 0.33.221 | 2026-04-17 | 158.6 MiB | 330.8 MiB | |
| 0.33.220 | 2026-04-17 | 158.6 MiB | 330.8 MiB | |
| 0.33.219 | 2026-04-17 | 158.6 MiB | 330.8 MiB | |
| 0.33.214 | 2026-04-16 | 158.6 MiB | 330.8 MiB | |
| 0.33.199 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.198 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.197 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.196 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.195 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.194 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.193 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.192 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.191 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.190 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.189 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.188 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.187 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.186 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.185 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.182 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.179 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.177 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.175 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.173 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.171 | 2026-04-15 | 158.3 MiB | 330.0 MiB | |
| 0.33.169 | 2026-04-14 | 158.3 MiB | 330.0 MiB | |
| 0.33.168 | 2026-04-14 | 158.3 MiB | 330.0 MiB | |
| 0.33.166 | 2026-04-14 | 155.9 MiB | 323.7 MiB | |
| 0.33.164 | 2026-04-14 | 155.9 MiB | 323.7 MiB | |
| 0.33.163 | 2026-04-14 | 155.9 MiB | 323.7 MiB | |
| 0.33.155 | 2026-04-14 | 155.0 MiB | 318.8 MiB | |
| 0.33.145 | 2026-04-14 | 155.0 MiB | 318.8 MiB | |
| 0.33.142 | 2026-04-14 | 155.0 MiB | 318.8 MiB | |
| 0.33.122 | 2026-04-13 | 155.5 MiB | 319.9 MiB | |
| 0.33.110 | 2026-04-13 | 155.5 MiB | 319.9 MiB | |
| 0.33.102 | 2026-04-12 | 155.5 MiB | 320.0 MiB | post-ultra-long-sessions |
| 0.33.91 | 2026-04-12 | 151.8 MiB | 320.8 MiB | pre-ultra-long-sessions |
| (pre-ANGLE) | 2026-03-29 | 147.7 MiB | 309.8 MiB | no libEGL/libGLESv2 |

---

## Version History (Latest First)

### v0.32.89 (2026-03-25)
- **Agent:** Claude Sonnet 4.6
- **Changes:**
  - fix: prevent zombie agent processes from causing sustained high CPU — SIGTERM/SIGKILL on stop(), sysinfo dead-PID eviction, delete_block now calls delete_controller to avoid registry leak
  - fix: xterm.js cursor blink rAF loop — cursorBlink: false by default, re-enabled on focus

### v0.32.83 (2026-03-24)
- **Agent:** Claude Sonnet 4.6
- **Changes:**
  - fix: pin macOS window buttons on chrome zoom — use width:100% not calc(100vw/factor)

### v0.32.9-fork (2026-03-17)
- **Agent:** Claude Sonnet 4.6
- **Changes:**
  - fix: Linux AppImage cog icon — replace .DirIcon absolute symlink with real file copy
  - fix: backspace regression on Linux — force xterm.js Canvas renderer on WebKitGTK
  - fix: remove stale versioned .desktop registration (Wayland app_id = binary name)
  - docs: CLAUDE.md — document recurring Linux issues to prevent regressions

### v0.31.42-fork (2026-03-04)
- **Agent:** AgentX
- **Changes:**
  - feat: add bottom status bar (backend health, connections, config errors, update status, version)
  - fix: version-specific bundle identifier (v0-31) for macOS multi-instance coexistence

### v0.31.18-fork (2026-03-02)
- **Agent:** AgentA
- **Changes:**
  - feat: add Copy and Paste to pane right-click context menu
  - Copy reads xterm.js selection for terminals, window.getSelection() for others
  - Paste uses async clipboard read + terminal.paste() for terminal panes

### v0.31.17-fork (2026-03-02)
- **Agent:** AgentA
- **Changes:**
  - fix: correct package:portable:linux build — add missing copy:schema step, fix AppDir path, preserve AppImage output

### v0.31.16-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - feat: pane right-click context menu — Split Up/Down/Left/Right + Open in VSCode

### v0.31.15-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - chore: deploy build

### v0.31.14-fork (2026-03-01)
- **Agent:** AgentA
- **Changes:**
  - fix: restore shell integration scripts — deploy bash/zsh/pwsh/fish hooks on terminal start
  - fix: inject AGENTMUX_BLOCKID, AGENTMUX_TABID, TERM_PROGRAM env vars into PTY
  - fix: pane title and color from WAVEMUX_AGENT_ID now works again

### v0.31.13-fork (2026-02-28)
- **Agent:** AgentA
- **Changes:**
  - fix: agent Connect button — capture OAuth URL from PTY and open browser
  - feat: full-screen "Waiting for authorization" overlay during auth flow
  - docs: AGENT_AUTH_STATE_MACHINES.md — state machine reference

### v0.31.12-fork (2026-02-28)
- **Agent:** AgentA
- **Changes:**
  - feat: window instance indicator (1), (2) in title bar
  - docs: updated README, BUILD, CONTRIBUTING for 100% Rust stack

### v0.31.9-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - perf: Convert Hack Nerd Mono fonts from TTF to WOFF2 (-5 MB)
  - perf: Exclude duplicate Monaco workers and NLS locales from static copy (-11.2 MB)
  - perf: Lazy-load WaveStreamdown to defer shiki (9.4 MB) to on-demand
  - perf: Strip redundant KaTeX TTF/WOFF fonts from build output (-876 KB)

### v0.31.4-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - Simplified agent view to single Connect button flow
  - Removed debug logs and unused barrel exports from agent widget
  - Fixed #351: add copy:schema task to Taskfile.yml (dist/schema/ missing after clean)
  - Added screenshot patterns to .gitignore
  - Added debugging quick reference to CLAUDE.md

### v0.31.3-fork (2026-02-21)
- **Agent:** AgentA
- **Changes:**
  - Fix CLI auth flow: correct state machine, proper --verbose flag for stream-json
  - Remove hardcoded OAuth URL (was wrong endpoint)
  - Auth status check before session start

### v0.31.2-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - Multi-provider CLI onboarding, auth management, and session abstraction
  - New providers/ directory with Claude, Codex, Gemini translator stubs
  - SetupWizard component for first-run onboarding
  - Rust backend providers.rs for multi-provider CLI auth checks
  - SPEC_CLAUDE_CLI_INTEGRATION.md design doc

### v0.31.0-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - 100% Rust release: removed all Go source code (cmd/, pkg/, go.mod, go.sum)
  - wsh rewritten in Rust (wsh-rs crate): 1.1 MB binary vs 11 MB Go (90% size reduction)
  - Added sysinfo data collection to Rust backend (CPU, memory, network graphs)
  - Added getmeta, setmeta, waveinfo RPC handlers to Rust backend
  - Updated build system: all build tasks now use cargo (no Go/CGO dependency)
  - Binary size: agentmuxsrv-rs 4.4 MB + wsh 1.1 MB = 5.5 MB total (vs ~25 MB Go)

### v0.30.8-fork (2026-02-20)
- **Agent:** AgentA
- **Changes:**
  - Tree shake: delete 8 dead Rust modules (wcloud, shellutil, webhookdelivery, suggestion, telemetry, faviconcache, blocklogger, authkey)
  - Suppress 911 compiler warnings with #![allow(dead_code)] on Go-port modules
  - Remove all Electron references from frontend (rename ElectronApi → AppApi, ElectronContextMenuItem → NativeContextMenuItem, etc.)
  - Archive old docs/specs, reorganize debug scripts
  - Net removal of 3,449 lines of dead code

### v0.30.6-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Fix grey screen on startup: add 5s RPC timeouts and error recovery
  - showStartupError() renders user-facing error instead of blank screen
  - 30s safety-net timeout forces body visible if still hidden

### v0.30.5-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Modularize filestore.rs (1531 lines) into 7 focused files under filestore/ directory
  - No behavior changes — pure mechanical extraction
  - All 34 filestore tests pass

### v0.30.4-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Fix widgets, config event, and object CRUD in Rust backend

### v0.30.3-fork (2026-02-19)
- **Agent:** AgentA
- **Changes:**
  - Terminal I/O with real PTY support (portable-pty) in Rust backend
  - Wire controllerresync, controllerinput RPC handlers
  - Wire blockinput, setblocktermsize wscommands
  - Wire eventsub/eventunsub/eventunsuball to WPS Broker
  - Add EventBusBridge for Broker → EventBus → WebSocket event delivery
  - Replace unsafe run_lock pointer with safe Arc<AtomicBool>

### v0.30.0-fork (2026-02-17)
- **Agent:** AgentO
- **Changes:**
  - Rust backend parity fixes: match Go response shapes for all startup RPC calls
  - Fix meta null/empty serialization, otype in GetObject, isnew/pos/winsize defaults
  - Fix ListWorkspaces, GetAllConnStatus, tab naming, pinned tabs
  - Add parity test harness (scripts/parity-test.sh) — 8/8 tests pass
  - Default sidecar to Rust backend (agentmuxsrv-rs)

### v0.29.1-fork (2026-02-17)
- **Agent:** AgentX
- **Changes:**
  - Fix Linux AppImage build: use appimagetool when linuxdeploy crashes
  - Add agentmuxsrv-rs (Rust backend) to package:portable:linux build pipeline
  - Fix icon naming issue in AppDir (AgentMux.png → agentmux.png for desktop file)
  - Add scripts/build-appimage.sh with dynamic version and clear step ordering

### v0.29.0-fork (2026-02-16)
- **Agent:** AgentO
- **Changes:**
  - Wire Rust backend (agentmuxsrv-rs): replace all 501 stubs with real handlers
  - Implement full service dispatch (30+ methods: object, client, window, workspace, block, userinput)
  - Wire file endpoint, 9 reactive endpoints, WebSocket, AI chat SSE streaming, schema/docsite
  - Backend initialization: WaveStore, FileStore, EventBus, Broker, ReactiveHandler, Poller
  - Binary 9x smaller (3.1MB vs 28.5MB), memory 3.6x lower, latency 19-44% faster than Go
  - All 1089 unit tests + 4 integration tests pass

### v0.28.20-fork (2026-02-16)
- **Agent:** AgentO
- **Changes:**
  - Harden E2E tests: replace browser.pause() with proper waitUntil waits
  - Add data-testid attributes to UI components for stable test selectors
  - Create macOS-compatible WDIO config with mocked Tauri IPC
  - Add window-controls and layout regression test specs
  - Add byTestId() and waitForZoomChange() test helpers
  - Update SPEC_E2E_TESTING_MACOS.md with implementation details

### v0.28.5-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Remove notification bell icon from widget bar (unused dev-only UI)

### v0.28.4-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: zsh "no matches found: wsh-*" error in shell integration
  - Use zsh (N) nullglob qualifier for portable wsh detection
  - Prevents zsh nomatch error when no wsh-* files exist in app directory

### v0.28.3-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: Deploy wsh binary on macOS for shell integration
  - Set WAVETERM_APP_PATH env var so Go backend can locate wsh
  - Runtime copy of bundled wsh to bin/ with correct versioned name
  - Sync wsh binaries for dev mode in Taskfile.yml

### v0.27.14-fork (2026-02-15)
- **Agent:** AgentO
- **Changes:**
  - Fix: Skip systray on macOS to prevent backend crash (CGO signal fault in getlantern/systray)
  - Resolves blank screen issue on macOS ARM64

### v0.27.11-fork (2026-02-15)
- **Agent:** AgentX
- **Changes:**
  - Feat: Phase 5 - Unified Agent Widget Registration & Integration
  - Fix: Complete state scoping refactor - per-instance atoms to prevent state bleeding
  - Created AgentViewModel for state management and terminal streaming
  - Registered agent widget in block registry and widget config
  - Added AgentViewWrapper to bridge ViewModel and component interfaces
  - Enhanced stream parser with parseEvent() method
  - Users can now create and use unified agent widgets from UI
  - Completes Phases 1-5 of unified agent widget implementation

### v0.27.10-fork (2026-02-15)
- **Agent:** AgentX
- **Changes:**
  - Feat: Robust shell integration with self-healing
  - Add version guard to detect stale shell integration files
  - Implement multi-strategy wsh binary discovery (portable > installed > PATH)
  - Add defensive execution with graceful degradation
  - Wrap all wsh calls in Test-WshAvailable checks
  - Use -ErrorAction SilentlyContinue on all cleanup operations
  - Add template versioning support (AGENTMUX_VERSION, TIMESTAMP)

### v0.27.9-fork (2026-02-14)
- **Agent:** AgentX
- **Changes:**
  - Feat: Add `package:macos` task for platform-specific macOS builds
  - Creates .app and .dmg bundles on macOS
  - Documented CGO code signing limitations and workarounds
  - Fixed ExpectedVersion constant synchronization

### v0.26.0 (2026-02-12)
- **Agent:** AgentClaude
- **Changes:**
  - Feat: Display AgentMux version in tabbar (centered, clickable to copy)
  - Feat: Enable window dragging from entire tabbar area
  - Feat: Add right-click context menu to toggle widget visibility
  - Fix: Add macOS-specific version bump script (bump-version-osx.sh)

### v0.16.7 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Auto-load agentmux config from file on startup
  - Add LoadAgentMuxConfigFile() to load ~/.waveterm/agentmux.json
  - Add SaveAgentMuxConfigFile() to persist runtime config changes
  - ReconfigureGlobalPoller() now saves config to file automatically
  - No pre-configuration needed - just place agentmux.json and restart
  - Priority: config file < env vars (env vars override file config)

### v0.16.6 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Runtime agentmux config via wsh agentmux command
  - Add ReconfigureGlobalPoller() for runtime poller updates
  - Add HTTP endpoints: /wave/reactive/poller/config, /status
  - Add OSC 16162 "X" command for agentmux config
  - New wsh commands: `wsh agentmux config`, `wsh agentmux status`
  - Allows configuring AgentMux without restarting AgentMux

### v0.16.5 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Revert to synchronous Enter key for reactive injection
  - Add rate limiter (10 req/sec) for DoS protection
  - Docs: Add REACTIVE_INJECTION_REGRESSION_REPORT.md

### v0.16.4 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Enter key retry with 3 attempts (still broken)
  - Added documentation for the issue

### v0.16.3 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Fix: Enter key timing for reactive injection (300ms delay, CRLF)
  - Added retry after 700ms (still broken)

### v0.16.2 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Made Enter key async to prevent DoS (breaking change)
  - This change broke message/Enter coordination

### v0.16.1 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Cross-host reactive messaging poller (#144)
  - AgentMux polls AgentMux for pending injections from remote agents
  - New endpoint: /wave/reactive/poller/stats for monitoring
  - Configurable via AGENTMUX_URL, AGENTMUX_TOKEN env vars
  - Enables agent-to-agent messaging across different machines

### v0.16.0 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Feat: Reactive agent-to-agent messaging (#140, #141)
  - Inject messages directly into running Claude Code instances
  - New HTTP API: /wave/reactive/inject, /agents, /register, /unregister, /audit
  - Frontend auto-registers agents via OSC 16162 WAVEMUX_AGENT_ID
  - Message sanitization and audit logging
  - AgentMux inject_terminal MCP tool (agentmux#69)

### v0.15.15 (2026-01-16)
- **Agent:** AgentX
- **Changes:**
  - Feat: add WAVEMUX_AGENT_TEXT_COLOR support for pane header text (#137)
  - Customizable text color for agent pane headers
  - Smart defaults: white text on dark backgrounds, black on light

### v0.15.14 (2026-01-16)
- **Agent:** AgentA
- **Changes:**
  - Refactor: remove AGENTMUX_AGENT_ID coupling (#135)
  - AgentMux now only uses WAVEMUX_AGENT_ID for agent identity
  - Shell integration scripts cleaned of AGENTMUX fallbacks

### v0.15.13 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Fix: prevent duplicate title display in pane header and titlebar (#134)
  - Fix: decouple system hostname from agent detection (#132)

### v0.15.12 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Docs: update VERSION_HISTORY.md to reflect current state (#130)

### v0.15.9 - v0.15.11 (2026-01-15)
- **Agent:** AgentA
- **Changes:**
  - Fix: disable hostname-based agent detection for local terminals (#127)
  - Local terminals no longer auto-detect agent from hostname patterns
  - Explicit `agent-workspaces` directory pattern still works
  - Env vars (WAVEMUX_AGENT_ID) take highest priority

### v0.15.5 - v0.15.8 (2026-01-14)
- **Agent:** AgentX
- **Changes:**
  - Fix: Claude activity display - no duplicate, bold in header (#126)
  - Fix: per-pane agent identification + build system fixes (#125)
  - Fix: re-enable hardware acceleration by default (#123)

### v0.15.4 (2026-01-13)
- **Agent:** AgentX
- **Changes:**
  - Feat: add AgentY to default agent colors (#122)
  - Feat: Display Claude activity summaries in pane title bar (#121)
  - Feat: per-pane agent colors via shell environment variables (#120)
  - Fix: improve agent detection path matching with trailing slash (#119)

### v0.15.0 - v0.15.3 (2026-01-12)
- **Agent:** AgentX
- **Changes:**
  - Feat: Add agent colors to terminal pane headers (#103)
  - Feat: environment variable-based agent detection (#102)
  - Disable Dependabot - causing too many blockers (#118)
  - Sync missing aiprompts files from upstream waveterm

### v0.14.0 (2026-01-09)
- **Changes:**
  - Removed Storybook (unused dev tool, ~36MB savings)
  - Removed Storybook references from Dependabot config
  - Fixed remote desktop startup failures (reverted to simple 1-terminal layout)
  - Disabled hardware acceleration for Windows Sandbox/RDP compatibility
  - Added console window with verbose startup logging
  - Multiple dependency updates (xterm, monaco, react-hook-form, etc.)

### v0.13.3 - v0.13.6 (2026-01-08)
- **Changes:**
  - Various hardware acceleration and startup fixes
  - Window size calculation debugging
  - Layout fixes for remote desktop

### v0.12.14-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Agent:** agentx
- **Changes:**
  - **P0 FIX:** Cross-platform wsh binary exclusions (breaks macOS/Linux builds)
  - **P1 FIX:** Updater IPC handler crash when auto-update disabled
  - Added RELEASE_CHECKLIST.md with comprehensive workflow guide
  - Enhanced bump-version.sh to prevent releasing old code
  - Documented correct release workflow to prevent v0.12.13 issue recurrence

### v0.12.13-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Agent:** agentx
- **Changes:**
  - Fix title bar instance number parsing bug (was showing "undefined")
  - Add comprehensive app name and instance tests
  - **NOTE:** This version was released BEFORE instance parsing fix was committed
  - **ISSUE:** Users downloaded old code under new version number
  - **RESOLUTION:** v0.12.14 includes all fixes with corrected workflow

### v0.12.12-fork (2025-10-20)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Changes:**
  - Package verification and version consistency fixes

### v0.12.11-fork (2025-10-20)
- **Changes:**
  - Version management improvements and documentation

### v0.12.10-fork (2025-10-19)

- **Branch:** `feature/high-contrast-terminal-borders`
- **Changes:**
  - Fix waveConfigDirName undefined error
  - Add smoke tests for configuration

### v0.12.9-fork (2025-10-19)
- **Changes:**
  - Fix waveDirName undefined error
  - Add more configuration tests

### v0.12.8-fork (2025-10-19)
- **Changes:**
  - Implement portable multi-instance mode with persistent settings

### v0.12.7-fork (2025-10-19)
- **Changes:**
  - UI improvements: hard corners, better borders
  - Fix settings persistence issues
  - Optimize build size: remove heavy artifacts

### v0.12.6-fork (2025-10-19)
- **Changes:**
  - Add comprehensive crash reporting system

### v0.12.3-fork (2025-10-19)

- **Agent:** agentx
- **Branch:** `agentx/merge-upstream-v0.12.0`
- **Changes:**
  - Add high-contrast white borders to unselected terminal blocks
  - Fix electron-builder packaging bug (upgrade to v26.1.0)
  - Document critical electron-builder files configuration bug
  - Add build investigation spec and artifact verification
  - Added multi-instance development support
  - Added comprehensive documentation (BUILD.md, CLAUDE.md)
  - **Added version management scripts (bump-version.sh/ps1) and this VERSION_HISTORY.md**

### v0.12.2-fork
- Multi-instance support improvements
- Multi-instance dialog

### v0.12.1-fork
- Inherit main install profile
- Initial multi-instance support with shared config

### v0.12.0-fork
- Initial merge from upstream v0.12.0

---

## Upstream

- **Upstream:** https://github.com/wavetermdev/waveterm (base v0.12.0)
- **Fork:** https://github.com/agentmuxai/agentmux

## Version Bumps

Always use [`@a5af/bump-cli`](https://github.com/a5af/bump-cli) — never edit version numbers manually.

```bash
bump patch -m "Description" --commit
bump verify
```

See `.bump.json` for config and [BUILD.md](./BUILD.md) for the full workflow.

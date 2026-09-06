# Specs Index

Navigational index of authoritative specs by subsystem. For each area the most
recent or canonical spec is listed first; earlier specs are shown when they
document a different decision or design phase.

See also:
- [`docs/analysis/`](../analysis/) — root-cause analyses and investigations
- [`docs/retro/`](../retro/) — incident retrospectives
- [`docs/architecture/`](../architecture/) — long-lived architecture docs

---

## Agent Pane

| Spec | Summary |
|---|---|
| [SPEC_AGENT_ARCHITECTURE_2026_05_27](SPEC_AGENT_ARCHITECTURE_2026_05_27.md) | Overall agent-pane component + state machine |
| [SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15](SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md) | ACP wire protocol (initialize / session_create / tool_result) |
| [SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23](SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md) | Pane lifecycle states and transitions |
| [SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15](SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md) | Cross-session state persistence |
| [SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16](SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md) | Launch + handshake sequence |
| [SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16](SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md) | UI behaviour on agent failure / crash |
| [SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11](SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md) | Failure diagnosis overlay |
| [SPEC_LIVE_LOG_PTY_REWORK_2026_05_16](SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md) | PTY-based live log streaming |
| [SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12](SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md) | Session-replay / history rehydration |
| [SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27](SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md) | Tab-switch paint performance |

## Agent Picker / Launch Modal

| Spec | Summary |
|---|---|
| [SPEC_AGENT_PICKER_TILE_GRID_2026_06_17](SPEC_AGENT_PICKER_TILE_GRID_2026_06_17.md) | Latest tile-grid picker design |
| [SPEC_AGENT_PICKER_TWO_TIER_2026_05_24](SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md) | Two-tier picker (provider → model) |
| [SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19](SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md) | Launch-modal state machine |
| [SPEC_AGENT_INSTALL_STAGE_2026_05_17](SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) | Provider install / setup stage |

## Agent API surface

| Spec | Summary |
|---|---|
| [SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17](SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md) | App-API `agent.*` surface (open/fork/define) |
| [SPEC_APP_API_AGENT_DEFINE_2026_06_06](SPEC_APP_API_AGENT_DEFINE_2026_06_06.md) | `agent.define` RPC |
| [SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06](SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md) | Agent fork / continuation across sessions |
| [SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21](SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md) | Quick-fork an agent into a new tab (hot clone, full identity) |
| [SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22](SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md) | Display naming/addressing scheme shared by the fork and pane-mirror specs |
| [SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16](SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md) | Agent definitions shared across workspaces |
| [SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13](SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md) | Cross-channel conversation continuity |
| [SPEC_ASK_USER_QUESTION_2026_06_15](SPEC_ASK_USER_QUESTION_2026_06_15.md) | `ask_user_question` tool |
| [SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06](SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md) | 30s auto-timeout + countdown, auto-selects the recommended option |
| [SPEC_CONTEXT_VISIBILITY_2026_06_17](SPEC_CONTEXT_VISIBILITY_2026_06_17.md) | Context-meter / token-budget visibility |

## Shell / Terminal Pane

| Spec | Summary |
|---|---|
| [SPEC_BULLETPROOF_TERMINALS_2026_05_21](SPEC_BULLETPROOF_TERMINALS_2026_05_21.md) | Terminal resilience requirements |
| [SPEC_PERSISTENT_SHELL_NODE_2026_06_11](SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md) | Long-lived shell nodes (persistent PTY) |
| [SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14](SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14.md) | Stop / teardown for persistent shells |
| [SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29](SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) | Input-latency spec |

## Browser Pane

| Spec | Summary |
|---|---|
| [SPEC_BROWSER_PANE_LIFECYCLE](SPEC_BROWSER_PANE_LIFECYCLE.md) | Lifecycle: mount, navigate, unmount |
| [SPEC_BROWSER_DOM_API](SPEC_BROWSER_DOM_API.md) | DOM injection / bridge API |
| [SPEC_BROWSER_PANE_FAVICON_TITLE_2026-05-15](SPEC_BROWSER_PANE_FAVICON_TITLE_2026-05-15.md) | Favicon + title propagation |

## Editor Pane

| Spec | Summary |
|---|---|
| [SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14](SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md) | Editor-widget default UX |
| [SPEC_EDITOR_AND_APP_FIND_2026_06_17](SPEC_EDITOR_AND_APP_FIND_2026_06_17.md) | Find / replace integration |
| [SPEC_EDITOR_FILE_ENCODINGS_2026_06_17](SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md) | File encoding handling |
| [SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16](SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md) | `openEditor` API + tree collapse |

## Floating Panes / Tear-off

| Spec | Summary |
|---|---|
| [SPEC_FLOATING_PANE_TEAROFF_2026_05_11](SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md) | Tear-off design (all platforms) |
| [SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26](SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md) | Cross-platform specifics |
| [SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29](SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md) | macOS tear-off |
| [SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30](SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md) | Linux tear-off |
| [SPEC_FLOATING_PANE_REDOCK_2026-05-27](SPEC_FLOATING_PANE_REDOCK_2026-05-27.md) | Re-dock design |
| [SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05](SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05.md) | Native-loop drag + resize |

## Layout / Workspace

| Spec | Summary |
|---|---|
| [SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01](SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md) | Layout reducer architecture |
| [SPEC_HOST_REDUCER_PHASE_H_2026-05-02](SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md) | Host-process reducer (phase H) |
| [SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14](SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14.md) | Workspace state sync to sidecar |
| [SPEC_MODAL_COMPACT_VARIANT_2026_05_25](SPEC_MODAL_COMPACT_VARIANT_2026_05_25.md) | Compact modal variant |
| [SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11](SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md) | Overlay clipping |
| [SPEC_TAB_CONTENT_REVEAL_GATE](SPEC_TAB_CONTENT_REVEAL_GATE.md) | Whole-tab-switch paint-cascade flicker fix (hide-until-settled gate) |
| [SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22](SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md) | Generalizes the reveal gate to leaf/pane scope — fixes flicker on "+", Quick Fork, Agent History |

## Authentication / OAuth

| Spec | Summary |
|---|---|
| [SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14](SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md) | Pre-launch OAuth flow |
| [SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14](SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14.md) | Auth state machine |
| [SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22](archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md) | Identity bundles for multi-account (archived — see issue #2024) |
| [SPEC_PROVIDER_PINNED_AUTH_2026_06_05](SPEC_PROVIDER_PINNED_AUTH_2026_06_05.md) | Per-provider pinned credentials |

## Providers

| Spec | Summary |
|---|---|
| [SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14](SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md) | Unified model+effort abstraction |
| [SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02](SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02.md) | Qwen + Aider providers |
| [SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18](SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md) | System-level provider prereqs |

## Data / Storage

| Spec | Summary |
|---|---|
| [SPEC_DATA_DIR_UNIFICATION_2026-05-05](archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md) | Data-dir unification (DataPaths) |
| [SPEC_PERSISTENCE_LAYER_ANALYSIS_2026-05-14](SPEC_PERSISTENCE_LAYER_ANALYSIS_2026-05-14.md) | Persistence-layer design |
| [SPEC_DATA_CHANNELS_2026_05_24](SPEC_DATA_CHANNELS_2026_05_24.md) | Data-channel multiplexing |

## Messaging / MuxBus

| Spec | Summary |
|---|---|
| [SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15](SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md) | MuxBus delivery hierarchy |
| [SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16](SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.md) | Agent discovery + persistent delivery |
| [SPEC_OBJ_UPDATE_BRIDGE_2026-05-14](SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md) | Obj-update bridge (sidecar↔renderer) |
| [SPEC_CROSS_PROCESS_DISPATCH_2026-05-01](SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) | Cross-process dispatch architecture |
| [SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21](SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21.md) | Mirrored agent panes across channels/LAN/WAN — reuses the jekt trust model for input authorization |

## Packaging / Build

| Spec | Summary |
|---|---|
| [SPEC_MSIX_PACKAGING_2026_05_30](SPEC_MSIX_PACKAGING_2026_05_30.md) | Windows MSIX packaging |
| [SPEC_MACOS_PACKAGING_2026_05_30](SPEC_MACOS_PACKAGING_2026_05_30.md) | macOS packaging + signing |
| [SPEC_LOCAL_BUILD_VERSIONING_2026_05_28](SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md) | Local-build version stamps |
| [SPEC_BUNDLE_MANAGEMENT_2026_05_22](archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md) | Bundle lifecycle management (archived — see issue #2024) |
| [SPEC_PORTABLE_SOURCE_MAPS_2026_06_01](SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md) | Source-map shipping in portable builds |

## Platform-specific

| Spec | Summary |
|---|---|
| [SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03](SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03.md) | macOS native menu bar |
| [SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03](SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md) | macOS accessibility |
| [SPEC_MACOS_WINDOW_CLOSE_LIFECYCLE_2026-06-04](SPEC_MACOS_WINDOW_CLOSE_LIFECYCLE_2026-06-04.md) | macOS window-close flow |
| [SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13](SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md) | Linux GPU backend selection |
| [SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05](SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md) | Linux launcher + splash |
| [SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03](SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md) | Multi-instance isolation |

## Design System

| Spec | Summary |
|---|---|
| [SPEC_DESIGN_SYSTEM_2026_04_23](SPEC_DESIGN_SYSTEM_2026_04_23.md) | Design-system tokens + components |
| [SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION](SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md) | Perf instrumentation spec |
| [SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21](SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21.md) | Zoom / DPI implementation |

---

<!-- BEGIN GENERATED INDEX — edit scripts/gen-docs-index.sh, not this section -->

## All specs by status

Generated by `scripts/gen-docs-index.sh` — do not hand-edit. Covers
every spec directly in `docs/specs/`, which the curated sections
above deliberately do not.

`archive/` is excluded on purpose — it means "not worth reading unless you
are doing history". Everything else is here; the completeness
assertion in the generator fails the build rather than emit a
partial list.

### implemented (134)

| Spec | Title |
|---|---|
| [`PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01`](PLAN_DOCS_CLEANUP_EXECUTION_2026_09_01.md) | Docs cleanup — execution plan |
| [`PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02`](PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md) | Plan — consolidate the agent pane's two (really three) separate login CTAs |
| [`PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03`](PLAN_MUXBUS_KEYCHAIN_WINDOWS_BLOB_LIMIT_2026_08_03.md) | Plan — fix MuxBus token persistence on Windows (Credential Manager 2560-byte cap) |
| [`SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02`](SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md) | Spec: ABF Import UI (Phase 3) — Selective Import + Collision Handling |
| [`SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01`](SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md) | Spec: ABF v0.1 — Single-File Format + Importer (Phase 2) |
| [`SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23`](SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md) | Activity Dock: coalesce event-triggered refreshes on pane reopen |
| [`SPEC_AGENT_COLOR_2026_08_08`](SPEC_AGENT_COLOR_2026_08_08.md) | SPEC: Per-agent color — assign at creation, backfill existing, show on the pane frame |
| [`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07`](SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md) | SPEC: GitHub review-notification agent detection — username-first, tag as fallback |
| [`SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20`](SPEC_AGENT_PANE_ARMORY_HEADER_ICON_2026_07_20.md) | SPEC: Vault Icon on the Agent-Setup Button + Responsive Tabs in the Per-Agent "Armory" |
| [`SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26`](SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md) | Agent Pane Mount/Auth Notifications & Launch-Auth Reducer |
| [`SPEC_AGENT_PANE_FIRST_OVERFLOW_SCROLL_PIN_FIX_2026_08_29`](SPEC_AGENT_PANE_FIRST_OVERFLOW_SCROLL_PIN_FIX_2026_08_29.md) | Spec: force stick-to-bottom on an agent pane's first-ever overflow |
| [`SPEC_AGENT_PANE_MOUNT_AUTH_CHECK_WRONG_DIR_2026_07_31`](SPEC_AGENT_PANE_MOUNT_AUTH_CHECK_WRONG_DIR_2026_07_31.md) | SPEC — Agent-pane mount-time auth check validates the wrong directory |
| [`SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10`](SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md) | SPEC: Move the agent pane's marching-ants progress bar above the tab strip |
| [`SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24`](SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md) | SPEC: Agent pane — fix silent auto-scroll-follow drops, extend the message-list scrollbar past the Working/Host status rows |
| [`SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10`](SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md) | SPEC: Agent pane tab strip floats over the conversation, doesn't reserve a row |
| [`SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24`](SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.md) | Spec: Agent Pane Zone Reorder + Enriched "Worked" Footer |
| [`SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21`](SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md) | SPEC: Quick-fork an agent into a new pane-stack tab (hot clone, full identity) |
| [`SPEC_AGENT_RUNTIME_DROPUP_CLOSE_BUTTON_2026_08_07`](SPEC_AGENT_RUNTIME_DROPUP_CLOSE_BUTTON_2026_08_07.md) | SPEC: Explicit close button on the Runtime (Mode/Model/Effort) dropup |
| [`SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02`](SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md) | SPEC: Agent Pane Session Cost/Token Totals |
| [`SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08`](SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md) | SPEC: Open the agent-pane Shell drawer below the composer, not above it |
| [`SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14`](SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md) | Agent shell drawer: PSReadLine thaw resize causes a visible ~9px width blip ~300-350ms after open |
| [`SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10`](SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md) | Agent shell drawer: font-size seed race causes zoom jerk on open |
| [`SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16`](SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md) | SPEC: Agent Startup Sequence |
| [`SPEC_AGENT_TOOL_CALL_TONES_2026_06_05`](SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md) | SPEC — Agent tool-call tones (subliminal "talking" voice) |
| [`SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18`](SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md) | SPEC: Agent turn-phase timeline — unified, replayable phase-history logging + `muxlog phases` |
| [`SPEC_AGENT_VIEW_SCSS_SPLIT_2026_04_24`](SPEC_AGENT_VIEW_SCSS_SPLIT_2026_04_24.md) | Spec: agent-view.scss Decomposition |
| [`SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01`](SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md) | Working row: stand down on promotion, and sit above the composer |
| [`SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17`](SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md) | SPEC: Pane title tracks the session's overall goal, not the latest micro-step |
| [`SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09`](SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md) | SPEC — Armory "Bind to Agent" context menu on account rows |
| [`SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22`](SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md) | Spec: Armory rail — "Global Memory" / "Personal Memory" rename + reposition |
| [`SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03`](SPEC_ASK_USER_QUESTION_ACCEPT_RECOMMENDED_BUTTON_2026_09_03.md) | SPEC: "Accept Recommended" button for AskUserQuestion |
| [`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06`](SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md) | SPEC: Auto-timeout for AskUserQuestion — 30s countdown, auto-select the recommended option |
| [`SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17`](SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md) | SPEC: Answered questions render as user input + inverted user-input surface |
| [`SPEC_ASK_USER_QUESTION_PANEL_SCROLL_2026_08_25`](SPEC_ASK_USER_QUESTION_PANEL_SCROLL_2026_08_25.md) | SPEC: Scrolling for the AskUserQuestion panel |
| [`SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10`](SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md) | SPEC: Hover-pause for the AskUserQuestion auto-timeout countdown |
| [`SPEC_BENCHMARK_PORTABLE_DISCOVERY_2026_05_20`](SPEC_BENCHMARK_PORTABLE_DISCOVERY_2026_05_20.md) | SPEC: Benchmark Auth-File Discovery — Dev and Portable Instances |
| [`SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25`](SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md) | SPEC: Block identity bindings from resolving to a provider's ambient home dir |
| [`SPEC_BRIDGE_INIT_RECOVERY_2026_06_15`](SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md) | SPEC: Host-Bridge Init Failure — Self-Heal + Recovery UI |
| [`SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01`](SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md) | Browser pane — camera (getUserMedia video) access |
| [`SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07`](SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md) | SPEC — Browser pane: clicking the body selects the pane (macOS) |
| [`SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26`](SPEC_CEF_PROPRIETARY_CODECS_ALL_PLATFORMS_2026_07_26.md) | Spec: CEF proprietary codec support (H.264/AAC) across Windows/macOS/Linux |
| [`SPEC_CEF_PROPRIETARY_CODECS_MACOS_BUILD_2026_07_27`](SPEC_CEF_PROPRIETARY_CODECS_MACOS_BUILD_2026_07_27.md) | Spec: Execute the macOS leg of issue #2311 (codec-enabled patched CEF) |
| [`SPEC_CI_COMPLETION_NOTIFICATIONS_2026_08_16`](SPEC_CI_COMPLETION_NOTIFICATIONS_2026_08_16.md) | SPEC: jekt notification when a PR's CI run completes (pass or fail) |
| [`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22`](SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md) | Spec: protect a pre-existing project `CLAUDE.md` from AgentMux's overwrite |
| [`SPEC_CODEX_JSONL_CONTRACT_2026_08_08`](SPEC_CODEX_JSONL_CONTRACT_2026_08_08.md) | Codex CLI JSONL Adapter Contract |
| [`SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02`](SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md) | Spec: `compaction_started` Arriving Before Turn-Phase Reconciliation Drops the Ping Permanently |
| [`SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11`](SPEC_COMPOSER_SHIFT_UP_SELECTION_VS_HISTORY_RACE_2026-08-11.md) | Composer: Shift+ArrowUp triggers history recall before the top line is fully selected |
| [`SPEC_COMPOSER_STRIP_CENTERED_SMART_SPLIT_2026_08_14`](SPEC_COMPOSER_STRIP_CENTERED_SMART_SPLIT_2026_08_14.md) | SPEC — Composer strip: stable width + deliberate edge-split tiers |
| [`SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24`](SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md) | SPEC: Composer strip — dynamic left/right slot pooling |
| [`SPEC_COMPOSER_STRIP_LEFT_JUSTIFIED_TIERED_WRAP_2026_08_03`](SPEC_COMPOSER_STRIP_LEFT_JUSTIFIED_TIERED_WRAP_2026_08_03.md) | SPEC — Composer strip: left-justified, tiered wrap (up to 3 levels) |
| [`SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02`](SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02.md) | SPEC — Promote Mode to the composer strip; retire the nested "Controls" panel under Log |
| [`SPEC_COMPOSER_STRIP_TWO_LINE_RESPONSIVE_2026_07_30`](SPEC_COMPOSER_STRIP_TWO_LINE_RESPONSIVE_2026_07_30.md) | SPEC — Composer strip: two-line wrap when the pane narrows |
| [`SPEC_COPY_BUTTON_FALSE_POSITIVE_FIX_2026_08_10`](SPEC_COPY_BUTTON_FALSE_POSITIVE_FIX_2026_08_10.md) | SPEC: Copy Button Silently Failing (Three Stacked Bugs) |
| [`SPEC_DEV_ENV_ISOLATION`](SPEC_DEV_ENV_ISOLATION.md) | Dev-Build Environment Isolation |
| [`SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16`](SPEC_DEV_MODE_LAUNCHER_IPC_2026_05_16.md) | SPEC: Restore Launcher IPC in `task dev` Mode |
| [`SPEC_DEV_WINDOW_TITLE_ARG_2026_06_25`](SPEC_DEV_WINDOW_TITLE_ARG_2026_06_25.md) | Spec: `task dev TITLE="..."` — per-session window title for dev builds |
| [`SPEC_DIVIDER_PILL_RULE_MISALIGNMENT_2026_08_12`](SPEC_DIVIDER_PILL_RULE_MISALIGNMENT_2026_08_12.md) | SPEC: Divider-Pill Rule Misalignment Fix |
| [`SPEC_DYNAMIC_TOOL_SUMMARY_TRUNCATION`](SPEC_DYNAMIC_TOOL_SUMMARY_TRUNCATION.md) | Dynamic ellipsis truncation for tool summaries |
| [`SPEC_FLEET_BROADCAST_CROSS_TIER_TARGETING_2026_08_22`](SPEC_FLEET_BROADCAST_CROSS_TIER_TARGETING_2026_08_22.md) | SPEC: `FleetBroadcast` reaches cross-channel/LAN/WAN targets |
| [`SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22`](SPEC_FLEET_BULK_STOP_CROSS_CHANNEL_2026_08_22.md) | SPEC: `FleetBulkStop` reaches cross-channel targets; LAN/WAN deliberately deferred |
| [`SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29`](SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md) | SPEC: Floating-pane edge-resize (Win32) |
| [`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24`](SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md) | SPEC: Global Memory "system" tier — an AgentMux-controlled, highest-priority entry |
| [`SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17`](SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17.md) | SPEC: External-link routing + single robust "Restore" recovery |
| [`SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16`](SPEC_INJECT_AT_TOOL_BOUNDARY_2026_06_16.md) | SPEC: Deliver a queued message mid-turn (at the next tool-call boundary) instead of waiting for idle |
| [`SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27`](SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md) | Spec: Opt-in isolated auth for `task dev` testing |
| [`SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14`](SPEC_JEKT_REAGENT_TRUST_RELAXATION_2026_08_14.md) | SPEC — Relax TIER=sensitive for cryptographically-verified WAN jekts |
| [`SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16`](SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md) | Spec — Pane Minimize as a Locked State (redesign) |
| [`SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13`](SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13.md) | Spec: Light Theme — Header/Status-Bar Depth Fixes + 3 New Light Themes |
| [`SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13`](SPEC_LINUX_GPU_BACKEND_PRECEDENCE_2026_06_13.md) | SPEC: Linux GPU Backend Precedence (capability-probed ANGLE selection) |
| [`SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24`](SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md) | macOS Tab Redock Parity — Implementation Scoping |
| [`SPEC_MCP_LOOP_TOOL_2026_06_16`](SPEC_MCP_LOOP_TOOL_2026_06_16.md) | SPEC: MCP `Loop` / `LoopStop` tools — recurring prompt injection |
| [`SPEC_MEDIA_PANE_2026_07_26`](SPEC_MEDIA_PANE_2026_07_26.md) | Spec: Media pane — live-updating image/video viewer for agent-generated files |
| [`SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03`](SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md) | Spec: Media pane v4 — agent-facing `OpenMedia` MCP tool |
| [`SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19`](SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md) | Spec: Native Memory Version Control — Single Source of Truth, Two Views (Stash + Armory) |
| [`SPEC_MUXLOG_SWARM_DISPATCH_VERDICT_2026_08_22`](SPEC_MUXLOG_SWARM_DISPATCH_VERDICT_2026_08_22.md) | SPEC: `muxlog swarm -d/--dispatch` — a correlated dispatch-lifecycle verdict |
| [`SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22`](SPEC_MUXSPECT_CROSS_INSTANCE_FIND_2026_08_22.md) | SPEC: `muxspect find` — cross-instance block/agent lookup |
| [`SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06`](SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md) | SPEC — `muxspect dock`: diagnose and clear stuck Activity Dock entries |
| [`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01`](SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md) | SPEC — "muxspect": a live-state introspection tool for running AgentMux instances |
| [`SPEC_MUXSPECT_PHASE_B_POLICY_AND_TIER_ENFORCEMENT_2026_08_22`](SPEC_MUXSPECT_PHASE_B_POLICY_AND_TIER_ENFORCEMENT_2026_08_22.md) | SPEC: `muxspect` Phase B — policy infrastructure + jekt tier enforcement |
| [`SPEC_MUXSPECT_PHASE_C_WAN_TIER_ENFORCEMENT_2026_08_22`](SPEC_MUXSPECT_PHASE_C_WAN_TIER_ENFORCEMENT_2026_08_22.md) | SPEC: `muxspect` Phase C — WAN tier enforcement |
| [`SPEC_MUXSPECT_SRV_VERSION_HEADER_2026_08_22`](SPEC_MUXSPECT_SRV_VERSION_HEADER_2026_08_22.md) | SPEC: `x-agentmux-srv-version` response header |
| [`SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21`](SPEC_MUXSPECT_VERIFY_SENDER_2026_08_21.md) | Spec: `muxspect verify-sender` — fast JEKT-sender liveness lookup |
| [`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07`](SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md) | SPEC: Durable, location-consistent, transparent native memory |
| [`SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10`](SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md) | SPEC: Restore the ghost-text suggestion when the composer is cleared back to empty |
| [`SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27`](SPEC_PANE_CLOSE_REOPEN_CONTINUITY_GUARANTEE_2026_07_27.md) | Spec: pane close/reopen must guarantee conversation continuity, or say so |
| [`SPEC_PANE_HEADER_HEIGHT_TAB_INDICATOR_2026_04_19`](SPEC_PANE_HEADER_HEIGHT_TAB_INDICATOR_2026_04_19.md) | SPEC: Pane Header Height + Tab Active Indicator Edge-to-Edge |
| [`SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20`](SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md) | SPEC: Pane tab strip — editor-style in-pane tabs for agent and terminal panes |
| [`SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12`](SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md) | SPEC: Bind the pane tab strip to its own pane's zoom, and fix top scroll-clearance for short agent conversations |
| [`SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22`](SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md) | SPEC: Pane tab strip — compact (shrink-to-fit) sizing + double-click rename |
| [`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12`](SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md) | SPEC: Frosted-glass backdrop for the agent pane tab strip's trailing space |
| [`SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04`](SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md) | SPEC — Wire failure classification + auto-retry into the persistent controller (Claude 429/overloaded) |
| [`SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09`](SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md) | SPEC — Persistent controller race cluster: gap audit + remaining fixes |
| [`SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30`](SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md) | SPEC — Require real explanation text before declaring a persistent-mode turn done |
| [`SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16`](SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md) | SPEC: Pillar 1 Step 6 — collapse launcher saga durability to an in-memory registry |
| [`SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11`](SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md) | Pillar 2 — Sanitize-Then-Decide: Retiring the Last Two Independent Quit Authorities |
| [`SPEC_PORTABLE_SOURCE_MAPS_2026_06_01`](SPEC_PORTABLE_SOURCE_MAPS_2026_06_01.md) | Source Maps in Portable Builds |
| [`SPEC_PROCESS_BROKER_PHASE_B_SHELL_ACP_REGISTRATION_2026_07_31`](SPEC_PROCESS_BROKER_PHASE_B_SHELL_ACP_REGISTRATION_2026_07_31.md) | SPEC — Process Broker Phase B: register `ShellController`/`AcpController` spawns with `process_tracker::registry` |
| [`SPEC_PR_TITLE_AGENT_HOST_PREFIX_2026_08_22`](SPEC_PR_TITLE_AGENT_HOST_PREFIX_2026_08_22.md) | SPEC: PR title `Agent@host` prefix for shared-identity agents |
| [`SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07`](SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07.md) | Split the low-memory banner into independent RAM and Page File warnings |
| [`SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19`](SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md) | Spec: Make `settings.json` isolated-by-default for every non-`stable` channel |
| [`SPEC_SETTINGS_PANE_COMPLETION_2026_07_14`](SPEC_SETTINGS_PANE_COMPLETION_2026_07_14.md) | SPEC — Settings pane: fill out the remaining sections (completes SPEC_SETTINGS_PANE_2026_06_25) |
| [`SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19`](SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md) | SPEC — Settings: new "Recording / Input" section (mic setup, engine config, test-your-mic) |
| [`SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07`](SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md) | SPEC: Shared filesystem-watcher framework — audit + design |
| [`SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03`](SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md) | SPEC: Shift+drag group resize — move all sibling panes together on one splitter drag |
| [`SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17`](SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md) | SPEC: Shift+drag group resize — fix borders that move opposite the drag direction |
| [`SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03`](SPEC_SRV_HANG_WHILE_ALIVE_DETECTION_2026_08_03.md) | SPEC: srv hang-while-alive detection (#942 family) |
| [`SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22`](SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md) | SPEC: subagent_watcher watches the identity-bound Claude config dir, not a stale spawn-time snapshot |
| [`SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19`](SPEC_SWARM_DISPATCH_ATTRIBUTION_AND_LIFECYCLE_2026_08_19.md) | SPEC: Robust dispatch attribution + formalized session lifecycle for Swarm |
| [`SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19`](SPEC_SWARM_DISPATCH_NAMING_AND_ROW_MODEL_2026_07_19.md) | SPEC: eager per-dispatch naming + two-bucket swarm row model |
| [`SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06`](SPEC_SWARM_ROW_AUTO_LINGER_COUNTDOWN_2026_08_06.md) | SPEC — Swarm Row Auto-Linger Countdown on Completion |
| [`SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24`](SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md) | SPEC: One-click system-toolchain installer (git, Node/npm, and friends) across Windows/macOS/Linux |
| [`SPEC_TAB_COLOR_DESATURATION_2026_08_13`](SPEC_TAB_COLOR_DESATURATION_2026_08_13.md) | Spec: Desaturate tab colors, keep agent pane border colors as-is |
| [`SPEC_TAB_CONTENT_REVEAL_GATE`](SPEC_TAB_CONTENT_REVEAL_GATE.md) | Tab content reveal gate |
| [`SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04`](SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04.md) | Instant tab-bar selection, decoupled from destination-pane reveal cost (window-level tabs) |
| [`SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19`](SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md) | SPEC: Terminal Input Echo-Latency Benchmark |
| [`SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23`](SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md) | SPEC: Terminal scrollback doesn't survive reconnect (all `view:"term"` panes) |
| [`SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30`](SPEC_TERM_DOUBLE_RAF_TEAROUT_2026_05_30.md) | SPEC: Remove the terminal Stage-1 RAF write-coalescer (double-rAF) |
| [`SPEC_THEME_PICKER_AND_MIDNIGHT_AGENT_BG`](SPEC_THEME_PICKER_AND_MIDNIGHT_AGENT_BG.md) | Theme picker in hamburger menu + midnight agent-pane black background |
| [`SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02`](SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02.md) | Plan: consolidate duplicated display-formatting utilities into `frontend/util/` |
| [`SPEC_TOOL_BLOCK_INTERACTION_HOLD_AND_GLOB_EXPAND_2026_06_09`](SPEC_TOOL_BLOCK_INTERACTION_HOLD_AND_GLOB_EXPAND_2026_06_09.md) | SPEC: Tool Block Interaction Hold + Glob Auto-Expand |
| [`SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17`](SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md) | SPEC: Tee redirected tool output to the feed + render tool output as a terminal |
| [`SPEC_TOOL_PREVIEW_DEDENT_2026_08_08`](SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md) | SPEC: Tool preview common-indentation stripping (dedent) |
| [`SPEC_TOOL_PREVIEW_SCROLLBAR_EDGE_PADDING_2026_08_08`](SPEC_TOOL_PREVIEW_SCROLLBAR_EDGE_PADDING_2026_08_08.md) | SPEC: Tool preview scrollbar-to-edge padding removal |
| [`SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03`](SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md) | Spec: hover-to-peek on tool calls and thinking clumps |
| [`SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25`](SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25.md) | Spec: hover-to-peek on EVERY transcript node kind, 50ms delay |
| [`SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04`](SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md) | SPEC: Window-close reliability — fix the `backend_window_id` race |
| [`SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04`](SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md) | SPEC — Chrome-style window snap: drag-to-top maximize, border-drag vertical snap |
| [`SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27`](SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md) | SPEC: Harden the "Working…" indicator and message-list auto-follow against four related recurring bugs |
| [`cef-portable-build`](cef-portable-build.md) | Spec: CEF Portable Build Pipeline |
| [`dev-build-env-isolation`](dev-build-env-isolation.md) | Dev-Build Env Isolation |
| [`frontend-log-pipe`](frontend-log-pipe.md) | Spec: Frontend Log Pipe |
| [`instance-panel-floating-panes`](instance-panel-floating-panes.md) | Spec: Instance Panel — Floating-Pane Focus Fix, Opacity Controls, Condensed Rows |
| [`jekt-visibility-completion`](jekt-visibility-completion.md) | Spec: Jekt Visibility Completion — persistent-agent visibility + outgoing echo |
| [`swarm-active-pane-sync`](swarm-active-pane-sync.md) | Swarm ↔ Pane Two-Way Active-Row Sync |

### active (15)

| Spec | Title |
|---|---|
| [`SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05`](SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md) | SPEC: Align pane scrollback with actual model context, and make cross-instance opens honest |
| [`SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09`](SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md) | SPEC: Session-scoped pane scrollback + a full "Agent History" view |
| [`SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04`](SPEC_AGENT_POLLING_AND_WAKEUP_HARDENING_2026_08_04.md) | Agent Recurring-Task / Polling Primitives — Design Hardening |
| [`SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04`](SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md) | Spec: unify the Working/Worked label with the long-running-process axis, and close the two live desync bugs |
| [`SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02`](SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md) | Spec: an orthogonal "attached task" status axis, sibling to `TurnPhase` |
| [`SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16`](SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md) | SPEC: Browser and Editor Panes |
| [`SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08`](SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md) | Codex Provider Integration: Claude-Parity Lifecycle |
| [`SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31`](SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md) | SPEC: A single content-resize contract for the agent pane |
| [`SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03`](SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md) | Docs Lifecycle Audit & Hardening Plan |
| [`SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03`](SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md) | Plan: MCP-opened markdown blank-preview investigation + Editor-pane reuse |
| [`SPEC_HEADLESS_TRANSIENT_RETRY_2026_08_31`](SPEC_HEADLESS_TRANSIENT_RETRY_2026_08_31.md) | Transient-failure retry for turns with no rendered pane |
| [`SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02`](SPEC_JEKT_CROSS_CHANNEL_TRUST_2026_09_02.md) | SPEC: Cross-channel jekt trust — closing the last unverifiable same-machine tier |
| [`SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03`](SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md) | Migration System Audit & Hardening Plan |
| [`SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29`](SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md) | Spec: multi-tier discovery + remote API invocation over muxbus |
| [`SPEC_WINDOW_NAME_API_HARDENING_2026_08_08`](SPEC_WINDOW_NAME_API_HARDENING_2026_08_08.md) | SPEC: Window-name App API hardening (phantom-id success + status codes) |

### proposed (97)

| Spec | Title |
|---|---|
| [`PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20`](PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md) | Plan — collapse every provider-login code path onto one |
| [`PLAN_MACOS_CLAUDE_KEYCHAIN_CREDENTIAL_ISOLATION_2026_08_17`](PLAN_MACOS_CLAUDE_KEYCHAIN_CREDENTIAL_ISOLATION_2026_08_17.md) | Plan — enforce the same per-agent Claude auth isolation on macOS that already holds on Windows |
| [`PLAN_WINDOWS_CI_SUBPROCESS_IO_FLAKE_FIX_2026_08_13`](PLAN_WINDOWS_CI_SUBPROCESS_IO_FLAKE_FIX_2026_08_13.md) | Plan — fix the recurring `create_no_window_flag_set` flake on Windows nightly CI |
| [`SPEC_ACTIVITY_DOCK_TITLE_WIDTH_AND_TAIL_GLYPH_2026_09_05`](SPEC_ACTIVITY_DOCK_TITLE_WIDTH_AND_TAIL_GLYPH_2026_09_05.md) | SPEC — Activity dock: title over-truncates; tail glyph renders wrong near the time |
| [`SPEC_AGENT_BUSY_ANTS_REFINEMENT_2026_06_22`](SPEC_AGENT_BUSY_ANTS_REFINEMENT_2026_06_22.md) | Agent Busy Bar (Marching Ants) Refinement |
| [`SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17`](SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17.md) | SPEC: two-level dispatch/member schema for subagents and workflows |
| [`SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11`](SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md) | SPEC: Agent History as a pane tab, composer draft preservation, and a scrolling link row |
| [`SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04`](SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md) | SPEC — Tighten the agent-pane login flow: auto-unblock on external bind, "Bind account" button |
| [`SPEC_AGENT_WORKING_ROW_TOOL_BURST_REVEAL_INTERRUPT_2026_08_21`](SPEC_AGENT_WORKING_ROW_TOOL_BURST_REVEAL_INTERRUPT_2026_08_21.md) | SPEC: Tool-call bursts restart the agent-pane "Working…" row's type-out reveal |
| [`SPEC_AGENT_WORKING_ROW_TYPOGRAPHY_REFRESH_2026_09_03`](SPEC_AGENT_WORKING_ROW_TYPOGRAPHY_REFRESH_2026_09_03.md) | SPEC: `AgentWorkingRow` typography refresh — drop the accent-color text, match the thinking-text font, go bold |
| [`SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22`](SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22.md) | Per-agent zoom persistence |
| [`SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01`](SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md) | Spec: Drop the "Claude Code — host CLI config" block from Armory Global Memory |
| [`SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30`](SPEC_ARMORY_MEMORY_TAB_MERGE_2026_08_30.md) | Spec: Armory rail — merge "Global Memory" + "Personal Memory" into one "Memory" tab |
| [`SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01`](SPEC_ARMORY_PERSONAL_MEMORY_AGENT_BLOCKS_2026_09_01.md) | Spec: Armory → Memory → Personal — browse by agent block, not a dropdown |
| [`SPEC_ARMORY_PERSONAL_MEMORY_FILE_TILES_2026_09_04`](SPEC_ARMORY_PERSONAL_MEMORY_FILE_TILES_2026_09_04.md) | Spec: Armory → Memory → Personal — file tiles, not a dropdown |
| [`SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02`](SPEC_ARMORY_PERSONAL_MEMORY_FILTER_AND_SORT_2026_09_02.md) | Spec: find/filter and sort for Armory → Memory → Personal |
| [`SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02`](SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md) | Spec: reactive updates across the Armory |
| [`SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20`](SPEC_ASK_USER_QUESTION_TIMEOUT_KEYBOARD_PAUSE_2026_08_20.md) | SPEC: Keyboard-driven pause for the AskUserQuestion auto-timeout countdown |
| [`SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20`](SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md) | Spec: Intelligent Long-Running-Task Dashboard (Phase C) |
| [`SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20`](SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md) | Spec: Background Task PID Capture (Phase A) |
| [`SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20`](SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md) | Spec: Background Task Teardown Survival (Phase B) |
| [`SPEC_BROWSER_PANE_FAVICON_TITLE_2026-05-15`](SPEC_BROWSER_PANE_FAVICON_TITLE_2026-05-15.md) | Browser Pane: Live Favicon + Page Title in Pane Header |
| [`SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31`](SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md) | Spec: Drop the composer strip's centered token/elapsed stats |
| [`SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26`](SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md) | SPEC: Composer Strip — Row-Based Layout (Rev 7) |
| [`SPEC_DEFAULT_TAB_NAME_TAB_N_2026_09_02`](SPEC_DEFAULT_TAB_NAME_TAB_N_2026_09_02.md) | Spec: default tab names — "Tab N", not "tabN" |
| [`SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25`](SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md) | SPEC: Default fresh-start widgets — Agent, Swarm, Armory, Sysinfo |
| [`SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27`](SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md) | SPEC — A repeatable process for Claude model catalog + CLI version upgrades |
| [`SPEC_DOCS_CLEANUP_AUDIT_2026_08_22`](SPEC_DOCS_CLEANUP_AUDIT_2026_08_22.md) | SPEC — Docs cleanup audit: what's stale, duplicated, or mis-shelved |
| [`SPEC_EARLY_ALPHA_WARNING_2026_06_05`](SPEC_EARLY_ALPHA_WARNING_2026_06_05.md) | SPEC: Early Alpha Warning — README & Microsoft Store Partner Center |
| [`SPEC_EDITOR_MD_PREVIEW_PANEL_2026_06_21`](SPEC_EDITOR_MD_PREVIEW_PANEL_2026_06_21.md) | SPEC — Editor Markdown Live Preview Panel |
| [`SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01`](SPEC_FIX_PERSONAL_MEMORY_EMPTY_WORKDIR_2026_09_01.md) | Spec: Personal Memory is empty for any agent with a blank `working_directory` |
| [`SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22`](SPEC_FLOATING_PANE_DND_RETHINK_2026_06_22.md) | Floating-pane DnD lifecycle — architecture rethink |
| [`SPEC_FLOATING_PANE_TEAROFF_2026_05_11`](SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md) | Floating pane tear-off (subordinate window, owned by mother instance) |
| [`SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26`](SPEC_FLOATING_PANE_TEAROFF_CROSS_PLATFORM_2026-05-26.md) | Floating pane tear-off — cross-platform recipes |
| [`SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24`](SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md) | SPEC — Global Identity, Memory, and Drone Definitions |
| [`SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03`](SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md) | SPEC — In-app (no-shell) Claude OAuth login, revived, at all three auth surfaces |
| [`SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06`](SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md) | Spec: Make isolated auth the default for every non-`stable` channel |
| [`SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31`](SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md) | Spec: Stop the isolated Claude Code config dir from falling back to the host's `~/.claude/CLAUDE.md` |
| [`SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15`](SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md) | SPEC: LAN-tier Ed25519 jekt signing |
| [`SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15`](SPEC_JEKT_SENSITIVE_TIER_NARROWING_2026_08_15.md) | SPEC: Narrow TIER=sensitive to real red flags only |
| [`SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17`](SPEC_JEKT_SENSITIVE_TIER_VERIFIED_SENDER_NO_STOP_2026_08_17.md) | SPEC: TIER=sensitive no longer STOPs work for a cryptographically verified sender |
| [`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13`](SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md) | Spec: Completing the jekt sender-trust layer (host-tier signing + WAN binding enforcement) |
| [`SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25`](SPEC_LINUX_APPIMAGE_PER_BUILD_CHANNEL_2026_06_25.md) | SPEC: Linux AppImage Per-Build Channel Isolation |
| [`SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02`](SPEC_MACOS_LAUNCH_SPEED_AND_SPLASH_TELEMETRY_2026_07_02.md) | SPEC: macOS Launch Speed + Splash Load-Time Telemetry |
| [`SPEC_MEDIA_PANE_V2_AGENT_WORKFLOW_GAPS_2026_07_28`](SPEC_MEDIA_PANE_V2_AGENT_WORKFLOW_GAPS_2026_07_28.md) | Spec: Media pane v2 — gaps found running a real agent video-editing workflow through it |
| [`SPEC_MEDIA_PANE_V3_BROWSER_AND_CUSTOM_TRANSPORT_2026_07_29`](SPEC_MEDIA_PANE_V3_BROWSER_AND_CUSTOM_TRANSPORT_2026_07_29.md) | Spec: Media pane v3 — persistent browser + custom playback/scrub UI |
| [`SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05`](SPEC_MEMORY_CARRYOVER_LOAD_AND_MANAGE_2026_09_05.md) | Memory carry-over: loading and management across the three agent-awareness cases |
| [`SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16`](SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md) | Memory-Pressure Supervision & Graceful Degradation (host / instance level) |
| [`SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02`](SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md) | Spec: fix agent:memory:{list,read_file,write_file,revert} for a blank working_directory |
| [`SPEC_MIGRATION_FRAMEWORK_2026_06_24`](SPEC_MIGRATION_FRAMEWORK_2026_06_24.md) | Migration Framework Spec |
| [`SPEC_MODEL_EFFORT_CAPABILITY_VALIDATION_2026_07_02`](SPEC_MODEL_EFFORT_CAPABILITY_VALIDATION_2026_07_02.md) | SPEC — Per-model effort-capability validation for the composer strip |
| [`SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15`](SPEC_MUXBUS_CLOUD_RELAYED_LOGIN_CALLBACK_2026_08_15.md) | SPEC: MuxBus cloud-relayed login callback (no loopback listener) |
| [`SPEC_MUXSPECT_CROSS_TIER_INSTANCE_INSPECTION_2026_09_02`](SPEC_MUXSPECT_CROSS_TIER_INSTANCE_INSPECTION_2026_09_02.md) | muxspect Phase 2: cross-tier instance inspection (same-host channels + LAN) |
| [`SPEC_PANE_MINIMIZE_AND_TOOLCALL_FAILCOLLAPSE_2026_06_21`](SPEC_PANE_MINIMIZE_AND_TOOLCALL_FAILCOLLAPSE_2026_06_21.md) | SPEC — Pane Minimize Button + Failed Tool Call Immediate Collapse |
| [`SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27`](SPEC_PANE_MINIMIZE_COLUMN_DISSOLVE_2026_06_27.md) | SPEC — Pane Minimize: Column Dissolve on Full-Column Collapse |
| [`SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24`](SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md) | SPEC — Pane Minimize Refinements |
| [`SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11`](SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md) | Auto-discovery pane-overlay clipping (declarative `data-pane-overlay`) |
| [`SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20`](SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md) | Pane Tear-Off — Mother Window Resize |
| [`SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03`](SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md) | SPEC — Peek overlay: track mouse Y while pinned to the right |
| [`SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION`](SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md) | Performance instrumentation + optimization strategy |
| [`SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03`](SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md) | Spec: true per-node token accounting |
| [`SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24`](SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md) | SPEC: Provider-aware startup instructions filename + visibility in Global Memory |
| [`SPEC_PROVIDER_CLI_VERSION_UPGRADE_2026_09_06`](SPEC_PROVIDER_CLI_VERSION_UPGRADE_2026_09_06.md) | Provider CLI version upgrade (2026-09-06 drift report) |
| [`SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24`](SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md) | SPEC: Move the narrow-width responsive tab bar to the top (from the bottom) |
| [`SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13`](SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md) | Spec: Restore-on-relaunch + named, reloadable "Layouts" |
| [`SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04`](SPEC_SHUTDOWN_COUNTDOWN_MODAL_2026_09_04.md) | A formal shutdown sequence: countdown-confirm modal + splash-style progress |
| [`SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30`](SPEC_STATUSBAR_TOKEN_PANEL_BY_AGENT_2026_08_30.md) | Spec: Token stats panel — break out by agent + value-add details |
| [`SPEC_STREAMING_BASH_RUNNER_2026_05_11`](SPEC_STREAMING_BASH_RUNNER_2026_05_11.md) | Streaming bash runner — PreToolUse command rewrite |
| [`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24`](SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md) | SPEC: Surface `~/.claude/CLAUDE.md` (read-only) in Global Memory |
| [`SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31`](SPEC_TAB_WINDOW_RENDER_ARCHITECTURE_2026_08_31.md) | Tab / window render architecture — coherent-frame design |
| [`SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31`](SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31.md) | SPEC — Terminal scroll wheel sensitivity setting |
| [`SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11`](SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md) | Tool block: live log popout + bottom action bar |
| [`SPEC_TOOL_BLOCK_SINGLE_LEFT_BAR_2026_06_27`](SPEC_TOOL_BLOCK_SINGLE_LEFT_BAR_2026_06_27.md) | SPEC: Tool Block Single Left Bar |
| [`SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17`](SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md) | SPEC: Tool-result renderer registry (rich, per-tool result UIs that scale) |
| [`SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01`](SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md) | SPEC: Window Transparency on macOS and Linux |
| [`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04`](SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md) | Spec: optional system-tray + persistent background service, cross-platform |
| [`SPEC_UNIFIED_MENU_SYSTEM_2026_05_11`](SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md) | Unified menu system |
| [`SPEC_WEBSEARCH_CARD_FULL_CONTENT_AND_STYLING_2026_08_13`](SPEC_WEBSEARCH_CARD_FULL_CONTENT_AND_STYLING_2026_08_13.md) | Spec: WebSearch tool-card — full (unclamped) content + styling fixes |
| [`SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20`](SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20.md) | SPEC: Widget bar "More" / pinned-parent flyout closes on its first click when hover already opened it |
| [`SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12`](SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md) | SPEC: Widget bar parent widgets (grouped submenus) |
| [`SPEC_WIDGET_PINBAR_DND_STATE_2026_06_15`](SPEC_WIDGET_PINBAR_DND_STATE_2026_06_15.md) | Spec: Widget Pin-Bar DnD State Machine — Robustness Rethink |
| [`SPEC_XTERM_PASTE_TRUNCATION_2026_06_12`](SPEC_XTERM_PASTE_TRUNCATION_2026_06_12.md) | SPEC: xterm Terminal Paste Truncation Fix |
| [`SPIKE_OPENROUTER_ORI_HARNESS_2026_09_02`](SPIKE_OPENROUTER_ORI_HARNESS_2026_09_02.md) | Spike: OpenRouter's Ori harness — does it change our integration story? |
| [`agent-input-auto-grow`](agent-input-auto-grow.md) | Agent Input Auto-Grow Textarea |
| [`agent-pane-cleanup-plan`](agent-pane-cleanup-plan.md) | Agent Pane Architecture & Cleanup Plan |
| [`agent-pane-runtime-controls`](agent-pane-runtime-controls.md) | Agent Pane Runtime Controls |
| [`agent-pane-slash-commands`](agent-pane-slash-commands.md) | Agent Pane Slash Commands |
| [`agent-pane-title-buttons`](agent-pane-title-buttons.md) | Agent Pane Title Buttons + Git Identity |
| [`app-api-extension`](app-api-extension.md) | App API Extension Spec |
| [`browser-pane-reducer-roadmap`](browser-pane-reducer-roadmap.md) | Browser-pane reducer migration — diagnostic-first roadmap |
| [`network-top-hosts-visibility`](network-top-hosts-visibility.md) | Spec: Per-Host Network Visibility (Top External Hosts + Data Transferred) |
| [`persistent-process-mode`](persistent-process-mode.md) | Persistent Process Mode for Agent Pane |
| [`portable-agent-working-dirs`](portable-agent-working-dirs.md) | Portable Agent Working Dirs |
| [`portable-data-dir`](portable-data-dir.md) | Portable Data Directory |
| [`termwrap-refactor-race-fix`](termwrap-refactor-race-fix.md) | Spec: TermWrap Refactor — Fix Terminal Init Race Condition |
| [`uptime-adaptive-width-help-zoom`](uptime-adaptive-width-help-zoom.md) | Spec: Adaptive Uptime Width + Help View Zoom |
| [`widget-visibility-rearchitecture`](widget-visibility-rearchitecture.md) | Widget Visibility Re-Architecture |

### draft (242)

| Spec | Title |
|---|---|
| [`KIMI_PROVIDER_INTEGRATION_SPEC`](KIMI_PROVIDER_INTEGRATION_SPEC.md) | Kimi Code CLI Provider Integration Spec |
| [`PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29`](PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md) | PLAN: Execute SPEC_INPUT_RESPONSIVENESS — terminal + agent pane |
| [`PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07`](PLAN_SRV_REDUCER_MODULARIZATION_2026-05-07.md) | Plan: srv reducer modularization |
| [`SPEC_ACP_CONTROLLER_2026_04_16`](SPEC_ACP_CONTROLLER_2026_04_16.md) | SPEC: ACP Controller — Universal Agent Client Protocol Support |
| [`SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20`](SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20.md) | SPEC: Move Activity Dock to Bottom of Agent Pane |
| [`SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02`](SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02.md) | SPEC: Add Qwen Code & aider as agent providers |
| [`SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17`](SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md) | Agent API: First-Class Surface (naming, layout, identity, introspection) |
| [`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28`](SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md) | SPEC: Agent App API — MCP is the Agent Entry Point |
| [`SPEC_AGENT_BROWSER_CONTROL_2026_04_17`](SPEC_AGENT_BROWSER_CONTROL_2026_04_17.md) | SPEC: Agent Browser Control |
| [`SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26`](SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md) | SPEC: Slim composer status strip + expandable details panel |
| [`SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23`](SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md) | SPEC: Agent Composer Strip Redesign — Model/Effort Dropdowns + Shell History + Context Text |
| [`SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15`](SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md) | SPEC: Agent Control Protocol — fix AskUserQuestion (+ unblock tool-permission UI) and align muxbus delivery |
| [`SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23`](SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md) | Spec: Agent Pane — Definition Cards + Launch Modal |
| [`SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06`](SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md) | SPEC — Escape delivers a queued message immediately (mimic Claude CLI's interrupt-and-steer) |
| [`SPEC_AGENT_HOST_CONTEXT_2026_04_14`](SPEC_AGENT_HOST_CONTEXT_2026_04_14.md) | Spec: Agent host context — machine binding, agentbus addressing, status bar integration, service attribution |
| [`SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14`](SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md) | Spec: Agent identity restructure — two names, easy rename, external usernames |
| [`SPEC_AGENT_INSTALL_STAGE_2026_05_17`](SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) | SPEC: Agent Install Stage |
| [`SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL_2026_05_21`](SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL_2026_05_21.md) | SPEC — Agent-launch default view + modal-dismissal discipline |
| [`SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22`](SPEC_AGENT_NAMING_AND_ADDRESSING_HOST_LAN_WAN_2026_08_22.md) | SPEC: Agent display naming & addressing across host, LAN, and WAN |
| [`SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18`](SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18.md) | SPEC: Agent Pane Activity Label from Claude CLI OSC Window-Title Sequences |
| [`SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22`](SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md) | Spec: Agent Pane Bottom Action Bar |
| [`SPEC_AGENT_PANE_COLORIZATION_2026_06_09`](SPEC_AGENT_PANE_COLORIZATION_2026_06_09.md) | SPEC: Agent Pane Output Colorization |
| [`SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21`](SPEC_AGENT_PANE_CROSS_CHANNEL_LAN_WAN_SYNC_2026_08_21.md) | SPEC: Cross-channel / LAN / WAN agent pane sync (mirrored panes) |
| [`SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13`](SPEC_AGENT_PANE_FOLLOWUPS_2026_04_13.md) | SPEC — Agent Pane Follow-ups (post-consolidation) |
| [`SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15`](SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md) | SPEC: Agent pane forks + a cohesive auxiliary-pins architecture |
| [`SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23`](SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md) | SPEC: Agent Pane Header Color Theme (Right-Click Picker) |
| [`SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29`](SPEC_AGENT_PANE_HEADER_NAME_PRECEDENCE_2026_06_29.md) | SPEC: Agent Pane Header — Name Precedence + Drop "continued" Chip |
| [`SPEC_AGENT_PANE_HYPERLINKS_2026_06_20`](SPEC_AGENT_PANE_HYPERLINKS_2026_06_20.md) | SPEC: Aggressive Hyperlink Detection in Agent Pane |
| [`SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19`](SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md) | Agent Pane Memory & Identity Modals |
| [`SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25`](SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25.md) | SPEC — Agent pane: remove the reserved-space gap above the tab strip; progress bar overlays instead |
| [`SPEC_AGENT_PANE_STATE_MACHINE_2026_05_07`](SPEC_AGENT_PANE_STATE_MACHINE_2026_05_07.md) | Spec: Agent Pane State Machine Refinement |
| [`SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23`](SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md) | SPEC — agent-pane turn-phase: discriminated union evolution |
| [`SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15`](SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md) | SPEC: Agent-Pane Reducer State Persistence |
| [`SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14`](SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md) | SPEC: Agent Pane — Status Zones Reorganization + Gradient Progress Bar |
| [`SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06`](SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md) | SPEC — agent-pane failure state: fold `useAgentFailure` into the turn-phase reducer |
| [`SPEC_AGENT_PICKER_TEMPLATE_SECTION_CLEANUP_2026_08_22`](SPEC_AGENT_PICKER_TEMPLATE_SECTION_CLEANUP_2026_08_22.md) | Spec: "New Agent" heading + harness-only icons in the template section |
| [`SPEC_AGENT_PICKER_TILE_GRID_2026_06_17`](SPEC_AGENT_PICKER_TILE_GRID_2026_06_17.md) | Agent picker: tile-grid layout |
| [`SPEC_AGENT_STATUS_LABELS_2026_06_27`](SPEC_AGENT_STATUS_LABELS_2026_06_27.md) | Agent Status Labels — Richer Working State UX |
| [`SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04`](SPEC_AGENT_SYSTEM_MANAGEMENT_API_2026_07_04.md) | Agent API: System Management Surface (reload, process/render diagnostics, saga health) |
| [`SPEC_AGENT_TOOL_STORE_2026_04_15`](SPEC_AGENT_TOOL_STORE_2026_04_15.md) | Spec: Agent Tool Store — managed CLI tool availability for agent panes |
| [`SPEC_AGENT_UX_STREAMING_SCROLL_OVERLAY_2026_04_15`](SPEC_AGENT_UX_STREAMING_SCROLL_OVERLAY_2026_04_15.md) | SPEC: Agent Pane — Status Line, Auto-Scroll, and Tool Overlay |
| [`SPEC_AGENT_VERIFICATION_ROUND_2026_04_16`](SPEC_AGENT_VERIFICATION_ROUND_2026_04_16.md) | SPEC: Agent Startup Verification Round |
| [`SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13`](SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md) | Spec: Modularize `frontend/app/view/agent/agent-view.tsx` |
| [`SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08`](SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md) | SPEC: Working-Indicator Shimmer + Mic Button Relocation |
| [`SPEC_ALWAYS_RESPOND_TO_USER_ACTIONS_2026_04_15`](SPEC_ALWAYS_RESPOND_TO_USER_ACTIONS_2026_04_15.md) | SPEC: Always Respond to User Actions |
| [`SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03`](SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md) | SPEC: Ghost-Text Next-Prompt Suggestion — a Second Ambient Model Call Gateway Bind Point |
| [`SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03`](SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md) | SPEC: A Unified Framework for Ambient (Non-User-Driven) Model Calls |
| [`SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08`](SPEC_AMBIENT_SUMMARY_SANITIZATION_AND_TERSENESS_2026_07_08.md) | SPEC: Ambient Haiku-Summary Sanitization + Terseness Pass |
| [`SPEC_APP_API_AGENT_DEFINE_2026_06_06`](SPEC_APP_API_AGENT_DEFINE_2026_06_06.md) | SPEC: App API — `agent.define` (Import / Upsert Agent Definition) |
| [`SPEC_ARMORY_ACCOUNTS_NO_MODALS_2026_07_16`](SPEC_ARMORY_ACCOUNTS_NO_MODALS_2026_07_16.md) | SPEC — Armory Accounts: AgentMux icon (already correct) + remove modals, match single-pane page dynamics |
| [`SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13`](SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md) | SPEC — Armory Phase 5: drop Identities, rename/reorder tabs, seed a starter Skill catalog |
| [`SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15`](SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md) | SPEC — Armory: eliminate split-screen list+detail layouts, single-pane at every width |
| [`SPEC_ARMORY_SHARED_PROVIDER_SETUP_2026_09_05`](SPEC_ARMORY_SHARED_PROVIDER_SETUP_2026_09_05.md) | SPEC: Global Memory is the only concept — remove shared provider config, materialize into the provider's file |
| [`SPEC_BACKEND_LIFECYCLE`](SPEC_BACKEND_LIFECYCLE.md) | Backend Process Lifecycle — Analysis & Fix Spec |
| [`SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22`](SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md) | SPEC — Browser pane: bookmarks (design exploration) + Go-button icon (quick tweak) |
| [`SPEC_BROWSER_PANE_CLICK_DISMISSES_MENUS_2026_08_15`](SPEC_BROWSER_PANE_CLICK_DISMISSES_MENUS_2026_08_15.md) | SPEC — Browser pane: clicking inside it should dismiss open menus/popovers |
| [`SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21`](SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md) | Spec: Browser pane default URL + in-pane popup redirect |
| [`SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18`](SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md) | SPEC: Browser Pane HTTP Basic / Digest Auth |
| [`SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17`](SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md) | SPEC — Browser pane: stop the loading-brain flicker / page-hide flashing |
| [`SPEC_BROWSER_PANE_OPTIMISTIC_HEADER_2026_05_18`](SPEC_BROWSER_PANE_OPTIMISTIC_HEADER_2026_05_18.md) | SPEC: Optimistic Browser-Pane Header on Navigation |
| [`SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15`](SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md) | SPEC — Browser pane: replace Chromium's native right-click menu with the app's own |
| [`SPEC_BROWSER_PANE_Z_ORDER_2026_04_21`](SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md) | Spec: Browser pane Z-order fixes |
| [`SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24`](SPEC_BUILDER_MACOS_LINUX_CI_2026_06_24.md) | SPEC: agentmux-builder — macOS + Linux CI Release Workflows |
| [`SPEC_CEF_LOG_ROBUSTNESS_2026_06_20`](SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md) | SPEC: Harden two CEF-init log errors (cache_path + debug-port bind) |
| [`SPEC_COMPOSER_STRIP_AND_HOST_POLISH_2026_06_25`](SPEC_COMPOSER_STRIP_AND_HOST_POLISH_2026_06_25.md) | Spec: Composer Strip Polish + Context Compaction Indicator + Host Type Coloring |
| [`SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10`](SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10.md) | SPEC: Composer-strip layout fixes, mic vertical centering, curated model defaults |
| [`SPEC_COMPOSER_UX_POLISH_2026_04_15`](SPEC_COMPOSER_UX_POLISH_2026_04_15.md) | Spec: Composer UX Polish — controls above input, Claude-style status line |
| [`SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13`](SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md) | SPEC — Consolidate Forge + Identity into the Agent Pane |
| [`SPEC_CONTAINER_PANE_SUPPORT_2026_06_11`](SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md) | SPEC: Container Pane Support |
| [`SPEC_CONTEXT_VISIBILITY_2026_06_17`](SPEC_CONTEXT_VISIBILITY_2026_06_17.md) | SPEC: Context & Token Visibility — what we can show the user, and how |
| [`SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25`](SPEC_CRON_LOOP_ROBUSTNESS_2026_06_25.md) | Cron & Loop Robustness — Research + Design |
| [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01`](SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) | Cross-Process Dispatch — Launcher → Host Command Pipe |
| [`SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11`](SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md) | Spec: Cross-Window Tab Remount (Drag a Tab onto Another Window's Header) |
| [`SPEC_DECISION_PROMPT_2026_04_24`](SPEC_DECISION_PROMPT_2026_04_24.md) | Spec: Per-Tool-Call Permission Decision Prompt |
| [`SPEC_DECOUPLE_AGENT_CONFIG_FROM_SEED_2026_04_16`](SPEC_DECOUPLE_AGENT_CONFIG_FROM_SEED_2026_04_16.md) | SPEC: Decouple Agent Type and Provider from Seed Manifest |
| [`SPEC_DESIGN_SYSTEM_2026_04_23`](SPEC_DESIGN_SYSTEM_2026_04_23.md) | Spec: Cohesive Design System |
| [`SPEC_DEV_VERSION_BADGE_2026_05_21`](SPEC_DEV_VERSION_BADGE_2026_05_21.md) | SPEC — Build identification in the status bar |
| [`SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11`](SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md) | Spec: Drag-Session Architecture Refactor (Cross-Tab/Cross-Window Drag, Layout Persistence, Block Registry) |
| [`SPEC_EDITOR_AND_APP_FIND_2026_06_17`](SPEC_EDITOR_AND_APP_FIND_2026_06_17.md) | Find: in-editor & app-wide |
| [`SPEC_EDITOR_FILE_ENCODINGS_2026_06_17`](SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md) | Editor file encodings (beyond UTF-8) |
| [`SPEC_EDITOR_FILE_TREE_2026-05-26`](SPEC_EDITOR_FILE_TREE_2026-05-26.md) | Spec: Editor Pane — File Tree Explorer + Extensions |
| [`SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18`](SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md) | Spec: live-reload for editor/preview panes on external file changes |
| [`SPEC_EDITOR_LSP_AND_THEMES_2026-05-26`](SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md) | Spec: Editor Pane — LSP integration + VS Code themes |
| [`SPEC_EDITOR_MARKDOWN_PREVIEW_SCROLLBAR_ALIGNMENT_2026_08_22`](SPEC_EDITOR_MARKDOWN_PREVIEW_SCROLLBAR_ALIGNMENT_2026_08_22.md) | SPEC: Editor pane — align markdown preview's scrollbar with source mode |
| [`SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20`](SPEC_ELIMINATE_BASHWRAP_CONSOLE_WINDOWS_2026_06_20.md) | SPEC: Eliminate Transparent Console Windows on Windows |
| [`SPEC_ERROR_CATALOG_2026_05_17`](SPEC_ERROR_CATALOG_2026_05_17.md) | SPEC: Global Error Code/Message Catalog |
| [`SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27`](SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27.md) | SPEC: Frontend source-map resolver for piped error stacks |
| [`SPEC_FLOATING_PANE_REDOCK_2026-05-27`](SPEC_FLOATING_PANE_REDOCK_2026-05-27.md) | Spec: Floating pane re-dock (with multi-window + drop-target highlighting) |
| [`SPEC_FORGE_AGENT_IDENTITY_2026_04_13`](SPEC_FORGE_AGENT_IDENTITY_2026_04_13.md) | Spec: Forge Agent Identity — GitHub + AWS + Git |
| [`SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20`](SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20.md) | Spec: Forge + Identity + Agent Instances Refinement |
| [`SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13`](SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md) | SPEC — Graceful Crash Handling |
| [`SPEC_GRACEFUL_OOM_EXIT_2026_06_29`](SPEC_GRACEFUL_OOM_EXIT_2026_06_29.md) | Graceful OOM Exit — Own the Death, Explain the Reason |
| [`SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20`](SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md) | SPEC: Host-side CLI login capture is broken for Claude Code v2.1.183 |
| [`SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02`](SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md) | SPEC: Host Reducer Buildout — 5-PR Plan |
| [`SPEC_HOST_REDUCER_PHASE_H_2026-05-02`](SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md) | SPEC: Host Reducer Buildout — Phase H |
| [`SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29`](SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) | SPEC: Input Responsiveness — Terminal and Agent Pane |
| [`SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18`](SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18.md) | SPEC: Install-Modal xterm Theme Binding |
| [`SPEC_INSTALL_MODAL_VERSION_DISPLAY_2026_06_25`](SPEC_INSTALL_MODAL_VERSION_DISPLAY_2026_06_25.md) | SPEC: Show CLI Version in Agent Install Modal |
| [`SPEC_INSTANCE_PANEL_FLOATING_PANES_SECTION_2026_06_24`](SPEC_INSTANCE_PANEL_FLOATING_PANES_SECTION_2026_06_24.md) | Spec: Floating Panes Section in Instance Panel |
| [`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01`](SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md) | Spec: Jekt Security & Visibility |
| [`SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13`](SPEC_LAUNCHER_DEV_INTEGRATION_2026-05-13.md) | SPEC: Integrating `agentmux-launcher` into `task dev` |
| [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01`](SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md) | Launcher Saga Durability + Recovery |
| [`SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19`](SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md) | SPEC: Launch Modal — Integration Tests (jsdom-based) |
| [`SPEC_LAUNCH_MODAL_PLAIN_LANGUAGE_2026_04_24`](SPEC_LAUNCH_MODAL_PLAIN_LANGUAGE_2026_04_24.md) | Spec: Launch Modal Plain-Language Rewrite |
| [`SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18`](SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md) | SPEC: Launch Modal — Profile Section + New Identity/Memory Modals |
| [`SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19`](SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md) | SPEC: Launch Modal — State Machine Hardening |
| [`SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23`](SPEC_LINUX_SANDBOX_APPARMOR_USERNS_2026_08_23.md) | SPEC: Linux Sandbox — Recover From AppArmor's Unprivileged-Userns Restriction |
| [`SPEC_LINUX_SPLASH_POLISH_2026_06_20`](SPEC_LINUX_SPLASH_POLISH_2026_06_20.md) | SPEC: Linux splash polish (fade-out, multi-monitor centering, rounded corners) |
| [`SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20`](SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md) | SPEC: Session-aware Linux startup splash (X11 + Wayland) |
| [`SPEC_LIVE_LOG_PTY_REWORK_2026_05_16`](SPEC_LIVE_LOG_PTY_REWORK_2026_05_16.md) | SPEC: Live-Log PTY Rework |
| [`SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25`](SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md) | Spec: Local Build Channel Pruner |
| [`SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15`](SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md) | SPEC: Pinned Activity Dock — Unified Long-Running Activities |
| [`SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03`](SPEC_MACOS_ACCESSIBILITY_ROBUSTNESS_2026-06-03.md) | SPEC: macOS Accessibility Robustness — surviving external AX clients without crashing |
| [`SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03`](SPEC_MACOS_NATIVE_MENU_BAR_2026-06-03.md) | SPEC: Native macOS menu bar (File / Edit / View / Window / Help) |
| [`SPEC_MACOS_WINDOW_CLOSE_LIFECYCLE_2026-06-04`](SPEC_MACOS_WINDOW_CLOSE_LIFECYCLE_2026-06-04.md) | SPEC: macOS window-close lifecycle — "closes but stays open hidden" |
| [`SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21`](SPEC_MAGNIFY_ZOOM_IMPLEMENTATION_2026-05-21.md) | SPEC: Magnify & Zoom — Implementation Plan |
| [`SPEC_MAGNIFY_ZOOM_REGRESSION_AND_DEFAULTS_2026-05-21`](SPEC_MAGNIFY_ZOOM_REGRESSION_AND_DEFAULTS_2026-05-21.md) | SPEC: Magnify zoom regression + magnified-pane defaults |
| [`SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21`](SPEC_MAXIMIZE_ZOOM_ARCHITECTURE_2026-05-21.md) | SPEC: Maximize & Zoom — Architecture Analysis |
| [`SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20`](SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20.md) | SPEC: Menu positioning framework — offset every menu into the paintable area |
| [`SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24`](SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md) | Spec: Messaging App Integrations — Unified Plan |
| [`SPEC_MESSAGING_INTEGRATION_DISCORD_POC_2026_06_24`](SPEC_MESSAGING_INTEGRATION_DISCORD_POC_2026_06_24.md) | Spec: Messaging App Integration — Discord POC |
| [`SPEC_MESSAGING_INTEGRATION_SLACK_2026_07_07`](SPEC_MESSAGING_INTEGRATION_SLACK_2026_07_07.md) | Spec: Messaging App Integration — Slack |
| [`SPEC_MESSAGING_INTEGRATION_TEAMS_2026_07_07`](SPEC_MESSAGING_INTEGRATION_TEAMS_2026_07_07.md) | Spec: Messaging App Integration — Microsoft Teams |
| [`SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07`](SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07.md) | Spec: Messaging App Integration — Telegram |
| [`SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07`](SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md) | Spec: Messaging App Integration — WhatsApp |
| [`SPEC_MODAL_PAINT_GATE_2026_05_18`](SPEC_MODAL_PAINT_GATE_2026_05_18.md) | SPEC: Modal Paint Gate |
| [`SPEC_MODAL_TRANSITIONS_2026_05_18`](SPEC_MODAL_TRANSITIONS_2026_05_18.md) | SPEC: Modal Transitions & Chained-Flow Crossfades |
| [`SPEC_MSSTORE_AUTOMATED_RELEASE_2026_06_29`](SPEC_MSSTORE_AUTOMATED_RELEASE_2026_06_29.md) | SPEC: Automated Microsoft Store Release (msstore CLI + GitHub Actions) |
| [`SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15`](SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md) | SPEC: Multi-Agent Version Coordination |
| [`SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03`](SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md) | SPEC: Multi-Instance Isolation Hardening & Crash-Safety Verification |
| [`SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06`](SPEC_MULTI_SESSION_AGENT_FORK_2026_06_06.md) | SPEC: Multi-Session Agent Fork |
| [`SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16`](SPEC_MUXBUS_AGENT_DISCOVERY_AND_PERSISTENT_DELIVERY_2026_06_16.md) | MuxBus — Persistent-Agent Delivery & Unified Agent Discovery |
| [`SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04`](SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md) | Plan: muxbus cross-channel duplicate delivery |
| [`SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15`](SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md) | MuxBus Delivery Hierarchy |
| [`SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06`](SPEC_MUXBUS_MULTI_TENANT_SECURITY_2026_07_06.md) | Plan: MuxBus multi-tenant security — current state and path to production isolation |
| [`SPEC_NATIVE_BROWSER_PANE_2026_04_17`](SPEC_NATIVE_BROWSER_PANE_2026_04_17.md) | SPEC: Native Browser Pane via CefBrowserView |
| [`SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23`](SPEC_NIGHTLY_RELEASE_CHANNEL_2026_08_23.md) | Nightly Release Automation — Auto-Publish the Latest Pending Version Bump |
| [`SPEC_OBJ_UPDATE_BRIDGE_2026-05-14`](SPEC_OBJ_UPDATE_BRIDGE_2026-05-14.md) | SPEC: Internal-event → frontend WaveObjUpdate bridge |
| [`SPEC_OPENCLAW_AGENT_2026_05_17`](SPEC_OPENCLAW_AGENT_2026_05_17.md) | SPEC: OpenClaw integration — shared interfaces, distinct flavor |
| [`SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23`](SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md) | SPEC: OS-level activity indicator when an agent is busy |
| [`SPEC_PANE_DRAG_TO_TAB_2026_07_10`](SPEC_PANE_DRAG_TO_TAB_2026_07_10.md) | Spec: Pane Drag-to-Tab (Cross-Tab Pane Relocation via Drag & Drop) |
| [`SPEC_PANE_FILE_DROP_2026_05_30`](SPEC_PANE_FILE_DROP_2026_05_30.md) | SPEC: Drag-and-drop files into Terminal and Agent panes |
| [`SPEC_PANE_ICON_AND_TEXT_VISIBILITY_2026_05_30`](SPEC_PANE_ICON_AND_TEXT_VISIBILITY_2026_05_30.md) | SPEC: Pane Icon and Text Visibility Pass |
| [`SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05`](SPEC_PANE_RESIZE_AND_FLOATER_DRAG_NATIVE_LOOP_2026_06_05.md) | Spec (v2) — Floating-pane drag — host-side manual loop |
| [`SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26`](SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md) | SPEC: Pane resize dimension overlay (WxH badge) |
| [`SPEC_PERSISTENT_SHELL_NODE_2026_06_11`](SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md) | SPEC: Persistent Shell Node in the Agent Pane |
| [`SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14`](SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14.md) | SPEC: Persistent Shell Node — Phase 3 (Stop / Lifecycle) |
| [`SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01`](SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md) | SPEC: Phase E.4 — Layout reducer migration |
| [`SPEC_PHASE_E_SRV_REDUCER_2026_04_29`](SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md) | SPEC: Phase E — srv reducer + saga coordinator (first multi-reducer validation) |
| [`SPEC_PHASE_F_HOST_REDUCER_2026-05-01`](SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md) | SPEC: Phase F — host reducer (third reducer in the multi-reducer architecture) |
| [`SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14`](SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md) | Spec: Pre-launch OAuth flow — identity-first agent setup |
| [`SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18`](SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md) | SPEC: Provider System-Tool Prerequisites |
| [`SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14`](SPEC_REACTIVE_WORKSPACE_SYNC_2026-05-14.md) | SPEC: Reactive Workspace + Object Sync (frontend reactivity gap) |
| [`SPEC_REMOVE_NODE_HOVER_STRIP_2026_06_15`](SPEC_REMOVE_NODE_HOVER_STRIP_2026_06_15.md) | SPEC: Remove the per-row hover strip (`NodeHoverStrip`) |
| [`SPEC_RENAME_WSH_TO_RPC_2026_04_17`](SPEC_RENAME_WSH_TO_RPC_2026_04_17.md) | SPEC: Rename WSH Files to RPC |
| [`SPEC_RETRO_FOLLOWUPS_2026_04_12`](SPEC_RETRO_FOLLOWUPS_2026_04_12.md) | Spec: Retro Follow-ups — 2026-04-12 |
| [`SPEC_ROBUST_MODAL_SYSTEM_2026_04_23`](SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md) | Spec: Robust Modal System |
| [`SPEC_RUNTIME_IDENTITY_AUTOBIND_2026_05_15`](SPEC_RUNTIME_IDENTITY_AUTOBIND_2026_05_15.md) | SPEC: Runtime Identity Auto-Bind & Named-Agent Display Fix |
| [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01`](SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) | Saga + Reducer + IPC Architecture — Target End State |
| [`SPEC_SAGA_DURABILITY_2026-05-01`](SPEC_SAGA_DURABILITY_2026-05-01.md) | SPEC: Saga durability — durable saga log |
| [`SPEC_SCHEMA_FLATTENING_2026_05_19`](SPEC_SCHEMA_FLATTENING_2026_05_19.md) | SPEC: objects.db Schema Flattening + De-Forge Rename |
| [`SPEC_SECTIONED_PANE_DYNAMIC_TITLE_2026_08_12`](SPEC_SECTIONED_PANE_DYNAMIC_TITLE_2026_08_12.md) | SPEC: Dynamic pane titles for rail/section panes (Armory, Warden, Settings) |
| [`SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20`](SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md) | SPEC — Service Supervision & Recovery |
| [`SPEC_SESSION_DIGEST_AS_PANE_ACCESSORY_2026_06_15`](SPEC_SESSION_DIGEST_AS_PANE_ACCESSORY_2026_06_15.md) | SPEC: Session digest as a Pane Accessory |
| [`SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19`](SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md) | SPEC — Settings pane audit: ranked candidates for new sections/controls |
| [`SPEC_SETTINGS_MESSAGING_BRIDGES_SECTION_2026_08_22`](SPEC_SETTINGS_MESSAGING_BRIDGES_SECTION_2026_08_22.md) | SPEC — Settings: new "Integrations" section (Discord / Telegram / Slack / WhatsApp bridges) |
| [`SPEC_SETTINGS_PANE_2026_06_25`](SPEC_SETTINGS_PANE_2026_06_25.md) | Spec: Settings → Widget Pane with UI Form |
| [`SPEC_SETTINGS_WIDGET`](SPEC_SETTINGS_WIDGET.md) | Spec: Settings Widget |
| [`SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14`](SPEC_SLASH_COMMAND_ARCHITECTURE_2026_04_14.md) | SPEC — Slash Command Architecture |
| [`SPEC_SLASH_TERMINAL_COMMAND_2026_06_25`](SPEC_SLASH_TERMINAL_COMMAND_2026_06_25.md) | SPEC: `/terminal` Slash Command — Open Agent CWD in New Terminal Pane |
| [`SPEC_SOUND_NOTIFICATIONS_2026_06_05`](SPEC_SOUND_NOTIFICATIONS_2026_06_05.md) | SPEC — Sound notifications subsystem |
| [`SPEC_SPLASH_SCREEN_BORDER_2026_08_25`](SPEC_SPLASH_SCREEN_BORDER_2026_08_25.md) | SPEC — Splash screen: add a darkened 2px border, across all 3 platforms |
| [`SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25`](SPEC_SPLASH_STARTUP_TELEMETRY_2026_06_25.md) | Spec: Splash Screen Startup Telemetry |
| [`SPEC_SPLASH_USERINFO_AND_DISABLE_2026_06_21`](SPEC_SPLASH_USERINFO_AND_DISABLE_2026_06_21.md) | SPEC: Splash user-info footer + "disable splash" setting |
| [`SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24`](SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md) | SPEC: Anchor the startup-injection summary at the cursor during hover-expand |
| [`SPEC_STATUSBAR_CPU_CORES_PANEL_2026_06_15`](SPEC_STATUSBAR_CPU_CORES_PANEL_2026_06_15.md) | SPEC: Status-bar CPU → per-core panel |
| [`SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24`](SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md) | Spec: Status-Bar Token Usage Indicator + Per-Service Breakdown |
| [`SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17`](SPEC_STATUS_BAR_POPOVER_AIRSPACE_CLIP_2026_08_17.md) | SPEC — Status bar popovers: fix native browser-pane airspace occlusion |
| [`SPEC_STATUS_BAR_WINDOW_COUNT_2026_05_16`](SPEC_STATUS_BAR_WINDOW_COUNT_2026_05_16.md) | SPEC: Status-Bar Window-Count Display |
| [`SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12`](SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md) | SPEC — subagent lifecycle: no reducer, no liveness, "working" forever |
| [`SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10`](SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10.md) | SPEC: Submenu positioning flash + hover-intent (safe-triangle) timing |
| [`SPEC_SUBPROCESS_INTEGRATION_TESTS_2026_04_16`](SPEC_SUBPROCESS_INTEGRATION_TESTS_2026_04_16.md) | SPEC: Subprocess Integration Tests |
| [`SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05`](SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md) | SPEC: Swarm Live Feed — backend code bindings |
| [`SPEC_SWARM_LIVE_FEED_UI_2026_07_05`](SPEC_SWARM_LIVE_FEED_UI_2026_07_05.md) | SPEC: Swarm Live Feed — UI |
| [`SPEC_SWARM_TREE_REDESIGN_2026_06_19`](SPEC_SWARM_TREE_REDESIGN_2026_06_19.md) | Swarm View Tree Redesign |
| [`SPEC_TABLE_FORMATTER_2026_05_07`](SPEC_TABLE_FORMATTER_2026_05_07.md) | Spec: Refined Table Formatter for Presentation Layer |
| [`SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25`](SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md) | Spec: Tab Bar — Designed From First Principles |
| [`SPEC_TAB_CONTENT_AWARE_SIZING_2026-06-14`](SPEC_TAB_CONTENT_AWARE_SIZING_2026-06-14.md) | Spec: Content-Aware Tab Sizing (VS Code Model) |
| [`SPEC_TAB_CONTENT_FOLDER_SURFACE_2026_06_03`](SPEC_TAB_CONTENT_FOLDER_SURFACE_2026_06_03.md) | SPEC — Workspace tabs as a continuous surface with their content ("folder" model) |
| [`SPEC_TAB_GAPS_AND_NAMING_2026_04_25`](SPEC_TAB_GAPS_AND_NAMING_2026_04_25.md) | Spec: Constant Tab Gaps + Plain-Language Default Names |
| [`SPEC_TERMINAL_FLOW_CONTROL_2026_05_30`](SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md) | SPEC: Terminal flow control (PTY backpressure) |
| [`SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31`](SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md) | SPEC: Terminal Predictive Local Echo |
| [`SPEC_TERM_SCROLLBAR_ZERO_GAP_2026_06_10`](SPEC_TERM_SCROLLBAR_ZERO_GAP_2026_06_10.md) | SPEC: Zero-Width Terminal Gap at All Zoom Levels |
| [`SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16`](SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md) | SPEC: Tool Auto-Expand Panel (Replace Portal Overlay) |
| [`SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13`](SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md) | Spec: Tool Hover Overlay + Scroll-on-Type |
| [`SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14`](SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md) | Spec: Syntax highlighting inside the tool hover overlay |
| [`SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10`](SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md) | SPEC: Unified Agent Conversation-History Store |
| [`SPEC_UNIFIED_CLIPBOARD_2026_05_18`](SPEC_UNIFIED_CLIPBOARD_2026_05_18.md) | SPEC: Unified Copy/Paste + Export |
| [`SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21`](SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md) | SPEC — Unified modal system (scope-based) |
| [`SPEC_UNIFIED_RELEASE_CICD_2026_06_29`](SPEC_UNIFIED_RELEASE_CICD_2026_06_29.md) | Unified Release CI/CD — agentmux + MS Store + Landing Page |
| [`SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24`](SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md) | SPEC: User input visibility + startup-injection collapse |
| [`SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30`](SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md) | SPEC: v1 — MCP Servers & Skills as first-class primitives |
| [`SPEC_VERSION_INSTANCE_PANEL_2026_04_25`](SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md) | Spec: Version-Click Instance Panel |
| [`SPEC_VOICE_INPUT_PER_PANE_2026_05_19`](SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md) | SPEC: Voice input — per-pane, header button near pane controls, Terminal + Agent |
| [`SPEC_WARDEN_WIDGET_2026-05-25`](SPEC_WARDEN_WIDGET_2026-05-25.md) | Spec: Warden Widget |
| [`SPEC_WAVE_TS_CLEANUP_2026_04_17`](SPEC_WAVE_TS_CLEANUP_2026_04_17.md) | SPEC: wave.ts Cleanup and Modularization |
| [`SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_2026_06_24`](SPEC_WIDGET_CONTEXT_MENU_OPEN_ACTIONS_2026_06_24.md) | SPEC: Widget Context Menu — "Open in New Window" + "Open in Floating Pane" |
| [`SPEC_WIDGET_ICON_COLORS_2026-05-26`](SPEC_WIDGET_ICON_COLORS_2026-05-26.md) | SPEC: Top-bar widget icons — theme-driven, monochrome by default |
| [`SPEC_WIDGET_LABEL_CASING_2026-05-27`](SPEC_WIDGET_LABEL_CASING_2026-05-27.md) | Widget & Pane Label Casing — Title-Case the User-Visible Names |
| [`SPEC_WIDGET_OPEN_IN_NEW_WINDOW_2026_04_17`](SPEC_WIDGET_OPEN_IN_NEW_WINDOW_2026_04_17.md) | SPEC: Widget "Open in New Window" Context Menu |
| [`SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03`](SPEC_WINDOWS_CEF_BUNDLE_VERSION_INTEGRITY_2026_06_03.md) | SPEC: Windows CEF Bundle Version Integrity & Loud Startup Failure |
| [`SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13`](SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md) | SPEC: Robust DPI Handling for Window-Header Drag |
| [`SPEC_WINDOW_DRAG_HANDLE_2026_06_06`](SPEC_WINDOW_DRAG_HANDLE_2026_06_06.md) | SPEC: Always-visible window drag handle (grip) in the tab bar |
| [`SPEC_WINDOW_INSTANCE_NAMING_CLEANUP_2026-05-14`](SPEC_WINDOW_INSTANCE_NAMING_CLEANUP_2026-05-14.md) | SPEC: Window/Instance Naming Cleanup |
| [`SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27`](SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md) | SPEC: AgentMux Window & Process State Machine |
| [`SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23`](SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md) | SPEC: Restore keyboard focus to the active pane on window re-activation (Windows) |
| [`SPEC_WINDOW_TITLE_FORMAT_2026-05-13`](SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md) | SPEC: Window Title Format — `Window - Tab - AgentMux` |
| [`SPEC_WINDOW_TRANSPARENCY`](SPEC_WINDOW_TRANSPARENCY.md) | Spec: Full Window Transparency |
| [`SPEC_WORKSPACE_TAB_SIZING_2026-05-27`](SPEC_WORKSPACE_TAB_SIZING_2026-05-27.md) | Spec: Workspace Tab Sizing — Editor-Tab Parity |
| [`SPEC_WRITE_STATE_NDJSON_RESTORE_2026_06_12`](SPEC_WRITE_STATE_NDJSON_RESTORE_2026_06_12.md) | SPEC: write_state Option 4 — NDJSON-Reconstructed Snapshot (Schema v2) |
| [`agent-pane-document-reducer-2026-05-03`](agent-pane-document-reducer-2026-05-03.md) | Agent Pane Document Reducer — root-cause + architecture spec |
| [`codex-gemini-cli-integration`](codex-gemini-cli-integration.md) | Spec: Codex CLI & Gemini CLI — Install, Auth, and Launch Integration |
| [`command-palette`](command-palette.md) | Command Palette — Spec |
| [`frontend-reducer-conventions-2026-05-03`](frontend-reducer-conventions-2026-05-03.md) | Frontend Reducer Conventions |
| [`frontend-reducer-implementation-plan-2026-05-03`](frontend-reducer-implementation-plan-2026-05-03.md) | Frontend Reducer Implementation Plan |
| [`instance-indicator`](instance-indicator.md) | Spec: Window Instance Indicator |
| [`jekt-auto-registration`](jekt-auto-registration.md) | Jekt Auto-Registration via AGENTMUX_AGENT_ID |
| [`lan-awareness-and-embedded-jekt-api`](lan-awareness-and-embedded-jekt-api.md) | Analysis: LAN Instance Awareness & Embedded Jekt API |
| [`lan-discovery-toggle`](lan-discovery-toggle.md) | Spec: LAN Discovery Toggle (HostPopover) |
| [`linux-appimage-cold-launch-tax-2026-05-08`](linux-appimage-cold-launch-tax-2026-05-08.md) | Linux AppImage cold-launch tax |
| [`openclaw-agent-runtime`](openclaw-agent-runtime.md) | OpenClaw Agent Runtime Integration |
| [`per-pane-process-tree-metrics`](per-pane-process-tree-metrics.md) | Spec: Per-Pane Process Tree CPU + Memory Metrics |
| [`provider-auth-isolation`](provider-auth-isolation.md) | Spec: Provider Auth Isolation per AgentMux Version |
| [`rename-db-and-default-layout`](rename-db-and-default-layout.md) | Spec: Rename wave.db → objects.db + Default 2-column launch layout |
| [`settings-cleanup`](settings-cleanup.md) | Spec: Settings Cleanup (Dead Code + Drift) |
| [`settings-jsonc-live-reload`](settings-jsonc-live-reload.md) | Spec: JSONC Settings with Live Reload |
| [`settings-modal`](settings-modal.md) | Spec: Settings Modal UI |
| [`statusbar-hostname`](statusbar-hostname.md) | Spec: Hostname Display in Status Bar |
| [`tearoff-pane-size`](tearoff-pane-size.md) | Spec: Pane Tear-Off Window Sizing |
| [`versioned-process-names`](versioned-process-names.md) | Versioned Process Names Spec |
| [`web-widget`](web-widget.md) | Web Widget Implementation Spec - Tauri v2 |
| [`widget-dnd-reorder`](widget-dnd-reorder.md) | Widget Drag-and-Drop Reorder |

### living (2)

| Spec | Title |
|---|---|
| [`SPEC_DECISION_PROMPT_DESIGN_2026_04_25`](SPEC_DECISION_PROMPT_DESIGN_2026_04_25.md) | Decision Prompt — Cohesive Design (Step-Back Doc) |
| [`SPEC_POOL_COVERAGE_AND_ROADMAP_2026_06_20`](SPEC_POOL_COVERAGE_AND_ROADMAP_2026_06_20.md) | Pre-warmed Window Pool — Coverage Map and Implementation Roadmap |

### historical (1)

| Spec | Title |
|---|---|
| [`SPEC_ACTIVE_TAB_COLOR_LINE_STOP_AT_TAB_STRIP_2026_07_13`](SPEC_ACTIVE_TAB_COLOR_LINE_STOP_AT_TAB_STRIP_2026_07_13.md) | SPEC — active-tab color line: stop at the tab strip's right edge, not the viewport edge |

### superseded (3)

| Spec | Title |
|---|---|
| [`SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09`](SPEC_ACCOUNT_ADOPTION_PIGGYBACK_LOGIN_2026_08_09.md) | SPEC — Account adoption: piggyback an unlinked agent onto an existing login |
| [`SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06`](SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md) | Spec: Continuous AgentWorkingRow background through the scrollbar gutter |
| [`SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24`](SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24.md) | SPEC: Composer strip — balance misc elements across left/right zones |

### no status line (150)

Predate the closed vocabulary. Not a backlog to bulk-restamp —
an unverified restamp turns "unknown" into "confidently wrong".
Fix one when you touch it and know its real state.

| Spec | Title |
|---|---|
| [`ANALYSIS_ARTIFACT_NAMING_2026_04_14`](ANALYSIS_ARTIFACT_NAMING_2026_04_14.md) | Artifact Naming Analysis — Remove "cef" from Release Artifact Names |
| [`ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06`](ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md) | Drift-storm renderer crash (post-PR #706 smoke) — architectural analysis |
| [`ANALYSIS_MULTI_PROCESS_BEST_PRACTICES_2026_04_27`](ANALYSIS_MULTI_PROCESS_BEST_PRACTICES_2026_04_27.md) | ANALYSIS: Multi-Process Desktop App State Management — Best Practices |
| [`ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27`](ANALYSIS_WINDOW_PROCESS_STATE_INVENTORY_2026_04_27.md) | ANALYSIS: AgentMux Window/Process State Inventory |
| [`BUG_MACOS26_DUAL_DOCK_ICON_2026_06_20`](BUG_MACOS26_DUAL_DOCK_ICON_2026_06_20.md) | BUG: Dual Dock Icon on macOS 26 Tahoe |
| [`CLEANUP_LEGACY_REMNANTS`](CLEANUP_LEGACY_REMNANTS.md) | Cleanup Spec: Remove Legacy Remnants |
| [`MASTER_REDUCER_STACK_STATUS_2026-05-05`](MASTER_REDUCER_STACK_STATUS_2026-05-05.md) | Master Reducer-Stack Status — 2026-05-05 |
| [`OAUTH_FLOW_SMOKE_DIAGNOSTIC_2026_05_14`](OAUTH_FLOW_SMOKE_DIAGNOSTIC_2026_05_14.md) | OAuth Pre-Launch Smoke-Test Diagnostic — 2026-05-14 |
| [`OAUTH_RESUME_AFTER_REBOOT_2026_05_14`](OAUTH_RESUME_AFTER_REBOOT_2026_05_14.md) | OAuth pre-launch — resume notes after PC reboot (2026-05-14) |
| [`PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30`](PHASE_E_SAGAS_EXECUTION_PLAN_2026-04-30.md) | Phase E.5 Sagas — Execution Plan |
| [`PLAN_BROWSER_DOM_API`](PLAN_BROWSER_DOM_API.md) | PLAN: Browser-pane DOM API implementation |
| [`PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22`](PLAN_SPLASH_TELEMETRY_OPEN_ITEMS_2026_07_22.md) | PLAN: Splash telemetry — consolidated open-items tracker |
| [`REPORT_AGENT_PANE_ARCHITECTURE_MODULARIZATION_ANALYSIS_2026_07_31`](REPORT_AGENT_PANE_ARCHITECTURE_MODULARIZATION_ANALYSIS_2026_07_31.md) | REPORT — Agent Pane Architecture: Full Inventory & Modularization Analysis |
| [`REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20`](REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md) | Report: Haiku ambient-summary ghost text populates outside genuine turn completion |
| [`REPORT_AUTH_ARCHITECTURE_2026_06_25`](REPORT_AUTH_ARCHITECTURE_2026_06_25.md) | Auth Architecture Report — AgentMux |
| [`REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21`](REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md) | AgentMux Auth Architecture — Current State & Rethink |
| [`REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07`](REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md) | REPORT — context-menu gap audit: Swarm copy, agent-pane paste, and beyond |
| [`REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14`](REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md) | Report: "New Window" startup color-flash sequence |
| [`REPORT_NPM_CI_EMNAPI_LOCKFILE_RETRO_2026_08_07`](REPORT_NPM_CI_EMNAPI_LOCKFILE_RETRO_2026_08_07.md) | REPORT — retro: the `npm ci` / `@emnapi` lockfile EUSAGE failure |
| [`REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22`](REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md) | AgentMux Process Architecture — Current State & Rethink |
| [`REPORT_SWARM_SUBAGENT_INTERRUPTED_STATUS_2026_07_20`](REPORT_SWARM_SUBAGENT_INTERRUPTED_STATUS_2026_07_20.md) | REPORT — why every subagent under a long-running pane shows "Interrupted" |
| [`RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07`](RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md) | Tab tear-off — research report on cross-platform best practices |
| [`SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01`](SAGA_ARCHITECTURE_EXECUTION_PLAN_2026-05-01.md) | Saga Architecture — Execution Plan |
| [`SPEC-explicit-runtime-summary`](SPEC-explicit-runtime-summary.md) | SPEC: Explicit Runtime Summary in Agent Control Bar |
| [`SPEC_AGENT_BUSY_ANIMATION_2026_06_21`](SPEC_AGENT_BUSY_ANIMATION_2026_06_21.md) | Agent Busy Animation — Aurora Bar |
| [`SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20`](SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md) | SPEC: Agent Error Framework — Durable Error State + Global Error Surface |
| [`SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11`](SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md) | SPEC: Agent Failure Diagnostics — Surfacing the "Why" Behind a Non-Zero Exit |
| [`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16`](SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md) | SPEC: Agent Failure Recovery UI (per-error-class actions) |
| [`SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN`](SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md) | SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md |
| [`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18`](SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md) | SPEC: Agent UI Automation (Click + Screenshot) as a First-Class Agent App API Capability |
| [`SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13`](SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md) | Spec: Default Seed Catalog for the Armory's MCP Servers Tab |
| [`SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12`](SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md) | Spec: Completing Armory Phase 4 — the Storage Rename |
| [`SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10`](SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md) | Spec: Preloaded MCP Server Connectors for Creative Apps (Ableton Live, TouchDesigner, and Others) |
| [`SPEC_BROWSER_DOM_API`](SPEC_BROWSER_DOM_API.md) | SPEC: Browser-pane DOM API (`/agentmux/browser/*`) |
| [`SPEC_BROWSER_PANE_FOCUS_LOCK`](SPEC_BROWSER_PANE_FOCUS_LOCK.md) | SPEC: Browser Pane Focus Lock |
| [`SPEC_BROWSER_PANE_LIFECYCLE`](SPEC_BROWSER_PANE_LIFECYCLE.md) | SPEC: Browser Pane Lifecycle & State Machine |
| [`SPEC_BROWSER_PANE_LIFECYCLE_TESTS`](SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md) | SPEC: Browser Pane Lifecycle — Automated Test Coverage |
| [`SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11`](SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md) | Spec: Loading-Brain Indicator for Browser Panes (Messenger Widgets) |
| [`SPEC_BROWSER_PANE_MODULARIZATION`](SPEC_BROWSER_PANE_MODULARIZATION.md) | SPEC: Browser Pane Code Modularization |
| [`SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03`](SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md) | SPEC: Windows browser-pane renderer teardown — Phase-0 spike scope |
| [`SPEC_BULLETPROOF_TERMINALS_2026_05_21`](SPEC_BULLETPROOF_TERMINALS_2026_05_21.md) | SPEC_BULLETPROOF_TERMINALS_2026_05_21.md |
| [`SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17`](SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md) | SPEC: Bundle-as-container v2 (GH issue #2024, item 3) |
| [`SPEC_CEF_WINDOWS_PR_FINALIZATION_2026_07_27`](SPEC_CEF_WINDOWS_PR_FINALIZATION_2026_07_27.md) | Spec: Finalize Media pane + CEF Windows codec PRs, confirm CI pulls the real build |
| [`SPEC_CI_TEST_RUNNER_2026_06_22`](SPEC_CI_TEST_RUNNER_2026_06_22.md) | SPEC — CI test runner on public GitHub-hosted runners |
| [`SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30`](SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30.md) | SPEC: Color Palette Expansion & Reuse |
| [`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31`](SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md) | Spec: Detecting and Handling Context Compaction in Agent Panes |
| [`SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13`](SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md) | SPEC — Cross-Channel Agent Persistence |
| [`SPEC_DATA_CHANNELS_2026_05_24`](SPEC_DATA_CHANNELS_2026_05_24.md) | SPEC: Data channels — version-spanning data isolation for AgentMux |
| [`SPEC_DEFAULT_AGENT_NAME`](SPEC_DEFAULT_AGENT_NAME.md) | SPEC_DEFAULT_AGENT_NAME.md |
| [`SPEC_DEV_BADGE_RUNTIME_MODE_2026_06_12`](SPEC_DEV_BADGE_RUNTIME_MODE_2026_06_12.md) | SPEC: "DEV" Badge Shows on Every Build — Runtime-Mode Self-Identification |
| [`SPEC_DEV_ENV_ISOLATION.fix-plan`](SPEC_DEV_ENV_ISOLATION.fix-plan.md) | Fix Plan: Dev Environment Isolation — macOS LaunchServices Gap |
| [`SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05`](SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md) | SPEC — Drone Canvas: Expansive Node-Graph Editor |
| [`SPEC_DRONE_INLINE_NODE_PARAMS_2026_06_05`](SPEC_DRONE_INLINE_NODE_PARAMS_2026_06_05.md) | SPEC — Drone: Inline In-Node Parameter Editing |
| [`SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12`](SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md) | SPEC: Editor File-Tree "Open" Actions (Open to the Side / Open in New Tab) |
| [`SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14`](SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md) | SPEC: Editor Widget Default UX — Scratch File + Collapsed Tree |
| [`SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14`](SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md) | SPEC: File Tree Right-Click Context Menu |
| [`SPEC_FLOATER_DRAG_FIX_PLAN_2026_06_05`](SPEC_FLOATER_DRAG_FIX_PLAN_2026_06_05.md) | Fix Plan — Floater Drag Remaining Issues |
| [`SPEC_FLOATING_PANE_MULTI_MONITOR_TASKBAR_2026_07_27`](SPEC_FLOATING_PANE_MULTI_MONITOR_TASKBAR_2026_07_27.md) | Spec: taskbar/Dock presence for floating panes dragged to another monitor |
| [`SPEC_HARD_CORNERS_2026_05_26`](SPEC_HARD_CORNERS_2026_05_26.md) | SPEC: Hard corners on buttons and modals |
| [`SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10`](SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC_2026_07_10.md) | Spec: Identity direct-links, PR-C (Armory read-only view + bundle-free agent creation) |
| [`SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21`](SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md) | SPEC — Instance Lifecycle Consolidation |
| [`SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13`](SPEC_JEKT_LAN_WAN_TRUST_HARDENING_2026_08_13.md) | Spec: Securing LAN and WAN tier jekt delivery — closing cross-tenant and cross-network trust gaps |
| [`SPEC_LARGE_TIER_MODULARIZATION_INDEX_2026_07_02`](SPEC_LARGE_TIER_MODULARIZATION_INDEX_2026_07_02.md) | Large-Tier Modularization — Index & Sequencing |
| [`SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25`](SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md) | SPEC: Move launch modal from tab-scope to pane-scope lock |
| [`SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN`](SPEC_LAYOUT_HEAL_ROOTNODE_ORPHAN.md) | SPEC: Layout Healer Misses Rootnode-Is-Orphan Case |
| [`SPEC_LINUX_TEAROFF_HEADER_ONLY_2026-06-20`](SPEC_LINUX_TEAROFF_HEADER_ONLY_2026-06-20.md) | Fix Spec: Linux pane tear-off restricted to header only |
| [`SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18`](SPEC_MACOS_LAUNCH_COHERENCE_2026_06_18.md) | SPEC: macOS Launch Coherence — Per-Version Bundle ID, Reopen Handler, Unix Window Forward |
| [`SPEC_MACOS_REOPEN_NEW_WINDOW_2026_06_22`](SPEC_MACOS_REOPEN_NEW_WINDOW_2026_06_22.md) | SPEC: macOS reopen → open a new window (kill "AgentMux is not responding") |
| [`SPEC_MEMORY_ANALYSIS_2026_06_26`](SPEC_MEMORY_ANALYSIS_2026_06_26.md) | Memory Analysis — AgentMux Long-Running Stability |
| [`SPEC_MODAL_COMPACT_VARIANT_2026_05_25`](SPEC_MODAL_COMPACT_VARIANT_2026_05_25.md) | SPEC: Compact modal variant — auto-trigger for narrow lock regions |
| [`SPEC_MODULARIZE_AGENT_SESSION_2026_07_02`](SPEC_MODULARIZE_AGENT_SESSION_2026_07_02.md) | Spec: Modularize `agent_session.rs` |
| [`SPEC_MODULARIZE_CEF_CLIENT_2026_07_02`](SPEC_MODULARIZE_CEF_CLIENT_2026_07_02.md) | Spec: Modularize `agentmux-cef/src/client/mod.rs` |
| [`SPEC_MODULARIZE_LAUNCHER_MAIN_2026_07_02`](SPEC_MODULARIZE_LAUNCHER_MAIN_2026_07_02.md) | Spec: Modularize `agentmux-launcher/src/main.rs` |
| [`SPEC_MODULARIZE_SHELL_2026_07_02`](SPEC_MODULARIZE_SHELL_2026_07_02.md) | Spec: Modularize `blockcontroller/shell.rs` |
| [`SPEC_MSIX_PACKAGING_2026_05_30`](SPEC_MSIX_PACKAGING_2026_05_30.md) | SPEC: MSIX Packaging for the Microsoft Store |
| [`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20`](SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md) | SPEC: Multi-Agent Fleet Control — Select, Broadcast, and Bulk-Act on Many Agents at Once |
| [`SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23`](SPEC_NIGHTLY_CROSS_PLATFORM_BUILDS_2026_06_23.md) | SPEC — Nightly cross-platform CI builds |
| [`SPEC_PANE_FOCUS_STRESS_TEST`](SPEC_PANE_FOCUS_STRESS_TEST.md) | SPEC: Pane Focus Stress Test |
| [`SPEC_PANE_REFLOW_ANIMATION_2026_05_29`](SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md) | SPEC: Coordinated Pane Reflow Animation (DOM + native browser panes) |
| [`SPEC_PARK_AND_BLANK_CLOSE_2026_07_09`](SPEC_PARK_AND_BLANK_CLOSE_2026_07_09.md) | SPEC — Park-and-blank for non-demotable window closes (renderer-zombie commit leak) |
| [`SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02`](SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md) | Spec: Preset → Bundle internal refactor (Composable Agent Model, Phases 2–4) |
| [`SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14`](SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md) | SPEC: Per-Provider Models + Generalized Reasoning/Effort |
| [`SPEC_README_AUDIT_2026_06_25`](SPEC_README_AUDIT_2026_06_25.md) | README Audit — 2026-06-25 |
| [`SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27`](SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md) | Spec: schedule the deferred redock/floating-pane structural fixes (P3, P6) |
| [`SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22`](SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md) | SPEC — Reducer single-source-of-truth (SSOT) consolidation |
| [`SPEC_REMOVE_BOOKMARKS_2026_06_11`](SPEC_REMOVE_BOOKMARKS_2026_06_11.md) | SPEC: Remove Bookmark Feature — 2026-06-11 |
| [`SPEC_RENAME_DRAG_REGION_ATTR`](SPEC_RENAME_DRAG_REGION_ATTR.md) | SPEC: Rename `data-tauri-drag-region` → `data-drag-region` |
| [`SPEC_TAB_COLORS`](SPEC_TAB_COLORS.md) | Spec: Tab Color System |
| [`SPEC_TEST_API_ACCESS`](SPEC_TEST_API_ACCESS.md) | SPEC: Test-Harness Access to the App API (+ /wave → /agentmux rename) |
| [`SPEC_TOOLCHAIN_MANAGER_2026-06-15`](SPEC_TOOLCHAIN_MANAGER_2026-06-15.md) | SPEC: Toolchain Manager + GUI-launch PATH enrichment |
| [`SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28`](SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md) | SPEC: Tool-hover consolidation (2026-05-28) |
| [`SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22`](SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22.md) | SPEC_TOOL_LOG_INPLACE_ANIMATION_2026_06_22 |
| [`SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27`](SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27.md) | SPEC_TOOL_LOG_UNIVERSAL_ANIMATION_COLLAPSE_2026_07_27 |
| [`SPEC_TOOL_OUTPUT_CAP_2026_05_30`](SPEC_TOOL_OUTPUT_CAP_2026_05_30.md) | SPEC: Tool-Output Render Cap — interim DOM-bloat mitigation |
| [`SPEC_TOPBAR_PROGRESSIVE_COLLAPSE_2026_06_05`](SPEC_TOPBAR_PROGRESSIVE_COLLAPSE_2026_06_05.md) | SPEC — Top Bar Progressive Collapse (3-tier widget + tab responsive system) |
| [`SPEC_WAVE_TO_MUX_RENAME_2026-05-14`](SPEC_WAVE_TO_MUX_RENAME_2026-05-14.md) | SPEC: `Wave*` → `Mux*` rename (purge Wave Terminal branding) |
| [`SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22`](SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md) | SPEC — Stale window count "(N)" after closing a Views window |
| [`SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29`](SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29.md) | Window drag — host-side manual native move loop (Windows) |
| [`SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29`](SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29.md) | Window drag → OS native move loop (Windows) |
| [`SPEC_WINDOW_RENAME_2026_04_27`](SPEC_WINDOW_RENAME_2026_04_27.md) | Window Rename + Click Behaviors in InstancePanel |
| [`SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08`](SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md) | SPEC — WRR quit gate fires on a live window (false exit on non-last window close) |
| [`STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27`](STATUS_CEF_PROPRIETARY_CODECS_MACOS_2026_07_27.md) | Status: macOS codec-enabled patched CEF rebuild (issue #2311) |
| [`VSCODE_MENUBAR_REFERENCE`](VSCODE_MENUBAR_REFERENCE.md) | VS Code Menu Bar — Reference for AgentMux |
| [`agent-6-buttons-plan`](agent-6-buttons-plan.md) | Plan: Agent Pane — 6 Provider Buttons (Raw + Styled) |
| [`agent-health-codebase-audit`](agent-health-codebase-audit.md) | Agent Health/Liveness Detection — Codebase Audit |
| [`agent-widget-forge-integration`](agent-widget-forge-integration.md) | Spec: Agent Widget — Forge Integration |
| [`app-api-pane-open`](app-api-pane-open.md) | App API — `pane.open` Spec |
| [`app-api-status`](app-api-status.md) | App API — Implementation Status |
| [`app-update-check`](app-update-check.md) | App Update Check |
| [`backend-status-tests`](backend-status-tests.md) | Test Spec: Backend Status Atom (`backendStatusAtom`) |
| [`cef-white-flash-testbed`](cef-white-flash-testbed.md) | CEF White Flash Testbed — Spec |
| [`chrome-zoom`](chrome-zoom.md) | Chrome Zoom: Status Bar + Title Bar |
| [`cli-install-spec`](cli-install-spec.md) | CLI Install Spec — Per-Version Isolated Installs |
| [`computer-use-pane`](computer-use-pane.md) | computer-use-pane.md |
| [`dead-code-removal`](dead-code-removal.md) | Spec: Dead Code Removal & Treeshake |
| [`dead-code-strip`](dead-code-strip.md) | Dead Code Strip — AgentMux Pre-SolidJS Cleanup |
| [`dnd-debug-logging`](dnd-debug-logging.md) | Drag-and-Drop Debug Logging Spec |
| [`double-click-maximize`](double-click-maximize.md) | Double-Click Window Header to Maximize/Restore |
| [`forge-responsive`](forge-responsive.md) | Spec: Responsive Forge Pane |
| [`forge-widget`](forge-widget.md) | Spec: Forge Widget |
| [`integration-vision`](integration-vision.md) | AgentMux Integration Vision |
| [`jekt-inject-timing`](jekt-inject-timing.md) | Jekt Inject Timing Spec |
| [`openclaw-widget`](openclaw-widget.md) | OpenClaw Widget Spec |
| [`pane-highlight-fix-plan`](pane-highlight-fix-plan.md) | Pane Highlight Border — Fix Implementation Plan |
| [`pane-popout-to-new-window`](pane-popout-to-new-window.md) | Spec: Pane Pop-Out to New Window |
| [`per-pane-zoom-hover`](per-pane-zoom-hover.md) | Per-Pane Zoom: Hover-Aware Scroll Wheel |
| [`portable-build-spec`](portable-build-spec.md) | AgentMux Windows Portable Build — Spec |
| [`pr179-cross-platform-verification`](pr179-cross-platform-verification.md) | PR #179 Cross-Platform Verification Report |
| [`process-lifecycle-v2`](process-lifecycle-v2.md) | Process Lifecycle v2: OS-Level Parent-Child Binding |
| [`process-state-tracker`](process-state-tracker.md) | Process State Tracker — Spec |
| [`readme-rewrite`](readme-rewrite.md) | Spec: README.md Rewrite |
| [`replace-pane-widget`](replace-pane-widget.md) | Replace Pane Context Menu |
| [`responsive-agent-pane`](responsive-agent-pane.md) | Spec: Responsive Agent Pane |
| [`runtime-logging`](runtime-logging.md) | Spec: Runtime Logging Infrastructure Rewrite |
| [`secondary-windows-impl-plan`](secondary-windows-impl-plan.md) | Secondary Windows CEF Views — Implementation Plan |
| [`settings-json-template-sync`](settings-json-template-sync.md) | Spec: Settings.json Template Sync |
| [`settings-open-in-editor`](settings-open-in-editor.md) | Spec: Open Settings in Code Editor |
| [`swarm-analysis`](swarm-analysis.md) | Swarm Observability — Subagent Watcher Analysis |
| [`swarm-orchestration`](swarm-orchestration.md) | swarm-orchestration.md |
| [`sysinfo-history-length`](sysinfo-history-length.md) | Sysinfo CPU History Length Setting |
| [`sysinfo-scrollbar-investigation`](sysinfo-scrollbar-investigation.md) | Sysinfo Scrollbar Regression — Investigation |
| [`tab-styling`](tab-styling.md) | Spec: Tab Styling Improvements |
| [`themes-spec`](themes-spec.md) | Spec: Theme System |
| [`tool-collapse`](tool-collapse.md) | SPEC: Tool Call Collapsed-by-Default with Hover Expand |
| [`websocket-modularization`](websocket-modularization.md) | Spec: websocket.rs Modularization |
| [`widget-bar-opacity-submenu`](widget-bar-opacity-submenu.md) | Spec: Widget Bar Context Menu — Opacity Submenu |
| [`widget-pinning`](widget-pinning.md) | Spec: Widget Bar Pinning & "More" Dropdown |
| [`ws-robustness-impl`](ws-robustness-impl.md) | WebSocket Robustness — Implementation Plan |
| [`xterm-v6-size-opacity-fix`](xterm-v6-size-opacity-fix.md) | xterm v6: Terminal Size & Opacity Regression Fix |
| [`zoom-architecture`](zoom-architecture.md) | AgentMux Zoom System Architecture |

### non-canonical status (210)

These carry a `**Status:**` line whose first word is not in the closed enum
(`docs/specs/README.md`). Grouped by the word actually found, so the
real state is visible rather than guessed at. `check-doc-status.sh`
requires a fix the next time one of these is edited; as with the
section above, do not bulk-restamp them.

**`addendum`**

| Spec | Title |
|---|---|
| [`SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29`](SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md) | Win10 Commit-Limit OOM — Addendum: Free-Disk Regression & the 0xE0000008 Crash Class |

**`all`**

| Spec | Title |
|---|---|
| [`SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06`](SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md) | Spec: `replaceChild` crash in the agent-pane virtualizer — full analysis and fix plan |
| [`identity-implementation-plan`](identity-implementation-plan.md) | Identity Pane — Implementation Plan (Phase 1) |

**`analysis`**

| Spec | Title |
|---|---|
| [`REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20`](REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md) | Report — Armory Ctrl+Wheel Zoom (missing) + Browser Pane Zoom (not per-instance) |
| [`SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24`](SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24.md) | SPEC — Long-Running Process UX: Working Stuck + Red X |
| [`SPEC_PERSISTENCE_LAYER_ANALYSIS_2026-05-14`](SPEC_PERSISTENCE_LAYER_ANALYSIS_2026-05-14.md) | SPEC: Persistence layer analysis — keep SQLite, or move? |
| [`SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26`](SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md) | SPEC: Resize refinements — flip group/direct defaults, and Shift+window-resize feeding only the edge panes |
| [`SPEC_TOOL_PREVIEW_REFINEMENTS_2026_06_26`](SPEC_TOOL_PREVIEW_REFINEMENTS_2026_06_26.md) | SPEC — Tool Preview Refinements: Word-wrap + Independent Zoom |
| [`browser-pane-state-catalog`](browser-pane-state-catalog.md) | Browser pane state catalog |
| [`cef-drag-window-management`](cef-drag-window-management.md) | Spec: CEF Drag, Drop, and Window Management |
| [`cef-transparency-architecture`](cef-transparency-architecture.md) | Spec: CEF Transparency Architecture |
| [`interactive-maximize`](interactive-maximize.md) | Spec: Interactive Maximize (Pane Magnify Overhaul) |
| [`service-update-consolidation`](service-update-consolidation.md) | Analysis: Consolidate Object Update Return Paths |
| [`sysinfo-continuous-monitor-animation-2026-05-03`](sysinfo-continuous-monitor-animation-2026-05-03.md) | Sysinfo Plot — Continuous-Monitor Animation |

**`approved`**

| Spec | Title |
|---|---|
| [`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14`](SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md) | SPEC — honest account-delete semantics: spawn gating, agent reconciliation, Armory truthfulness |
| [`SPEC_AGENT_RUNTIME_DROPUP_2026_07_09`](SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md) | SPEC: Consolidate Mode / Model / Effort into a single Runtime dropup |
| [`SPEC_CEF_SANDBOX_2026_06_20`](SPEC_CEF_SANDBOX_2026_06_20.md) | SPEC: Enable CEF Renderer Sandbox |
| [`SPEC_CEF_SANDBOX_WIN_PHASE3_2026_06_20`](SPEC_CEF_SANDBOX_WIN_PHASE3_2026_06_20.md) | SPEC: CEF Windows Renderer Sandbox — Phase 3 |
| [`SPEC_IDENTITY_STORE_SPLIT_2026_08_17`](SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md) | SPEC: Split the multi-concern shared store — permanent global identity data vs. explicitly-disposable Armory test accounts |
| [`SPEC_PROVIDER_ISOLATION_2026_06_20`](SPEC_PROVIDER_ISOLATION_2026_06_20.md) | SPEC: Provider environment isolation — never touch the user's `~/.claude` or global CLI |
| [`SPEC_RELEASE_CICD_CORRECTION_2026_06_30`](SPEC_RELEASE_CICD_CORRECTION_2026_06_30.md) | Release CI/CD Correction — remove the `dl.agentmux.ai` fabrication |
| [`SPEC_TAB_UI_REFINEMENTS_2026_06_20`](SPEC_TAB_UI_REFINEMENTS_2026_06_20.md) | SPEC: Tab UI Refinements |
| [`SPEC_TITLEBAR_CONTEXTMENU_REWORK_2026_06_19`](SPEC_TITLEBAR_CONTEXTMENU_REWORK_2026_06_19.md) | Spec: Title Bar Context Menu Rework & Reusable PopoverMenu |
| [`agentmux-local-url-injection`](agentmux-local-url-injection.md) | AGENTMUX_LOCAL_URL Pane Injection |
| [`xterm-v6-upgrade-spec`](xterm-v6-upgrade-spec.md) | xterm.js v6.0.0 Upgrade Spec |

**`architecture`**

| Spec | Title |
|---|---|
| [`frontend-reducer-architecture-2026-05-03`](frontend-reducer-architecture-2026-05-03.md) | Frontend Reducer Architecture — spec roadmap |

**`assessment`**

| Spec | Title |
|---|---|
| [`SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29`](SPEC_ARCHITECTURE_HEALTH_AND_REFACTOR_2026_06_29.md) | Architecture Health Assessment & Refactor Proposal |

**`attempted`**

| Spec | Title |
|---|---|
| [`SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28`](SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md) | Spec: replace HTML5/OLE drag with a native pointer-capture drag loop for tab + pane tear-off (Windows) |

**`audit`**

| Spec | Title |
|---|---|
| [`REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30`](REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md) | Report: why the message-list scrollbar still visibly drifts up before snapping to bottom, and how to make it pin unconditionally |
| [`modal-cleanup-migration-2026-05-01`](modal-cleanup-migration-2026-05-01.md) | Modal Cleanup — Migration Audit & Plan |

**`both`**

| Spec | Title |
|---|---|
| [`SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25`](SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) | SPEC: Agent-pane status cleanup — remove "unresponsive" detection, consolidate Reconnecting/Compacting/Working |

**`bug`**

| Spec | Title |
|---|---|
| [`cef-isolation-audit`](cef-isolation-audit.md) | CEF Version Isolation Audit |

**`conclusions`**

| Spec | Title |
|---|---|
| [`SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30`](SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md) | Pillar 1 — Host Reproject: Open-Question Resolutions & Design Foundation |

**`confirmed`**

| Spec | Title |
|---|---|
| [`SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22`](SPEC_JEKT_TRANSCRIPT_REQUEST_TIER_RULES_2026_08_22.md) | SPEC: `transcript_request` jekt tier rules — repo-owner confirmation |

**`consolidation`**

| Spec | Title |
|---|---|
| [`SPEC_TAB_WINDOW_DRAG_CONSOLIDATION_2026_07_13`](SPEC_TAB_WINDOW_DRAG_CONSOLIDATION_2026_07_13.md) | Spec: Tab / Window / Pane Drag — Consolidated Status & Roadmap |

**`decided`**

| Spec | Title |
|---|---|
| [`REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26`](REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md) | Bashwrap, the Dock, and the Process Broker — a Seventh Mechanism Nobody Wired Up |

**`decision`**

| Spec | Title |
|---|---|
| [`SPEC_FLOATING_PANE_REDOCK_PHASE_4A_SCOPING_2026-05-27`](SPEC_FLOATING_PANE_REDOCK_PHASE_4A_SCOPING_2026-05-27.md) | Phase 4a Re-dock — MVP scope decision |
| [`SPEC_MULTIWINDOW_TASKBAR_GROUPING`](SPEC_MULTIWINDOW_TASKBAR_GROUPING.md) | SPEC: Multi-Window Taskbar Behaviour — Full Instances + Sub-Windows |

**`decisions`**

| Spec | Title |
|---|---|
| [`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14`](ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md) | Architecture rethink: making ABF mandatory ("every agent must have an ABF") |

**`design`**

| Spec | Title |
|---|---|
| [`SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24`](SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md) | SPEC: Agent concept consolidation — DRY rethink |
| [`SPEC_AGENT_GENERIC_PANE_OPEN_TOOL_2026_08_21`](SPEC_AGENT_GENERIC_PANE_OPEN_TOOL_2026_08_21.md) | Spec: `OpenPane` — a general-purpose, agent-facing "open any pane" MCP tool |
| [`SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02`](SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md) | Agent-Pane Layout State Machine — unify zoom + virtualization + tool-expansion into one reducer |
| [`SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09`](SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09.md) | SPEC: Responsive Aux Info + Color System for Agent Pane Tool Blocks |
| [`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27`](SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md) | SPEC: Agent pane tab-switch perf |
| [`SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17`](SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md) | SPEC: Filter/search box atop the AgentPicker |
| [`SPEC_AGENT_PICKER_TWO_TIER_2026_05_24`](SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md) | SPEC: Two-tier agent picker — "My Agents" + "Templates" |
| [`SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14`](SPEC_LAUNCH_AUTH_STATE_MACHINE_2026_05_14.md) | Pre-launch auth — complete user stories + state machine |
| [`SPEC_ORPHAN_THINKING_NODES_2026_05_27`](SPEC_ORPHAN_THINKING_NODES_2026_05_27.md) | SPEC: Orphan in-progress nodes — cancel + collapse on session reopen |
| [`SPEC_STORE_MODULARIZATION_2026_05_27`](SPEC_STORE_MODULARIZATION_2026_05_27.md) | SPEC: `wstore` → `store` rename + modularization |
| [`SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29`](SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md) | SPEC: "Working…" State — Liveness Model (rethink, not rewrite) |
| [`container-agent-runtime`](container-agent-runtime.md) | Spec: Container Agent Runtime |
| [`gpu-and-extended-system-metrics`](gpu-and-extended-system-metrics.md) | Spec: GPU Monitoring & Extended System Metrics |

**`diagnosis`**

| Spec | Title |
|---|---|
| [`SPEC_WINDOW_OPACITY_GPU_2026_05_21`](SPEC_WINDOW_OPACITY_GPU_2026_05_21.md) | SPEC — Window transparency: one cross-platform problem (GPU-composited Chromium) |

**`final`**

| Spec | Title |
|---|---|
| [`SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16`](SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md) | SPEC: Globally Portable Agents — Final Implementation |

**`fix`**

| Spec | Title |
|---|---|
| [`SPEC_TOOL_BLOCK_COLLAPSED_OVERLAY_LAYOUT_2026_06_02`](SPEC_TOOL_BLOCK_COLLAPSED_OVERLAY_LAYOUT_2026_06_02.md) | Collapsed Tool Overlays Are Laid Out While Hidden → Slow Zoom/Scroll |

**`fixes`**

| Spec | Title |
|---|---|
| [`REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27`](REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md) | Report: "Error: not logged in" after a successful login, and two stuck-"Working…" states |

**`formal`**

| Spec | Title |
|---|---|
| [`srv-phase-e4b-formal-spec-2026-05-03`](srv-phase-e4b-formal-spec-2026-05-03.md) | SPEC: srv Phase E.4.B — Layout Tree as Reducer State |

**`implementation`**

| Spec | Title |
|---|---|
| [`SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29`](SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md) | B.7.3 — launcher events to renderer via CEF JS bridge |
| [`SPEC_CONTEXT_COMPACTION_NOTIFICATION_2026_06_20`](SPEC_CONTEXT_COMPACTION_NOTIFICATION_2026_06_20.md) | SPEC: Context Compaction Notification |
| [`SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29`](SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md) | Pillar 2 — Wire `reconcile_quit` as the Single Lifecycle Authority |
| [`SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28`](SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md) | SPEC: window_hwnds cache stale-HWND fix |
| [`nodejs-detection-notification`](nodejs-detection-notification.md) | Spec: Node.js Detection & User Notification |

**`implementing`**

| Spec | Title |
|---|---|
| [`SPEC_MCP_SETNAME_TARGET_ID_2026_06_19`](SPEC_MCP_SETNAME_TARGET_ID_2026_06_19.md) | SPEC: `SetName` explicit target-id parameter |
| [`SPEC_PROVIDER_PINNED_AUTH_2026_06_05`](SPEC_PROVIDER_PINNED_AUTH_2026_06_05.md) | Spec: Provider-Pinned, Instance-Independent Auth |

**`in`**

| Spec | Title |
|---|---|
| [`PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16`](PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md) | Implementation Plan: Scroll-Driven Tool-Block Collapse |
| [`SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16`](SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md) | SPEC: OpenEditor — collapsed file-tree + floating-pane support |
| [`SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20`](SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20.md) | SPEC: Re-authentication from Agent Auth Failure |
| [`SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07`](SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md) | Tab tear-off — position match + Chrome-style paint |
| [`SPEC_VOICE_STT_ENGINE_2026_06_20`](SPEC_VOICE_STT_ENGINE_2026_06_20.md) | SPEC: Voice STT engine — capture-and-send to Whisper |

**`investigation`**

| Spec | Title |
|---|---|
| [`REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04`](REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md) | Report: agent pane blank-load period + brain-logo loading indicator |
| [`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07`](REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md) | Report: agent/swarm pane loading, ambient-call flood, and stale status |
| [`SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31`](SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md) | SPEC — Process & Turn-State Tracking: This Session's Findings, and the Case for a Unified State Machine |
| [`agent-pane-icon-debug`](agent-pane-icon-debug.md) | Agent Pane Icon Buttons — Debug Log |

**`no`**

| Spec | Title |
|---|---|
| [`console-flash-report`](console-flash-report.md) | Console Window Flash Report |

**`npm`**

| Spec | Title |
|---|---|
| [`cli-install-research`](cli-install-research.md) | CLI Installation Research (March 2026) |

**`open`**

| Spec | Title |
|---|---|
| [`env-tilde-expansion-bug`](env-tilde-expansion-bug.md) | Bug: Tilde (`~`) Not Expanded in `cmd:env` Values |

**`p`**

| Spec | Title |
|---|---|
| [`SPEC_VERSION_ISOLATION_2026_06_01`](SPEC_VERSION_ISOLATION_2026_06_01.md) | Version Isolation — Spec & Fix Plan |
| [`SPEC_WINDOWS_LIFECYCLE_ROBUSTNESS_2026_06_26`](SPEC_WINDOWS_LIFECYCLE_ROBUSTNESS_2026_06_26.md) | Windows Lifecycle Robustness — Surviving External Termination |

**`part`**

| Spec | Title |
|---|---|
| [`SPEC_CLAUDE_CLI_PIN_CONTRACT_TESTS_2026_07_14`](SPEC_CLAUDE_CLI_PIN_CONTRACT_TESTS_2026_07_14.md) | SPEC — CLI pin consolidation + contract tests against the pinned Claude CLI |

**`partially`**

| Spec | Title |
|---|---|
| [`SPEC_MUXBUS_FREE_ACCOUNT_ABUSE_HARDENING_2026_08_17`](SPEC_MUXBUS_FREE_ACCOUNT_ABUSE_HARDENING_2026_08_17.md) | SPEC: muxbus free-account abuse hardening — closing the sign-up/messaging backdoors |

**`phase`**

| Spec | Title |
|---|---|
| [`SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24`](SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md) | SPEC: Robust cross-instance window discovery, capture, and naming |
| [`SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01`](SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01.md) | Agent-Pane Virtualization Overlap Under Zoom |
| [`SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30`](SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md) | SPEC: Safe unrestricted screen capture for agents |
| [`SPEC_ASK_USER_QUESTION_2026_06_15`](SPEC_ASK_USER_QUESTION_2026_06_15.md) | SPEC: AskUserQuestion — interactive agent questions in the agent pane |
| [`SPEC_GATED_RENDERER_RECOVERY_2026_06_01`](SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md) | Gated Renderer Recovery — Memory-Aware Crash Handling |
| [`SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11`](SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md) | SPEC: Launcher-side teardown backstop (UI-thread liveness probe + armed J0 teardown) |
| [`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21`](SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md) | Spec: Cross-tier conversation visibility for `muxspect` (host / cross-channel / LAN / WAN) |
| [`SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20`](SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md) | SPEC — live subagent reconciliation + Retire action (best-practices plan) |
| [`SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16`](SPEC_TERMINAL_INPUT_PRIORITY_OVER_SYSINFO_2026_06_16.md) | SPEC: Terminal I/O Has Complete Priority Over Perf Monitoring |
| [`SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03`](SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md) | Spec: Scroll Chaining for Nested Tool-Preview Regions |

**`phases`**

| Spec | Title |
|---|---|
| [`SPEC_LIGHT_THEME_AND_DEPTH_FIXES_2026_07_11`](SPEC_LIGHT_THEME_AND_DEPTH_FIXES_2026_07_11.md) | Spec: Light theme + theme-system depth fixes |
| [`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22`](SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md) | Pane block-stack mount flicker — root causes + reveal-gate generalization |

**`plan`**

| Spec | Title |
|---|---|
| [`SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29`](SPEC_PATCHED_MACOS_CEF_FRAMEWORK_RELEASE_2026_06_29.md) | SPEC: Patched macOS CEF Framework — Release Pipeline + CI Wiring |
| [`srv-phase-e4b-implementation-plan-2026-05-03`](srv-phase-e4b-implementation-plan-2026-05-03.md) | srv Phase E.4.B — Implementation Plan |

**`planned`**

| Spec | Title |
|---|---|
| [`SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20`](SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md) | SPEC: MuxBus — GitHub PR review notifications (end-to-end MVP) |
| [`SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22`](SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22.md) | SPEC: WebFetch content view |
| [`SPEC_WEBSEARCH_RICH_VIEW_2026_06_19`](SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md) | SPEC: Web-search rich result view |
| [`SPEC_WRITE_TOOL_CONTENT_VIEW_2026_06_19`](SPEC_WRITE_TOOL_CONTENT_VIEW_2026_06_19.md) | SPEC: Write tool expanded content view |
| [`SPEC_WRITE_TOOL_MD_RENDER_2026_06_23`](SPEC_WRITE_TOOL_MD_RENDER_2026_06_23.md) | SPEC: Render `.md` content as markdown in the Write tool overlay |

**`pr`**

| Spec | Title |
|---|---|
| [`SPEC_MACOS_RESIZE_HANDLES_V2`](SPEC_MACOS_RESIZE_HANDLES_V2.md) | macOS Window Resize Handles — Root Cause Analysis v2 |
| [`SPEC_SHARED_AGENT_REGISTRY_2026_05_12`](SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md) | Spec: Shared agent registry — cross-version "Continue agent" dropdown |

**`preimplementation`**

| Spec | Title |
|---|---|
| [`SPEC_REMOVE_WEBVIEW`](SPEC_REMOVE_WEBVIEW.md) | Spec: Remove Built-in Browser (Webview) and Tsunami |

**`proposal`**

| Spec | Title |
|---|---|
| [`ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19`](ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md) | Architecture: Armory/Stash Foundation Consolidation (North Star) |
| [`PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30`](PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md) | Proposal: A Composable Agent Model for the Armory |
| [`SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10`](SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md) | Spec: ABF v0.2 — Provider-Aware Components + Native Memory |
| [`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16`](SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md) | Canonical Agent Identity/History Persistence Protocol — Synthesis with Mandatory ABF |
| [`SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19`](SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md) | SPEC: Agent Waiting Ambient Sound |
| [`SPEC_LOCAL_BUILD_VERSIONING_2026_05_28`](SPEC_LOCAL_BUILD_VERSIONING_2026_05_28.md) | SPEC: Local-build versioning — stop committing bumps for smoke builds (2026-05-28) |
| [`SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08`](SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md) | Spec: MCP integration parity with Claude Desktop / Cursor, piloted on Ableton MCP |
| [`local-messagebus-architecture`](local-messagebus-architecture.md) | Local MessageBus Architecture |

**`ready`**

| Spec | Title |
|---|---|
| [`PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07`](PLAN_TAB_TEAROFF_PHASE1_WIN32_2026-05-07.md) | Tab tear-off Phase 1 — Win32 native drag loop |
| [`SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30`](SPEC_864_LAYOUT_SINGLE_WRITER_2026_06_30.md) | SPEC #864 — Collapse the Layout Split-Brain to a Single Writer |
| [`SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02`](SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md) | SPEC — Versioned model dropdowns (CLI-aware), Claude CLI pin-to-latest, single-toggle Log |
| [`SPEC_AGENT_PANE_MESSAGE_ENTER_ANIMATION_2026_05_30`](SPEC_AGENT_PANE_MESSAGE_ENTER_ANIMATION_2026_05_30.md) | SPEC: Agent Pane — New Message Enter Animation |
| [`SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15`](SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md) | SPEC: Auth Check False Positive — "authenticated as max" on Load |
| [`SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30`](SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30.md) | SPEC — Rename Pool-Promoted Floating Panes to `floating-<uuid>` (Option A) |
| [`SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18`](SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md) | Spec: Host vs Container Agent Differentiation |
| [`SPEC_LINUX_DOCS_UPDATE_2026_06_06`](SPEC_LINUX_DOCS_UPDATE_2026_06_06.md) | SPEC — Linux documentation catch-up |
| [`SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02`](SPEC_MEMORY_COMMIT_ATTRIBUTION_CORRECTION_2026_07_02.md) | SPEC — Commit-attribution correction + genuine AgentMux memory-hygiene fixes |
| [`SPEC_MODAL_PANE_CLIP_2026_04_24`](SPEC_MODAL_PANE_CLIP_2026_04_24.md) | Spec: Modal-v2 ↔ Native Pane Airspace Clipping |
| [`SPEC_MODEL_CATALOG_REFRESH_2026_07_02`](SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md) | SPEC — API-sourced model catalog: keep the agent-pane model dropdown current |
| [`SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06`](SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md) | Pillar 1 Step 2 — Persist the Two Host-Only Topology Facts to srv |
| [`SPEC_PILLAR1_STEP3_WINDOW_TOPOLOGY_2026_07_07`](SPEC_PILLAR1_STEP3_WINDOW_TOPOLOGY_2026_07_07.md) | Pillar 1 Step 3 — Persist Window Kind + Parent Linkage to srv |
| [`SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07`](SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md) | Pillar 1 Step 4 — Crash Reproject: Automatic Multi-Window Reconstruction |
| [`SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11`](SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11.md) | SPEC: Pool adoption for foreign labels + srv window-row label crumb + non-Windows close verification |
| [`SPEC_SPLASH_TELEMETRY_LINUX_2026_06_27`](SPEC_SPLASH_TELEMETRY_LINUX_2026_06_27.md) | SPEC: Splash Startup Telemetry — Linux |
| [`SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11`](SPEC_SRV_SUPERVISION_RECYCLE_2026_07_11.md) | SPEC: srv supervision via host recycle (#942 Phase 2) |
| [`SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30`](SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md) | SPEC — Strong Reducer-Authority for Layout (Intent-Driven srv Reducer) |
| [`SPEC_SYSINFO_CHART_ROBUSTNESS_2026_06_21`](SPEC_SYSINFO_CHART_ROBUSTNESS_2026_06_21.md) | Spec: Sysinfo CPU Chart Robustness |
| [`SPEC_TEST_SRV_SPAWN_GUARDS_2026_07_11`](SPEC_TEST_SRV_SPAWN_GUARDS_2026_07_11.md) | SPEC: Guard integration-test srv spawns (kill_on_drop / Job Object) |
| [`SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23`](SPEC_TOOL_BLOCK_UX_POLISH_2026_05_23.md) | SPEC: Tool Block UX Polish — Hover Delay, Collapse Animation, Post-Completion Hold, Thinking Label, Scroll Isolation |
| [`cef-portable-layout`](cef-portable-layout.md) | CEF Portable Layout — Clean Directory Spec |
| [`cef-ui-thread-dispatch`](cef-ui-thread-dispatch.md) | Spec: CEF UI Thread Dispatch for IPC Handlers |
| [`chrome-zoom-pane-headers`](chrome-zoom-pane-headers.md) | Spec: Chrome Zoom Includes Pane Headers |
| [`default-agent-roster`](default-agent-roster.md) | Default Agent Roster Spec |
| [`drag-drop-files-into-panes`](drag-drop-files-into-panes.md) | Spec: Drag & Drop Files Into Panes |
| [`layoutmodel-modularization`](layoutmodel-modularization.md) | LayoutModel Modularization Spec |
| [`node-timestamp-hover`](node-timestamp-hover.md) | Node Timestamp Hover |
| [`per-pane-cpu-memory`](per-pane-cpu-memory.md) | Spec: Per-Pane CPU + Memory Metrics Badge |
| [`status-bar-redesign`](status-bar-redesign.md) | Spec: Status Bar Redesign |
| [`sysinfo-body-context-menu`](sysinfo-body-context-menu.md) | Spec: Sysinfo Body Context Menu — Metric Selection |
| [`term-modularization`](term-modularization.md) | Term View Modularization Spec |
| [`uptime-clock-sync`](uptime-clock-sync.md) | Spec: Robust Multi-Window Uptime Clock Sync |
| [`window-close-process-cleanup`](window-close-process-cleanup.md) | Window Close Process Cleanup Spec |
| [`window-drag-dead-spots`](window-drag-dead-spots.md) | Spec: Eliminate Window Drag Dead Spots |

**`reference`**

| Spec | Title |
|---|---|
| [`ARCHITECTURE_ARMORY_2026_07_20`](ARCHITECTURE_ARMORY_2026_07_20.md) | Armory Architecture |
| [`AUDIT_SQLITE_SYSTEMS_2026_05_19`](AUDIT_SQLITE_SYSTEMS_2026_05_19.md) | AUDIT: SQLite Systems in AgentMux |
| [`CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17`](CHECKLIST_AGENT_DATA_SCOPE_ROUTING_2026_08_17.md) | PR Checklist: Agent Credential / Definition / Portable-Config / History Routing |

**`report`**

| Spec | Title |
|---|---|
| [`REPORT_AGENT_DEFINITION_DB_GAP_2026_07_27`](REPORT_AGENT_DEFINITION_DB_GAP_2026_07_27.md) | Cross-Branch Agent-Definition Gap — Why "Existing" Agents Fail Auth on a Fresh Dev Database |
| [`REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16`](REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md) | Agent Identity/History Fragmentation Across Builds — Root Cause and a Fast-Lookup Design |
| [`REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17`](REPORT_HISTORY_CONTINUITY_ACROSS_VERSION_UPGRADE_2026_08_17.md) | Conversation Continuity Across a Version/Channel Switch — Verification Report |
| [`REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16`](REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md) | Report: long-running processes, subagents, and the Swarm pane — consolidated state (2026-07-16) |
| [`REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26`](REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md) | Report: auto-detecting long-running tool calls (sleep and beyond) and docking them — status refresh, 2026-07-26 |
| [`REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16`](REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md) | Report: detecting blocking/long-running tool calls (sleep and beyond), returning the pane to the user, and dock lifecycle — 2026-07-16 |
| [`REPORT_REDUCER_STACK_AUDIT_2026_07_26`](REPORT_REDUCER_STACK_AUDIT_2026_07_26.md) | Reducer Stack Audit — Post-Mortem on Duplication, Coupling, and Modularization |

**`research`**

| Spec | Title |
|---|---|
| [`REPORT_AGENT_PANE_SYNTHESIZED_TEXT_AUDIT_2026_08_06`](REPORT_AGENT_PANE_SYNTHESIZED_TEXT_AUDIT_2026_08_06.md) | Report: Audit of AgentMux-Synthesized Text in the Agent Pane |
| [`REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16`](REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md) | Report: Is there a standard for Armory-style agent capability bundles? Research + proposal (2026-07-16) |
| [`agentmux-isolated-auth`](agentmux-isolated-auth.md) | Spec: AgentMux Isolated Claude/Anthropic Authentication |
| [`cef-size-reduction`](cef-size-reduction.md) | CEF Portable Size Reduction Spec |

**`resolved`**

| Spec | Title |
|---|---|
| [`SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22`](SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md) | SPEC — Status bar popovers render offset (and undersized) under Chrome zoom |
| [`SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25`](SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md) | Tab close (X) button — spurious select flash |
| [`linux-pool-startup-fill-2026-05-08`](linux-pool-startup-fill-2026-05-08.md) | Linux/macOS: wire startup-time window-pool fill |

**`root`**

| Spec | Title |
|---|---|
| [`REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16`](REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md) | Jekt Signing-Key Injection Gap — Normal-Launch Agents Never Got a Verified Identity |
| [`SPEC_LAN_DISCOVERY_TXT_CLOBBER_FIX_2026_08_16`](SPEC_LAN_DISCOVERY_TXT_CLOBBER_FIX_2026_08_16.md) | SPEC: LAN discovery peer metadata gets clobbered blank by TXT-less mDNS re-resolutions |
| [`SPEC_PANE_MINIMIZE_CARET_BUG_2026_06_24`](SPEC_PANE_MINIMIZE_CARET_BUG_2026_06_24.md) | SPEC — Pane Minimize Caret Not Flipping |
| [`focus-border-tab-switch-bug`](focus-border-tab-switch-bug.md) | Spec: Focus Border Breaks After Tab Switch |
| [`magnify-bugs`](magnify-bugs.md) | Spec: Magnify Button + Z-Index Bugs |
| [`magnify-z-index-analysis`](magnify-z-index-analysis.md) | Analysis: Magnified Pane Z-Index Stacking Bug |
| [`windows-firewall-fix`](windows-firewall-fix.md) | Windows Firewall Popup — Root Cause & Fix |

**`rootcause`**

| Spec | Title |
|---|---|
| [`SPEC_STATUSBAR_POPOVER_POSITION_WIN10_2026_06_26`](SPEC_STATUSBAR_POPOVER_POSITION_WIN10_2026_06_26.md) | SPEC: Status-Bar Popover Position Bug (Windows — browser pane open) |

**`rootcaused`**

| Spec | Title |
|---|---|
| [`SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02`](SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02.md) | SPEC — Composer strip responsive architecture: stop hiding the runtime controls |

**`scope`**

| Spec | Title |
|---|---|
| [`SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29`](SPEC_UPDATEAGENTINSTANCE_PARTIAL_UPDATE_2026_05_29.md) | SPEC: `updateagentinstance` partial-update refactor (2026-05-29) |

**`shipped`**

| Spec | Title |
|---|---|
| [`SPEC_POOL_PHASE7_MACOS_LINUX_2026_06_19`](SPEC_POOL_PHASE7_MACOS_LINUX_2026_06_19.md) | Phase 7 — Pre-warmed Window Pool for macOS and Linux |

**`spec`**

| Spec | Title |
|---|---|
| [`SPEC_AGENT_ACTIVITY_LOG_NO_AUTO_OPEN_2026_05_05`](SPEC_AGENT_ACTIVITY_LOG_NO_AUTO_OPEN_2026_05_05.md) | Agent Activity Log — kill auto-open + drop label |
| [`SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12`](SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md) | Spec: Agent pane session-replay framework |
| [`SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04`](SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md) | CEF 148 — Linux Drag/Right-Click/Transparency Forward-Port |
| [`SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05`](SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md) | Host Orphan-Instance Reconciliation — 2026-05-05 |
| [`SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05`](SPEC_LAUNCHER_LINUX_PACKAGED_AND_SPLASH_2026_06_05.md) | SPEC: Launcher + reducer/saga parity on Linux + Linux splash |
| [`SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30`](SPEC_LAUNCHER_MACOS_DEV_INTEGRATION_2026_05_30.md) | SPEC: Integrating `agentmux-launcher` into macOS / Linux `task dev` |
| [`SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31`](SPEC_LAUNCHER_MACOS_PACKAGED_AND_SPLASH_2026_05_31.md) | SPEC: Launcher in packaged macOS builds + restore the splash + tear-off crash |
| [`SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30`](SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md) | Linux floating-pane tear-off — implementation spec |
| [`SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28`](SPEC_MACOS_CEF_FRAMEWORK_BUNDLING_2026_05_28.md) | macOS CEF Framework Bundling for `task dev` and `task package:macos` |
| [`SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29`](SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md) | macOS floating-pane tear-off — implementation spec |
| [`SPEC_MACOS_PACKAGING_2026_05_30`](SPEC_MACOS_PACKAGING_2026_05_30.md) | Spec: macOS packaging — signed, launchable `AgentMux.app` / `.dmg` |
| [`SPEC_MACOS_TEAROFF_STABILITY_2026_05_29`](SPEC_MACOS_TEAROFF_STABILITY_2026_05_29.md) | macOS Tear-off Stability — implementation spec |
| [`SPEC_NAMED_AGENT_CONTINUATION_2026_05_12`](SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md) | Spec: Named agent continuation — launch modal dropdown of existing agents |
| [`SPEC_POOL_WINDOW_HWND_NULL_2026_05_06`](SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md) | Pool window HWND-null at promote time |
| [`SPEC_REMOVE_PIN_FEATURE`](SPEC_REMOVE_PIN_FEATURE.md) | SPEC: Remove Tab Pinning, Uniform Inter-Tab Separator |
| [`SPEC_SUPPRESS_OS_CREDENTIAL_PROMPTS_2026_05_30`](SPEC_SUPPRESS_OS_CREDENTIAL_PROMPTS_2026_05_30.md) | Spec: Never request OS credential / keychain access (all runtime modes) |
| [`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07`](SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md) | Tab tear-off — native drag loop (Chrome's Win32/X11 model) |
| [`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26`](SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md) | Tab Tear-Off — Chrome-Faithful Window-Move Architecture |
| [`SPEC_TASK_DEV_LAUNCHER_GAPS_2026_05_06`](SPEC_TASK_DEV_LAUNCHER_GAPS_2026_05_06.md) | task dev — launcher-driven shutdown gaps |
| [`SPEC_TEAR_OFF_POOL_PATH_2026_05_06`](SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md) | Tab Tear-Off — Always Use Warm Pool + Source-Side Renderer Crash |
| [`SPEC_TOOLCHAIN_MANAGER_EXTERNAL_WIDGETS_2026_06_22`](SPEC_TOOLCHAIN_MANAGER_EXTERNAL_WIDGETS_2026_06_22.md) | Toolchain Manager — External Widgets Extension |
| [`SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13`](SPEC_UNIFIED_TOOL_HOVER_OVERLAY_2026_05_13.md) | Spec: Unified tool-block hover overlay (no double-popup) |
| [`SPEC_UPGRADE_PANEL_2026_06_27`](SPEC_UPGRADE_PANEL_2026_06_27.md) | Maintenance Section in InstancePanel |
| [`clipboard-cef-impl`](clipboard-cef-impl.md) | Clipboard for CEF — Implementation Spec |
| [`embedded-browser-panes-linux-macos-2026-05-03`](embedded-browser-panes-linux-macos-2026-05-03.md) | Embedded Browser Panes — Linux & macOS Port |
| [`hostname-popover`](hostname-popover.md) | Hostname Popover — Network & Instance Info |
| [`launch-modal-rearchitecture-2026-05-01`](launch-modal-rearchitecture-2026-05-01.md) | Launch Agent Modal — Performance & Per-Tab Scoping |
| [`linux-icon-and-desktop-2026-05-03`](linux-icon-and-desktop-2026-05-03.md) | Linux Taskbar Icon & Desktop Registration |
| [`multi-window-pane-and-newwindow-fixes-linux-2026-05-15`](multi-window-pane-and-newwindow-fixes-linux-2026-05-15.md) | Multi-window correctness on Linux: pane RequestContext, new-window client, tab-switch overlay visibility |
| [`patched-libcef-bundling-2026-05-08`](patched-libcef-bundling-2026-05-08.md) | Patched libcef.so Bundling for Linux |
| [`secondary-windows-cef-views`](secondary-windows-cef-views.md) | Secondary Windows: Switch from Native to CEF Views |
| [`single-instance-new-window`](single-instance-new-window.md) | Single Instance + New Window on Re-launch |
| [`swarm-redesign-active-retired-2026-05-03`](swarm-redesign-active-retired-2026-05-03.md) | Swarm Pane Redesign — Active / Retired + Pane-Flip Detail |

**`specification`**

| Spec | Title |
|---|---|
| [`SPEC_PHASE_E_SAGAS_2026-04-30`](SPEC_PHASE_E_SAGAS_2026-04-30.md) | Phase E Sagas — Full Specification |

**`tier`**

| Spec | Title |
|---|---|
| [`SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24`](SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24.md) | Spec: GPU Memory Tracing Scaffolding — a real trace, not another process-level guess |

**`tracking`**

| Spec | Title |
|---|---|
| [`SPEC_AGENT_ARCHITECTURE_2026_05_27`](SPEC_AGENT_ARCHITECTURE_2026_05_27.md) | SPEC: Agent data-model architecture — consolidation plan & status |

**`updated`**

| Spec | Title |
|---|---|
| [`PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05`](PLAN_AGENT_PANE_RESIZE_SCROLL_PIN_2026_08_05.md) | Fix Plan: Agent Pane Loses Bottom-Pin on Pane Resize |

<!-- END GENERATED INDEX -->

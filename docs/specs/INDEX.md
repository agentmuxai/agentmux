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

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstancePanel — popover anchored under the version chip in the status
 * bar's bottom-right. Surfaces "About" metadata + a list of open windows
 * in this AgentMux process, with actions to focus a window or open a
 * new one. Replaces the version chip's old "click → openNewWindow"
 * behaviour with a richer affordance.
 *
 * Spec: SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md
 *
 * V1 scope: about-info + windows + actions. LAN peers stay in
 * HostPopover (they already have a richer hover-rich detail view).
 * Per-window token totals deferred until token-usage is per-window.
 */

import { atoms, getApi, isDev, openFloatingPaneEntriesAtom, openWindowEntriesAtom, type FloatingPaneEntry, type WindowEntry } from "@/store/global";
import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
import { MaintenanceSection } from "./MaintenanceSection";
import { reconcileKnownEntriesFromSnapshot } from "@/app/store/launcher-event-reducer";
import { launcherEventsActive } from "@/util/launcher-events";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { ObjectService } from "@/store/services";
import { getObjectValue, makeORef } from "@/store/wos";
import { dispatchWindowOpacity, liveWindowOpacity } from "@/app/store/window-opacity-store";
import { createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import {
    DISPLAY_NAME_MAX_LEN,
    DISPLAY_NAME_META_KEY,
    resolveFloatingPaneName,
    resolveWindowName,
} from "@/util/window-title";

interface InstancePanelProps {
    anchorRect: DOMRect | null;
    onClose: () => void;
}

const POPOVER_WIDTH = 320;
const GUTTER = 8;

export const InstancePanel = (props: InstancePanelProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the popover paints over any browser pane HWND that
    // the status bar overlaps. Same primitive as TokenBreakdownPopover,
    // MoreDropdown, and `<Modal>`.
    usePaneOverlay(() => rootRef);

    const about = createMemo(() => {
        const d = getApi().getAboutModalDetails();
        return {
            version: d?.version ?? "unknown",
            channel: d?.channel ?? null,
            buildLabel: (d as any)?.buildLabel ?? null,
            gitHash: d?.gitHash ?? null,
            buildTime: typeof d?.buildTime === "number" && d.buildTime > 0 ? d.buildTime : null,
            platform: (d as any)?.platform ?? null,
            arch: (d as any)?.arch ?? null,
        };
    });

    // Build timestamp -> "Jan 3, 2019 8:12AM": abbreviated month, no
    // leading-zero day/hour, 2-digit minute, AM/PM with no separating space.
    const formatBuildTime = (ms: number): string => {
        const s = new Date(ms).toLocaleString("en-US", {
            month: "short",
            day: "numeric",
            year: "numeric",
            hour: "numeric",
            minute: "2-digit",
            hour12: true,
        });
        return s.replace(/, (\d{1,2}:\d{2})/, " $1").replace(/\s(AM|PM)$/, "$1");
    };

    const entries = openWindowEntriesAtom;
    const floatingEntries = openFloatingPaneEntriesAtom;
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi().getWindowLabel().then((l) => setMyLabel(l)).catch(() => setMyLabel(null));

    // MuxBus connection status — refreshed on open (mirrors HostPopover's
    // own `void muxbus.refresh()` on-open pattern). A missing/expired
    // session fails WAN jekt delivery completely silently otherwise (no
    // error, nothing — see this row's CSS comment for the incident that
    // motivated surfacing it here).
    const muxbus = useMuxBusStatus();
    void muxbus.refresh();
    const muxbusOk = () => {
        const s = muxbus.status();
        return !!s && s.connected && s.valid;
    };

    // Refresh window-instance state ONLY when the launcher is silent
    // (`task dev` mode — no launcher process, no typed events). In
    // production the launcher pushes WindowOpened/Closed events and
    // `openWindowEntriesAtom` stays in sync; reconciling against a
    // stale `listWindowInstances()` snapshot would clobber a newer
    // launcher event the renderer just applied — same race protection
    // that motivates ApplySeed at boot (codex P2 PR #733).
    //
    // `launcherEventsActive` flips true on the first typed event the
    // renderer receives; if false here, we're in dev mode and the
    // reconcile is the only way windows opened/closed since boot will
    // appear/disappear in the panel.
    if (!launcherEventsActive()) {
        getApi()
            .listWindowInstances()
            .then((snapshot) => {
                // Re-check at resolution time. Between the sync gate
                // above and the snapshot arriving (~ms RPC round-trip),
                // a launcher event may have flipped launcherEventsActive
                // true — applying the now-stale snapshot would clobber
                // that newer state (codex P1 PR #733 round 2).
                if (launcherEventsActive()) return;
                reconcileKnownEntriesFromSnapshot(snapshot);
            })
            .catch(() => {});
    }

    // Rename mode state — keyed by host label so the row's identity
    // survives entries() reordering. Single rename at a time.
    const [editingLabel, setEditingLabel] = createSignal<string | null>(null);
    const [editValue, setEditValue] = createSignal("");

    // Defer single-click focus past the dblclick threshold so
    // double-clicking a row enters rename mode WITHOUT first
    // bringing the other window forward (which would close /
    // hide this panel and abort the rename). Tracks the pending
    // focus by row label so a click on row B while a focus is
    // pending for row A still treats B as a fresh single click.
    // (reagent / codex / gemini PR #569 P1)
    //
    // Threshold queried from the OS (Win32 GetDoubleClickTime,
    // user-configurable, default 500ms) so slow double-clickers
    // are accommodated. Defaults to 500ms while the query is
    // in flight, plus a 50ms buffer to absorb event-dispatch jitter.
    // (codex PR #569 round-2 P2)
    const [dblclickDelayMs, setDblclickDelayMs] = createSignal(550);
    getApi()
        .getDoubleClickTime()
        .then((ms) => setDblclickDelayMs(Math.max(120, ms + 50)))
        .catch(() => {/* keep default */});

    let pendingFocus: { label: string; timer: number } | null = null;
    const cancelPendingFocus = () => {
        if (pendingFocus) {
            clearTimeout(pendingFocus.timer);
            pendingFocus = null;
        }
    };

    const positioning = createMemo(() => {
        const r = props.anchorRect;
        if (!r) return { bottom: GUTTER, right: GUTTER };
        const rightFromViewport = Math.max(GUTTER, window.innerWidth - r.right);
        const bottomFromViewport = Math.max(GUTTER, window.innerHeight - r.top);
        return { bottom: bottomFromViewport, right: rightFromViewport };
    });

    const handleFocusWindow = async (label: string) => {
        if (label === myLabel()) return; // already focused
        try {
            await getApi().focusWindow(label);
        } catch (e) {
            console.error("[InstancePanel] focusWindow failed:", e);
        }
    };

    const handleFocusPane = async (label: string) => {
        try {
            await getApi().focusWindow(label);
        } catch (e) {
            console.error("[InstancePanel] focusPane failed:", e);
        }
    };

    // Readable label for a floating pane. Resolution order:
    //   1. block view type (title-cased, e.g. "Agent", "Terminal")
    //   2. workspace name
    //   3. positional fallback "Pane N"
    const PANE_VIEW_LABELS: Record<string, string> = {
        agent: "Agent",
        term: "Terminal",
        browser: "Browser",
        editor: "Editor",
        sysinfo: "System Info",
        drone: "Drone",
        swarm: "Swarm",
        help: "Help",
        warden: "Warden",
    };

    const resolveFloatingName = (entry: FloatingPaneEntry, idx: number): string => {
        let blockViewLabel: string | undefined;
        let workspaceName: string | undefined;
        if (entry.windowId) {
            const win = getObjectValue<WaveWindow>(makeORef("window", entry.windowId));
            if (win?.workspaceid) {
                const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
                workspaceName = ws?.name;
                const tab = ws?.activetabid
                    ? getObjectValue<Tab>(makeORef("tab", ws.activetabid))
                    : null;
                const blockId = tab?.blockids?.[0];
                if (blockId) {
                    const block = getObjectValue<Block>(makeORef("block", blockId));
                    const view = block?.meta?.view as string | undefined;
                    if (view) blockViewLabel = PANE_VIEW_LABELS[view] ?? view;
                }
            }
        }
        return resolveFloatingPaneName({ blockViewLabel, workspaceName, indexInOpenPanes: idx });
    };

    // For THIS window's row, fall back to atoms.waveWindow()?.oid when
    // the entry's windowId is still null. WindowEntry.windowId is null
    // for the first ~100ms after a window opens — until the
    // registerBackendWindow IPC round-trip completes (see comment at
    // global.ts:145). Without this fallback, the panel shows "Window N"
    // for the current window during early-startup while the OS title
    // (which uses initOpts.windowId directly) correctly shows the
    // workspace name — visible inconsistency the user reported.
    const resolveEntryWindowId = (entry: WindowEntry): string | null => {
        if (entry.windowId) return entry.windowId;
        if (entry.label === myLabel()) return atoms.waveWindow()?.oid ?? null;
        return null;
    };

    // Resolve a row's display name via the shared helper so the panel and
    // the OS window title (driven from app-init.ts) agree by construction.
    // Reactive via Wave's object subscriptions because getObjectValue reads
    // through atoms — when meta or workspace.name changes, this re-runs.
    const resolveName = (entry: WindowEntry, idx: number): string => {
        let displayName: string | undefined;
        let workspaceName: string | undefined;
        const windowId = resolveEntryWindowId(entry);
        if (windowId) {
            const win = getObjectValue<WaveWindow>(makeORef("window", windowId));
            displayName = win?.meta?.[DISPLAY_NAME_META_KEY] as string | undefined;
            if (win?.workspaceid) {
                const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
                workspaceName = ws?.name;
            }
        }
        const name = resolveWindowName({ displayName, workspaceName, indexInOpenWindows: idx });

        // Diagnostic — for this window's row only, log the same shape as
        // [wave-title] in app-init.ts. If both surfaces resolve the same
        // window with different `name` values, the inputs disagree and
        // the bug is the inputs (typically: idx mismatch from the
        // registerBackendWindow race). Tail with:
        //   muxlog host '\[fe\] \[wave-(title|panel)\]'
        if (entry.label === myLabel()) {
            console.debug(
                "[wave-panel]",
                "windowId=" + (windowId ?? "<null>"),
                "label=" + entry.label,
                "idx=" + idx,
                "displayName=" + (displayName ?? "<none>"),
                "workspaceName=" + (workspaceName ?? "<none>"),
                "→ name=" + JSON.stringify(name),
            );
        }
        return name;
    };

    const enterRename = (entry: WindowEntry, currentName: string) => {
        // Same fallback as resolveName: for this window's row we can use
        // atoms.waveWindow()?.oid when the entry's windowId hasn't been
        // populated yet via the registerBackendWindow round-trip.
        if (!resolveEntryWindowId(entry)) return;
        setEditingLabel(entry.label);
        setEditValue(currentName);
    };

    const cancelRename = () => {
        setEditingLabel(null);
        setEditValue("");
    };

    // Single shared persistence path used by both commitRename
    // (Enter / blur) and the onCleanup unmount-flush. Keeps the
    // trim + cap + RPC + error-log shape consistent. (gemini
    // PR #569 round-3 MEDIUM @ L176)
    const performRename = (windowId: string, name: string) => {
        ObjectService.UpdateObjectMeta(
            makeORef("window", windowId),
            { [DISPLAY_NAME_META_KEY]: name } as MetaType,
        ).catch((e) => console.error("[InstancePanel] rename failed:", e));
    };

    const commitRename = (entry: WindowEntry) => {
        const editing = editingLabel();
        if (editing !== entry.label) return; // stale
        const windowId = resolveEntryWindowId(entry);
        if (!windowId) {
            cancelRename();
            return;
        }
        const trimmed = editValue().trim().slice(0, DISPLAY_NAME_MAX_LEN);
        // Empty after trim → silent revert per spec §2.2.2.
        setEditingLabel(null);
        setEditValue("");
        if (!trimmed) return;
        performRename(windowId, trimmed);
    };

    // Inline opacity control — shares one row with the window/pane name
    // (docs/specs/instance-panel-floating-panes.md §3.3). `persistWindowId`
    // present → window rows persist to `window:opacity` meta on release;
    // null → floating panes, session-only (a floater has no backing window
    // object to persist to, and doesn't survive an app restart anyway).
    // stopPropagation on all three event kinds so a slider drag/keypress
    // never triggers the row's focus or rename handlers.
    const OpacityControl = (p: {
        label: string;
        persistWindowId: string | null;
        currentOpacity: () => number;
    }): JSX.Element => (
        <span
            class="instance-panel-opacity"
            onClick={(e) => e.stopPropagation()}
            onDblClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
        >
            <input
                type="range"
                class="instance-panel-opacity-slider"
                min={0.35}
                max={1.0}
                step={0.05}
                value={p.currentOpacity()}
                title="Opacity"
                aria-label="Opacity"
                onInput={(e) => {
                    const val = parseFloat(e.currentTarget.value);
                    dispatchWindowOpacity({
                        type: "SetWindowOpacity",
                        label: p.label,
                        opacity: val,
                        source: "user",
                    });
                }}
                onChange={(e) => {
                    if (!p.persistWindowId) return; // floater: session-only
                    const raw = parseFloat(e.currentTarget.value);
                    const val = Math.round(raw * 100) / 100;
                    // Set window:transparent alongside window:opacity so
                    // AppSettingsUpdater applies it correctly on restore.
                    const fullyOpaque = val >= 1.0;
                    ObjectService.UpdateObjectMeta(
                        makeORef("window", p.persistWindowId),
                        {
                            "window:opacity": fullyOpaque ? null : val,
                            "window:transparent": fullyOpaque ? false : true,
                        } as MetaType,
                    ).catch((err) =>
                        console.error("[InstancePanel] opacity persist failed:", err),
                    );
                }}
            />
            <span class="instance-panel-opacity-value">
                {/* Live store value tracks the drag tick-by-tick; falls back
                    to the row's resolved opacity when the store has no entry
                    yet this session. */}
                {Math.round((liveWindowOpacity(p.label) ?? p.currentOpacity()) * 100)}%
            </span>
        </span>
    );

    // Panel-unmount cleanup:
    //   1. Cancel any pending focus timer so it doesn't fire after
    //      the panel is gone (focus call would still hit a real
    //      window, but it's a wasted call + a small leak).
    //      (gemini PR #569 round-3 MEDIUM @ L77)
    //   2. Flush any in-flight rename. StatusBar's outside-click
    //      dismiss can unmount the input before its onBlur fires,
    //      which would otherwise silently lose the typed value.
    //      (codex PR #569 round-2 P1)
    onCleanup(() => {
        cancelPendingFocus();
        const editing = editingLabel();
        if (!editing) return;
        const entry = entries().find((e) => e.label === editing);
        if (!entry) return;
        const windowId = resolveEntryWindowId(entry);
        if (!windowId) return;
        const trimmed = editValue().trim().slice(0, DISPLAY_NAME_MAX_LEN);
        if (!trimmed) return;
        performRename(windowId, trimmed);
    });

    const handleOpenNewWindow = async () => {
        try {
            await getApi().openNewWindow();
        } catch (e) {
            console.error("[InstancePanel] openNewWindow failed:", e);
        }
        props.onClose();
    };

    const handleCopy = (label: string, value: string) => {
        clipboardWriteText(`${label}: ${value}`);
    };


    return (
        <div
            ref={(el) => (rootRef = el)}
            class="instance-panel"
            role="dialog"
            aria-label="AgentMux instance panel"
            style={{
                position: "fixed",
                bottom: `${positioning().bottom}px`,
                right: `${positioning().right}px`,
                width: `${POPOVER_WIDTH}px`,
            }}
        >
            <div class="instance-panel-header">
                <div class="instance-panel-row instance-panel-row-meta">
                    <span class="instance-panel-label">Version</span>
                    <span class="instance-panel-value">
                        v{about().version}
                        <Show when={isDev()}>
                            <span class="status-version-dev">DEV</span>
                        </Show>
                    </span>
                    <button
                        type="button"
                        class="instance-panel-copy"
                        title="Copy version"
                        onClick={() => handleCopy("version", `v${about().version}`)}
                    >
                        ⧉
                    </button>
                </div>
                <Show when={about().channel}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">Channel</span>
                        <span class="instance-panel-value instance-panel-mono">{about().channel}</span>
                        <button
                            type="button"
                            class="instance-panel-copy"
                            title="Copy channel"
                            onClick={() => clipboardWriteText(about().channel!)}
                        >
                            ⧉
                        </button>
                    </div>
                </Show>
                <Show when={about().gitHash}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">Build</span>
                        <span class="instance-panel-value instance-panel-mono">{about().gitHash}</span>
                        <button
                            type="button"
                            class="instance-panel-copy"
                            title="Copy build hash"
                            onClick={() => clipboardWriteText(about().gitHash!)}
                        >
                            ⧉
                        </button>
                    </div>
                </Show>
                <Show when={about().buildTime}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">Time</span>
                        <span class="instance-panel-value instance-panel-mono">
                            {formatBuildTime(about().buildTime!)}
                        </span>
                    </div>
                </Show>
                <Show when={about().platform || about().arch}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">Runtime</span>
                        <span class="instance-panel-value instance-panel-mono">
                            {[about().platform, about().arch].filter(Boolean).join(" · ")}
                        </span>
                    </div>
                </Show>
                <Show when={muxbus.status() !== null && !muxbusOk()}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">MuxBus</span>
                        <span class="instance-panel-value">
                            <span class="instance-panel-muxbus-icon">◈</span>
                            <span
                                class="instance-panel-muxbus-pill"
                                title="This instance has no valid MuxBus session — cloud/WAN jekt delivery will not work until you sign in again."
                            >
                                Not connected
                            </span>
                        </span>
                    </div>
                </Show>
            </div>
            <div class="instance-panel-divider" />
            <div class="instance-panel-section">
                <div class="instance-panel-section-title">
                    This process — {entries().length} window{entries().length !== 1 ? "s" : ""}
                </div>
                <For each={entries()}>
                    {(entry, i) => {
                        const isCurrent = () => entry.label === myLabel();
                        const isEditing = () => editingLabel() === entry.label;
                        const currentName = () => resolveName(entry, i());
                        const currentOpacity = () => {
                            if (!entry.windowId) return 1.0;
                            const win = getObjectValue<WaveWindow>(makeORef("window", entry.windowId));
                            return (win?.meta?.["window:opacity"] as number | undefined) ?? 1.0;
                        };
                        return (
                            <>
                            <div
                                class="instance-panel-window-row"
                                classList={{
                                    "instance-panel-window-row-current": isCurrent(),
                                    "instance-panel-window-row-editing": isEditing(),
                                }}
                                onClick={(e) => {
                                    if (isEditing()) {
                                        e.stopPropagation();
                                        return;
                                    }
                                    // Focus immediately — the previous design
                                    // deferred via setTimeout(dblclickDelayMs)
                                    // to disambiguate from dblclick-rename, but
                                    // the StatusBar's outside-click dismiss
                                    // unmounts InstancePanel before the timer
                                    // fires, and onCleanup cancels the timer.
                                    // Result: clicks did nothing.
                                    //
                                    // Firing focus immediately means a
                                    // double-click still triggers focus first
                                    // (harmless — rename input then takes
                                    // over). dblClick handler still runs and
                                    // enters rename mode.
                                    handleFocusWindow(entry.label);
                                }}
                                onDblClick={(e) => {
                                    e.preventDefault();
                                    e.stopPropagation();
                                    cancelPendingFocus();
                                    enterRename(entry, currentName());
                                }}
                                onKeyDown={(e) => {
                                    if (isEditing()) return; // input owns key handling
                                    if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        cancelPendingFocus();
                                        handleFocusWindow(entry.label);
                                    } else if (e.key === "F2") {
                                        e.preventDefault();
                                        cancelPendingFocus();
                                        enterRename(entry, currentName());
                                    }
                                }}
                                title={isCurrent() ? "This window — double-click to rename (F2)" : `Click to focus, double-click to rename (F2)`}
                                role="button"
                                tabIndex={0}
                            >
                                <span class="instance-panel-window-dot">{isCurrent() ? "●" : "○"}</span>
                                <Show
                                    when={isEditing()}
                                    fallback={
                                        <span class="instance-panel-window-name">{currentName()}</span>
                                    }
                                >
                                    <input
                                        ref={(el) => {
                                            if (el) {
                                                queueMicrotask(() => {
                                                    el.focus();
                                                    el.select();
                                                });
                                            }
                                        }}
                                        class="instance-panel-window-name-input"
                                        value={editValue()}
                                        maxLength={DISPLAY_NAME_MAX_LEN}
                                        onInput={(e) => setEditValue(e.currentTarget.value)}
                                        onKeyDown={(e) => {
                                            // Stop global key handlers from firing while editing.
                                            e.stopPropagation();
                                            if (e.key === "Enter") {
                                                e.preventDefault();
                                                commitRename(entry);
                                            } else if (e.key === "Escape") {
                                                e.preventDefault();
                                                cancelRename();
                                            }
                                        }}
                                        onBlur={() => commitRename(entry)}
                                        onClick={(e) => e.stopPropagation()}
                                        onDblClick={(e) => e.stopPropagation()}
                                    />
                                </Show>
                                <Show when={isCurrent() && !isEditing()}>
                                    <span class="instance-panel-window-badge">this</span>
                                </Show>
                                {/* Inline opacity — name + slider share one row
                                    (instance-panel-floating-panes.md §3.3). Stays
                                    visible during rename; its own stopPropagation
                                    keeps drags from focusing/renaming. */}
                                <Show when={!!entry.windowId}>
                                    <OpacityControl
                                        label={entry.label}
                                        persistWindowId={entry.windowId}
                                        currentOpacity={currentOpacity}
                                    />
                                </Show>
                            </div>
                            </>
                        );
                    }}
                </For>
            </div>
            <div class="instance-panel-divider" />
            <div class="instance-panel-section">
                <div class="instance-panel-section-title">
                    Floating panes — {floatingEntries().length}
                </div>
                <Show
                    when={floatingEntries().length > 0}
                    fallback={
                        <div class="instance-panel-pane-empty">No floating panes</div>
                    }
                >
                    <For each={floatingEntries()}>
                        {(entry, i) => (
                            <div
                                class="instance-panel-pane-row"
                                onClick={() => handleFocusPane(entry.label)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter" || e.key === " ") {
                                        e.preventDefault();
                                        handleFocusPane(entry.label);
                                    }
                                }}
                                title="Click to focus"
                                role="button"
                                tabIndex={0}
                            >
                                <span class="instance-panel-pane-icon">◈</span>
                                <span class="instance-panel-pane-name">
                                    {resolveFloatingName(entry, i())}
                                </span>
                                {/* Floaters get the same inline opacity control
                                    as windows (instance-panel-floating-panes.md
                                    §3.2). Session-only: no backing window object
                                    to persist to, so persistWindowId is null and
                                    the resolved value comes from the live store. */}
                                <OpacityControl
                                    label={entry.label}
                                    persistWindowId={null}
                                    currentOpacity={() => liveWindowOpacity(entry.label) ?? 1.0}
                                />
                            </div>
                        )}
                    </For>
                </Show>
            </div>
            <div class="instance-panel-divider" />
            <div class="instance-panel-section">
                <MaintenanceSection />
            </div>
            <div class="instance-panel-divider" />
            <div class="instance-panel-footer">
                <button
                    type="button"
                    class="instance-panel-btn instance-panel-btn-primary"
                    onClick={handleOpenNewWindow}
                >
                    + Open another window
                </button>
                <button
                    type="button"
                    class="instance-panel-btn"
                    onClick={props.onClose}
                >
                    Close
                </button>
            </div>
        </div>
    );
};

InstancePanel.displayName = "InstancePanel";

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

import { getApi, openWindowEntriesAtom, type WindowEntry } from "@/store/global";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { ObjectService } from "@/store/services";
import { getObjectValue, makeORef } from "@/store/wos";
import { createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

const DISPLAY_NAME_META_KEY = "window:displayname";
const DISPLAY_NAME_MAX_LEN = 64;

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
    // MoreDropdown, modal-v2.
    usePaneOverlay(() => rootRef);

    const about = createMemo(() => {
        const d = getApi().getAboutModalDetails();
        return {
            version: d?.version ?? "unknown",
            buildTime: d?.buildTime ? String(d.buildTime) : null,
            platform: (d as any)?.platform ?? null,
            arch: (d as any)?.arch ?? null,
        };
    });

    const entries = openWindowEntriesAtom;
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi().getWindowLabel().then((l) => setMyLabel(l)).catch(() => setMyLabel(null));

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

    // Resolve a row's display name in priority order:
    //   1. user-set name in Window meta (`window:displayname`)
    //   2. workspace.name (when the user has named the workspace)
    //   3. index-based fallback "Window N"
    // Returns the resolved string for rendering. Reactive via Wave's
    // object subscriptions because getObjectValue reads through atoms.
    const resolveName = (entry: WindowEntry, idx: number): string => {
        if (entry.windowId) {
            const win = getObjectValue<WaveWindow>(makeORef("window", entry.windowId));
            const userName = (win?.meta?.[DISPLAY_NAME_META_KEY] as string | undefined)?.trim();
            if (userName) return userName;
            if (win?.workspaceid) {
                const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
                if (ws?.name?.trim()) return ws.name.trim();
            }
        }
        return `Window ${idx + 1}`;
    };

    const enterRename = (entry: WindowEntry, currentName: string) => {
        if (!entry.windowId) return; // can't rename a window without a backend record yet
        setEditingLabel(entry.label);
        setEditValue(currentName);
    };

    const cancelRename = () => {
        setEditingLabel(null);
        setEditValue("");
    };

    const commitRename = async (entry: WindowEntry) => {
        const editing = editingLabel();
        if (editing !== entry.label) return; // stale
        if (!entry.windowId) {
            cancelRename();
            return;
        }
        const trimmed = editValue().trim().slice(0, DISPLAY_NAME_MAX_LEN);
        // Empty after trim → silent revert per spec §2.2.2.
        const next = editingLabel();
        setEditingLabel(null);
        setEditValue("");
        if (!trimmed || next !== entry.label) return;
        try {
            await ObjectService.UpdateObjectMeta(
                makeORef("window", entry.windowId),
                { [DISPLAY_NAME_META_KEY]: trimmed } as MetaType,
            );
        } catch (e) {
            console.error("[InstancePanel] rename failed:", e);
        }
    };

    // Commit any in-flight rename on panel unmount. The panel can be
    // dismissed by StatusBar's outside-click handler before the input's
    // onBlur fires, which would otherwise silently lose the typed
    // value. fireAndForget — the panel is going away regardless and
    // we don't block teardown on the RPC. (codex PR #569 round-2 P1)
    onCleanup(() => {
        const editing = editingLabel();
        if (!editing) return;
        const entry = entries().find((e) => e.label === editing);
        if (!entry || !entry.windowId) return;
        const trimmed = editValue().trim().slice(0, DISPLAY_NAME_MAX_LEN);
        if (!trimmed) return;
        ObjectService.UpdateObjectMeta(
            makeORef("window", entry.windowId),
            { [DISPLAY_NAME_META_KEY]: trimmed } as MetaType,
        ).catch((e) => console.error("[InstancePanel] cleanup rename failed:", e));
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
                    <span class="instance-panel-value">v{about().version}</span>
                    <button
                        type="button"
                        class="instance-panel-copy"
                        title="Copy version"
                        onClick={() => handleCopy("version", `v${about().version}`)}
                    >
                        ⧉
                    </button>
                </div>
                <Show when={about().buildTime}>
                    <div class="instance-panel-row instance-panel-row-meta">
                        <span class="instance-panel-label">Build</span>
                        <span class="instance-panel-value instance-panel-mono">{about().buildTime}</span>
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
                        return (
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
                                    // If a focus was pending on THIS row, this is
                                    // the second click of a dblclick — let dblClick
                                    // handle it; cancel the pending focus.
                                    if (pendingFocus && pendingFocus.label === entry.label) {
                                        cancelPendingFocus();
                                        return;
                                    }
                                    // Otherwise schedule a fresh focus past the
                                    // dblclick threshold so a follow-up click can
                                    // promote it to rename instead.
                                    cancelPendingFocus();
                                    const label = entry.label;
                                    pendingFocus = {
                                        label,
                                        timer: window.setTimeout(() => {
                                            pendingFocus = null;
                                            handleFocusWindow(label);
                                        }, dblclickDelayMs()),
                                    };
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
                            </div>
                        );
                    }}
                </For>
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

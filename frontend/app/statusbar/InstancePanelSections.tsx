// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The sections of the InstancePanel around its window list: the About rows, the workspace heading
 * with its inline rename, the floating panes, and the footer.
 */

import { getApi, isDev, type FloatingPaneEntry } from "@/store/global";
import { hostHas } from "@/util/host-caps";
import { liveWindowOpacity } from "@/app/store/window-opacity-store";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { formatUtcTimestamp } from "@/util/format-time";
import { openNewWindowFromHere } from "@/app/store/window-appearance";
import { For, Show, type Accessor, type JSX } from "solid-js";
import { Button, IconButton } from "@/app/element/ui";
import { WORKSPACE_NAME_MAX_LEN, workspaceHeading } from "@/app/store/user-workspace";
import { tip } from "@/app/element/tip";
import { resolveFloatingName } from "./instance-floating-name";
import type { OpacityControlComponent } from "./InstancePanelRows";
import type { AboutDetails, WorkspaceRename } from "./instance-panel-state";

/** "About" metadata: version, the channel its data is in, commit, CEF, runtime and build time, each only when the host reports it. */
export const AboutSection = (props: { about: Accessor<AboutDetails> }): JSX.Element => {
    const about = props.about;
    return (
        <div class="instance-panel-header">
            <div class="instance-panel-row instance-panel-row-meta">
                <span class="instance-panel-label">Version</span>
                <span class="instance-panel-value">
                    v{about().version}
                    <Show when={isDev()}>
                        <span class="status-version-dev">DEV</span>
                    </Show>
                </span>
                <IconButton
                    icon="copy"
                    label="Copy version"
                    density="compact"
                    class="instance-panel-copy"
                    onClick={() => clipboardWriteText(`version: v${about().version}`)}
                />
            </div>
            <Show when={about().channel}>
                <div class="instance-panel-row instance-panel-row-meta">
                    <span class="instance-panel-label">Data channel</span>
                    <span class="instance-panel-value instance-panel-mono">{about().channel}</span>
                    <IconButton
                        icon="copy"
                        label="Copy data channel"
                        density="compact"
                        class="instance-panel-copy"
                        onClick={() => clipboardWriteText(about().channel!)}
                    />
                </div>
            </Show>
            <Show when={about().gitHash}>
                <div class="instance-panel-row instance-panel-row-meta">
                    <span class="instance-panel-label">Commit</span>
                    <span class="instance-panel-value instance-panel-mono">{about().gitHash}</span>
                    <IconButton
                        icon="copy"
                        label="Copy commit hash"
                        density="compact"
                        class="instance-panel-copy"
                        onClick={() => clipboardWriteText(about().gitHash!)}
                    />
                </div>
            </Show>
            <Show when={about().cefVersion}>
                <div class="instance-panel-row instance-panel-row-meta">
                    <span class="instance-panel-label">CEF</span>
                    <span class="instance-panel-value instance-panel-mono">{about().cefVersion}</span>
                    <IconButton
                        icon="copy"
                        label="Copy CEF version"
                        density="compact"
                        class="instance-panel-copy"
                        onClick={() => clipboardWriteText(about().cefVersion!)}
                    />
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
            <Show when={about().buildTime}>
                <div class="instance-panel-row instance-panel-row-meta">
                    <span class="instance-panel-label">Build Time</span>
                    <span class="instance-panel-value instance-panel-mono">{formatUtcTimestamp(about().buildTime!)}</span>
                </div>
            </Show>
        </div>
    );
};

/** The open workspace's heading: its name and window count; double-click or F2 renames it in place. */
export const WorkspaceTitle = (props: { rename: WorkspaceRename; windowCount: () => number }): JSX.Element => {
    const r = props.rename;
    return (
        <div
            class="instance-panel-section-title instance-panel-workspace-title"
            classList={{ "instance-panel-workspace-title-editing": r.editing() }}
            {...tip("The open workspace — double-click to rename (F2)")}
            role="button"
            tabIndex={0}
            onDblClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                if (!r.editing()) r.enter();
            }}
            onKeyDown={(e) => {
                if (r.editing()) return; // the input owns its keys
                if (e.key === "F2") {
                    e.preventDefault();
                    r.enter();
                }
            }}
        >
            <Show when={r.editing()} fallback={workspaceHeading(r.name(), props.windowCount())}>
                <input
                    ref={(el) => {
                        if (el) {
                            queueMicrotask(() => {
                                el.focus();
                                el.select();
                            });
                        }
                    }}
                    class="instance-panel-window-name-input instance-panel-workspace-name-input" size={1} data-raw-text-field="the workspace title's inline rename: it takes the name's place and ends on Enter, Escape or blur, not a form field"
                    aria-label="Workspace name"
                    value={r.value()}
                    maxLength={WORKSPACE_NAME_MAX_LEN}
                    onInput={(e) => r.setValue(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") {
                            e.preventDefault();
                            r.commit();
                        } else if (e.key === "Escape") {
                            e.preventDefault();
                            r.cancel();
                        }
                    }}
                    onBlur={() => r.commit()}
                    onClick={(e) => e.stopPropagation()}
                    onDblClick={(e) => e.stopPropagation()}
                />
            </Show>
        </div>
    );
};

const handleFocusPane = async (label: string) => {
    try {
        await getApi().focusWindow(label);
    } catch (e) {
        console.error("[InstancePanel] focusPane failed:", e);
    }
};

/** The floating panes: each focuses on click, with the same inline opacity control as a window. */
export const FloatingPanesSection = (props: { entries: Accessor<FloatingPaneEntry[]>; OpacityControl: OpacityControlComponent }): JSX.Element => (
    <div class="instance-panel-section">
        <div class="instance-panel-section-title">
            Floating panes — {props.entries().length}
        </div>
        <Show
            when={props.entries().length > 0}
            fallback={
                <div class="instance-panel-pane-empty">No floating panes</div>
            }
        >
            <For each={props.entries()}>
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
                        {...tip("Click to focus")}
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
                        <props.OpacityControl
                            label={entry.label}
                            persistWindowId={null}
                            currentOpacity={() => liveWindowOpacity(entry.label) ?? 1.0}
                        />
                    </div>
                )}
            </For>
        </Show>
    </div>
);

/** "+ Open another window" (a multi-window host only) and Close. */
export const PanelFooter = (props: { onClose: () => void }): JSX.Element => {
    const handleOpenNewWindow = async () => {
        try {
            await openNewWindowFromHere();
        } catch (e) {
            console.error("[InstancePanel] openNewWindow failed:", e);
        }
        props.onClose();
    };
    return (
        <div class="instance-panel-footer">
            <Show when={hostHas("multiWindow")}>
                <Button
                    tone="accent"
                    class="instance-panel-btn instance-panel-btn-primary"
                    onClick={handleOpenNewWindow}
                >
                    + Open another window
                </Button>
            </Show>
            <Button
                class="instance-panel-btn"
                onClick={props.onClose}
            >
                Close
            </Button>
        </div>
    );
};

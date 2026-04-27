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

import { atoms, getApi, openWindowLabelsAtom, windowInstanceNumAtom } from "@/store/global";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

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

    const labels = openWindowLabelsAtom;
    const myInstanceNum = windowInstanceNumAtom;
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi().getWindowLabel().then((l) => setMyLabel(l)).catch(() => setMyLabel(null));

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

    // Display label: "main" → "Window 1", "window-<uuid>" → use the
    // hex prefix as a stable short name. The host doesn't track human-
    // readable per-window names today; the user sees the index +
    // short-id which is enough to disambiguate.
    const displayLabel = (label: string, idx: number): string => {
        if (label === "main") return "Window 1";
        const m = /^window-([0-9a-f]{8})/i.exec(label);
        if (m) return `Window ${idx + 1} · ${m[1]}`;
        return `Window ${idx + 1}`;
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
                    This process — {labels().length} window{labels().length !== 1 ? "s" : ""}
                </div>
                <For each={labels()}>
                    {(label, i) => {
                        const isCurrent = () => label === myLabel();
                        return (
                            <button
                                type="button"
                                class="instance-panel-window-row"
                                classList={{ "instance-panel-window-row-current": isCurrent() }}
                                onClick={() => handleFocusWindow(label)}
                                disabled={isCurrent()}
                                title={isCurrent() ? "This window" : `Focus ${label}`}
                            >
                                <span class="instance-panel-window-dot">{isCurrent() ? "●" : "○"}</span>
                                <span class="instance-panel-window-name">{displayLabel(label, i())}</span>
                                <Show when={isCurrent()}>
                                    <span class="instance-panel-window-badge">this</span>
                                </Show>
                            </button>
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

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
 * The window list is the open workspace's (a named set of windows,
 * SPEC_WORKSPACES_2026_10_23.md §5): headed by its name, renamed by
 * double-click or F2; its open windows, then its closed ones greyed. Each row
 * has one action: an open window's removes it from the workspace (disabled
 * for the only one), a closed window's reopens it; right-click a closed one
 * to remove it.
 *
 * V1 scope: about-info + windows + actions. LAN peers stay in
 * HostPopover (they already have a richer hover-rich detail view).
 * Per-window token totals deferred until token-usage is per-window.
 */

import { getApi, openFloatingPaneEntriesAtom, openWindowEntriesAtom, type WindowEntry } from "@/store/global";
import { MaintenanceSection } from "./MaintenanceSection";
import { hostHas } from "@/util/host-caps";
import { liveWindowOpacity, setWindowOpacityFor } from "@/app/store/window-opacity-store";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { type ClosedWindow } from "@/store/services";
import { removeClosedWindow, reopenClosedWindow, requestRemoveWindow } from "@/app/window/window-close";
import { onCleanup, Show, type JSX } from "solid-js";
import { isHeldWindow, useSlotKeys } from "./window-slot-keys";
import { useClosedWindows } from "./useClosedWindows";
import {
    reconcileWhenLauncherSilent,
    useAboutDetails,
    useWindowIdentity,
    useWindowLabel,
    useWindowRename,
    useWorkspaceRename,
} from "./instance-panel-state";
import { WindowSlotList } from "./InstancePanelRows";
import { AboutSection, FloatingPanesSection, PanelFooter, WorkspaceTitle } from "./InstancePanelSections";

interface InstancePanelProps {
    /** The status bar's version chip. */
    anchor: PopoverAnchor;
    onClose: () => void;
}

const POPOVER_WIDTH = 320;

// Inline opacity control — shares one row with the window/pane name
// (docs/specs/instance-panel-floating-panes.md §3.3). `persistWindowId`
// present → window rows persist to `window:opacity` meta on release;
// null → floating panes, session-only (a floater has no backing window
// object to persist to, and doesn't survive an app restart anyway).
// stopPropagation on all three event kinds so a slider drag/keypress
// never triggers the row's focus or rename handlers.
export const OpacityControl = (p: {
    label: string;
    persistWindowId: string | null;
    currentOpacity: () => number;
}): JSX.Element => (
    // Only a host that can make its windows see-through has opacity to set.
    <Show when={hostHas("windowTransparency")}>
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
                // The hamburger's Opacity sets the same value through
                // the same setter (SPEC_PER_WINDOW_THEME_AND_OPACITY §4.2).
                onInput={(e) =>
                    setWindowOpacityFor(
                        { label: p.label, windowId: p.persistWindowId },
                        parseFloat(e.currentTarget.value),
                        "live",
                    )
                }
                onChange={(e) =>
                    setWindowOpacityFor(
                        { label: p.label, windowId: p.persistWindowId },
                        parseFloat(e.currentTarget.value),
                        "commit",
                    )
                }
            />
            <span class="instance-panel-opacity-value">
                {/* Live store value tracks the drag tick-by-tick; falls back
                    to the row's resolved opacity when the store has no entry
                    yet this session. */}
                {Math.round((liveWindowOpacity(p.label) ?? p.currentOpacity()) * 100)}%
            </span>
        </span>
    </Show>
);

export const InstancePanel = (props: InstancePanelProps): JSX.Element => {
    const about = useAboutDetails();
    const entries = openWindowEntriesAtom;
    const myLabel = useWindowLabel();
    reconcileWhenLauncherSilent();

    const { resolveEntryWindowId, resolveName } = useWindowIdentity(myLabel);
    const { closed, refreshClosed } = useClosedWindows(() => entries().length, () => placeSlots());
    const onlyOpenWindow = () => entries().length <= 1;
    const reopen = (w: ClosedWindow) => {
        if (!isHeldWindow(w)) void reopenClosedWindow(w.workspaceid).then(refreshClosed);
    };
    const removeClosed = (w: ClosedWindow) => {
        if (!isHeldWindow(w)) void removeClosedWindow(w.workspaceid).then(refreshClosed);
    };
    // Asks first when an agent in it is mid-turn (window-close.ts).
    const removeOpen = (entry: WindowEntry) => {
        const windowId = resolveEntryWindowId(entry);
        if (!windowId || onlyOpenWindow()) return;
        void requestRemoveWindow({ windowId, label: entry.label }, () => discard(entry)).then(refreshClosed);
    };

    const windowRename = useWindowRename(entries, resolveEntryWindowId);
    const workspaceRename = useWorkspaceRename();

    const focusWindow = async (label: string) => {
        if (label === myLabel()) return; // already focused
        try {
            await getApi().focusWindow(label);
        } catch (e) {
            console.error("[InstancePanel] focusWindow failed:", e);
        }
    };

    // On unmount, flush any rename still open: StatusBar's outside-click dismiss can unmount the
    // input before its onBlur fires (#569).
    onCleanup(() => {
        workspaceRename.commit();
        windowRename.flush();
    });

    const { openByKey, closedByKey, slotKeys, placeSlots, discard } = useSlotKeys(entries, closed, resolveEntryWindowId, resolveName);
    const rowContext = { OpacityControl, entries, myLabel, rename: windowRename, resolveName, onlyOpenWindow, focusWindow, remove: removeOpen };

    return (
        // Portaled through AnchoredPopover like every other status bar panel.
        // It used to render inside `.status-bar`, inheriting its zoom while
        // placing itself with real-pixel bottom/right, so it drifted off the
        // chip at any zoom but 100% (REPORT_CHROME_ZOOM_POPOVERS_2026_10_02 §4).
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="instance-panel"
            role="dialog"
            aria-label="AgentMux instance panel"
            style={{ width: `${POPOVER_WIDTH}px` }}
        >
            <AboutSection about={about} />
            <div class="instance-panel-divider" />
            <div class="instance-panel-section">
                <WorkspaceTitle rename={workspaceRename} windowCount={() => entries().length + closed().length} />
                <WindowSlotList
                    slotKeys={slotKeys}
                    openByKey={openByKey}
                    closedByKey={closedByKey}
                    context={rowContext}
                    reopen={reopen}
                    removeClosed={removeClosed}
                />
            </div>
            <div class="instance-panel-divider" />
            <FloatingPanesSection entries={openFloatingPaneEntriesAtom} OpacityControl={OpacityControl} />
            <div class="instance-panel-divider" />
            <div class="instance-panel-section">
                <MaintenanceSection />
            </div>
            <div class="instance-panel-divider" />
            <PanelFooter onClose={props.onClose} />
        </AnchoredPopover>
    );
};

InstancePanel.displayName = "InstancePanel";

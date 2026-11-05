// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind the InstancePanel: the About details, this window's label, how a window's id and
 * name are resolved, and the two inline renames (a window's name, the workspace's). The hooks are
 * called in this order from InstancePanel, which is also the order the host is asked for things.
 */

import { atoms, getApi, type WindowEntry } from "@/store/global";
import { reconcileKnownEntriesFromSnapshot } from "@/app/store/launcher-event-store";
import { launcherEventsActive } from "@/util/launcher-events";
import { ObjectService } from "@/store/services";
import { getObjectValue, makeORef } from "@/store/mos";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { DISPLAY_NAME_MAX_LEN, DISPLAY_NAME_META_KEY, resolveWindowName } from "@/util/window-title";
import { renameWorkspace, workspaceName } from "@/app/store/user-workspace";

export interface AboutDetails {
    version: string;
    channel: string | null;
    buildLabel: string | null;
    gitHash: string | null;
    buildTime: number | null;
    cefVersion: string | null;
    platform: string | null;
    arch: string | null;
}

export function useAboutDetails(): Accessor<AboutDetails> {
    return createMemo(() => {
        const d = getApi().getAboutModalDetails();
        return {
            version: d?.version ?? "unknown",
            channel: d?.channel ?? null,
            buildLabel: (d as any)?.buildLabel ?? null,
            gitHash: d?.gitHash ?? null,
            buildTime: typeof d?.buildTime === "number" && d.buildTime > 0 ? d.buildTime : null,
            cefVersion: d?.cefVersion ?? null,
            platform: (d as any)?.platform ?? null,
            arch: (d as any)?.arch ?? null,
        };
    });
}

/** This window's label, once the host has said it; null if it cannot. */
export function useWindowLabel(): Accessor<string | null> {
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi().getWindowLabel().then((l) => setMyLabel(l)).catch(() => setMyLabel(null));
    return myLabel;
}

/**
 * Refresh window-instance state ONLY when the launcher is silent (`task dev` mode — no launcher
 * process, no typed events). In production the launcher pushes WindowOpened/Closed events and
 * `openWindowEntriesAtom` stays in sync; reconciling against a stale `listWindowInstances()`
 * snapshot would clobber a newer launcher event the renderer just applied — the same race
 * protection that motivates ApplySeed at boot (#733). `launcherEventsActive` flips true on the
 * first typed event the renderer receives; if false here, we're in dev mode and the reconcile is
 * the only way windows opened/closed since boot will appear/disappear in the panel.
 */
export function reconcileWhenLauncherSilent(): void {
    if (launcherEventsActive()) return;
    getApi()
        .listWindowInstances()
        .then((snapshot) => {
            // Re-check at resolution time. Between the sync gate above and the snapshot arriving
            // (~ms RPC round-trip), a launcher event may have flipped launcherEventsActive true —
            // applying the now-stale snapshot would clobber that newer state (#733).
            if (launcherEventsActive()) return;
            reconcileKnownEntriesFromSnapshot(snapshot);
        })
        .catch(() => {});
}

export interface WindowIdentity {
    resolveEntryWindowId: (entry: WindowEntry) => string | null;
    resolveName: (entry: WindowEntry, idx: number) => string;
}

export function useWindowIdentity(myLabel: Accessor<string | null>): WindowIdentity {
    // For THIS window's row, fall back to atoms.muxWindow()?.oid when the entry's windowId is
    // still null. WindowEntry.windowId is null for the first ~100ms after a window opens — until
    // the registerBackendWindow IPC round-trip completes (see the `WindowEntry` comment in
    // global.ts). Without this fallback, the panel shows "Window N" for the current window during
    // early-startup while the OS title (which uses initOpts.windowId directly) correctly shows the
    // workspace name — a visible inconsistency.
    const resolveEntryWindowId = (entry: WindowEntry): string | null => {
        if (entry.windowId) return entry.windowId;
        if (entry.label === myLabel()) return atoms.muxWindow()?.oid ?? null;
        return null;
    };

    // Resolve a row's display name via the shared helper so the panel and the OS window title
    // (driven from app-init.ts) agree by construction. Reactive via AgentMux's object
    // subscriptions because getObjectValue reads through atoms — when meta or workspace.name
    // changes, this re-runs.
    const resolveName = (entry: WindowEntry, idx: number): string => {
        let displayName: string | undefined;
        let workspaceName: string | undefined;
        const windowId = resolveEntryWindowId(entry);
        if (windowId) {
            const win = getObjectValue<MuxWindow>(makeORef("window", windowId));
            displayName = win?.meta?.[DISPLAY_NAME_META_KEY] as string | undefined;
            if (win?.workspaceid) {
                const ws = getObjectValue<Workspace>(makeORef("workspace", win.workspaceid));
                workspaceName = ws?.name;
            }
        }
        const name = resolveWindowName({ displayName, workspaceName, indexInOpenWindows: idx });

        // Diagnostic — for this window's row only, log the same shape as [wave-title] in
        // app-init.ts. If both surfaces resolve the same window with different `name` values, the
        // inputs disagree and the bug is the inputs (typically: idx mismatch from the
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
    return { resolveEntryWindowId, resolveName };
}

export interface WindowRename {
    /** The label of the row being renamed (keyed by host label so the row's identity survives reordering); one at a time. */
    editingLabel: Accessor<string | null>;
    editValue: Accessor<string>;
    setEditValue: (v: string) => void;
    enterRename: (entry: WindowEntry, currentName: string) => void;
    cancelRename: () => void;
    commitRename: (entry: WindowEntry) => void;
    /** Save a rename still open as the panel goes away. */
    flush: () => void;
}

export function useWindowRename(
    entries: Accessor<WindowEntry[]>,
    resolveEntryWindowId: (entry: WindowEntry) => string | null,
): WindowRename {
    const [editingLabel, setEditingLabel] = createSignal<string | null>(null);
    const [editValue, setEditValue] = createSignal("");

    const enterRename = (entry: WindowEntry, currentName: string) => {
        // Same fallback as resolveName: for this window's row we can use atoms.muxWindow()?.oid
        // when the entry's windowId hasn't been populated yet via the registerBackendWindow round-trip.
        if (!resolveEntryWindowId(entry)) return;
        setEditingLabel(entry.label);
        setEditValue(currentName);
    };

    const cancelRename = () => {
        setEditingLabel(null);
        setEditValue("");
    };

    // The one persistence path, used by both commitRename (Enter / blur) and the unmount flush, so
    // the trim + cap + RPC + error-log shape stays consistent.
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

    // StatusBar's outside-click dismiss can unmount the input before its onBlur fires, which would
    // otherwise silently lose the typed value (#569).
    const flush = () => {
        const editing = editingLabel();
        if (!editing) return;
        const entry = entries().find((e) => e.label === editing);
        if (!entry) return;
        const windowId = resolveEntryWindowId(entry);
        if (!windowId) return;
        const trimmed = editValue().trim().slice(0, DISPLAY_NAME_MAX_LEN);
        if (!trimmed) return;
        performRename(windowId, trimmed);
    };
    return { editingLabel, editValue, setEditValue, enterRename, cancelRename, commitRename, flush };
}

export interface WorkspaceRename {
    name: () => string;
    editing: Accessor<boolean>;
    value: Accessor<string>;
    setValue: (v: string) => void;
    enter: () => void;
    cancel: () => void;
    commit: () => void;
}

/** The workspace heading's rename: one at a time, like a window's. */
export function useWorkspaceRename(): WorkspaceRename {
    const name = () => workspaceName(atoms.client());
    const [editing, setEditing] = createSignal(false);
    const [value, setValue] = createSignal("");
    const enter = () => {
        setValue(name());
        setEditing(true);
    };
    const cancel = () => {
        setEditing(false);
        setValue("");
    };
    // Enter or blur. Blank is no change (as for a window's name).
    const commit = () => {
        if (!editing()) return;
        const v = value();
        cancel();
        if (v.trim() === name()) return;
        renameWorkspace(v).catch((e) => console.error("[InstancePanel] workspace rename failed:", e));
    };
    return { name, editing, value, setValue, enter, cancel, commit };
}

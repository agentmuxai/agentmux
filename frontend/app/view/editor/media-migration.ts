// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A saved Media pane, opened as an Editor (the `media` view is an alias of
 * the Editor's, editor.tsx): its meta rewritten once into an Editor's, so its
 * files reopen as Editor tabs that follow their folder, as Media did.
 * - Media's `doctabs` tabs saved a plain path each (`serialize: d => d.path`);
 *   they become Editor buffers `{path, followFolder}`. Empty tabs go.
 * - `media:path` (a pane from before tabs, or opened on a file) becomes the
 *   Editor's `file`, following its folder.
 * - Pending `media:open` requests move to `editor:pending_open_files`.
 * Pure: the model applies the patch, and writes it back once mounted.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.2.
 */

import { canonicalizePath } from "@/app/store/editor-pane-state-store";

const MEDIA_PATH = "media:path";
const MEDIA_OPEN = "media:open";
const DOC_TABS = "doctabs";

interface SavedTab {
    key?: unknown;
    title?: unknown;
    icon?: unknown;
    pinned?: unknown;
    state?: unknown;
}

function baseName(path: string): string {
    return path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1) || path;
}

/** The meta patch that makes a saved Media pane an Editor, or null when the
 *  meta isn't a Media pane's. Keys set to null are removed. */
export function migrateMediaMeta(meta: Record<string, unknown> | undefined): Record<string, unknown> | null {
    if (!meta) return null;
    const isMedia = meta.view === "media" || MEDIA_PATH in meta || MEDIA_OPEN in meta;
    if (!isMedia) return null;
    const patch: Record<string, unknown> = { view: "editor", [MEDIA_PATH]: null, [MEDIA_OPEN]: null };

    // Doc tabs: a Media tab's state is its path, a string.
    const record = meta[DOC_TABS] as { v?: unknown; tabs?: unknown; active?: unknown } | null | undefined;
    let tabs = 0;
    if (record && record.v === 1 && Array.isArray(record.tabs)) {
        const out: SavedTab[] = [];
        let active = 0;
        (record.tabs as SavedTab[]).forEach((t, i) => {
            if (!t || typeof t.state !== "string" || !t.state) {
                // Already an Editor tab (a half-done migration): keep it.
                if (t && t.state && typeof t.state === "object") {
                    if (i === record.active) active = out.length;
                    out.push(t);
                }
                return;
            }
            if (i === record.active) active = out.length;
            out.push({
                key: canonicalizePath(t.state),
                title: baseName(t.state),
                ...(typeof t.icon === "string" ? { icon: t.icon } : {}),
                ...(t.pinned ? { pinned: true } : {}),
                state: { path: t.state, followFolder: true },
            });
        });
        tabs = out.length;
        patch[DOC_TABS] = out.length > 0 ? { ...record, tabs: out, active } : null;
    }

    // A pane from before tabs: its one file.
    const path = meta[MEDIA_PATH];
    if (tabs === 0 && typeof path === "string" && path) {
        patch.file = path;
        patch["editor:follow_folder"] = true;
    }

    // Files sent here before it mounted.
    const queued = meta[MEDIA_OPEN];
    if (Array.isArray(queued)) {
        const pending = Array.isArray(meta["editor:pending_open_files"]) ? [...(meta["editor:pending_open_files"] as unknown[])] : [];
        for (const r of queued) {
            const p = (r as { path?: unknown } | null)?.path;
            if (typeof p === "string" && p) pending.push({ path: p, follow_folder: true });
        }
        if (pending.length > 0) patch["editor:pending_open_files"] = pending;
    }

    // Media had no file tree: the migrated pane starts without one too.
    if (meta["editor:tree_expanded"] === undefined) patch["editor:tree_expanded"] = false;
    return patch;
}

/** `meta` with `patch` applied (null removes a key). */
export function applyMetaPatch(meta: Record<string, unknown> | undefined, patch: Record<string, unknown>): Record<string, unknown> {
    const out: Record<string, unknown> = { ...(meta ?? {}) };
    for (const [k, v] of Object.entries(patch)) {
        if (v === null) delete out[k];
        else out[k] = v;
    }
    return out;
}

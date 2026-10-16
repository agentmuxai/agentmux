// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's per-tab viewer state that isn't in the tab itself: srv's
 * probe of each tab's file (what the viewers and "Reopen with…" read), a
 * revision bumped when a non-text tab's file changes on disk (its viewer
 * fetches again), and dropped files with no host path. Kept out of
 * editor-model.ts, which holds the tabs.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.1, §4.2.
 */

import { AUDIO_EXTENSIONS, basenameOf, dirnameOf, extOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createSignal } from "solid-js";
import { viewerOptions, type FileProbe, type ViewerId } from "./viewer-choice";

/** Probes and revisions, by tab id. Not persisted: a restore probes again. */
export class ViewerTabs {
    private readonly probes = new Map<string, FileProbe>();
    private readonly revisions = new Map<string, number>();
    private readonly version = createSignal(0);

    private bump(): void {
        this.version[1]((v) => v + 1);
    }

    probe(tabId: string | null): FileProbe | null {
        void this.version[0]();
        return tabId ? (this.probes.get(tabId) ?? null) : null;
    }

    setProbe(tabId: string, probe: FileProbe): void {
        this.probes.set(tabId, probe);
        this.bump();
    }

    revision(tabId: string | null): number {
        void this.version[0]();
        return tabId ? (this.revisions.get(tabId) ?? 0) : 0;
    }

    /** The tab's file changed: its viewer fetches it again. */
    bumpRevision(tabId: string): void {
        this.revisions.set(tabId, (this.revisions.get(tabId) ?? 0) + 1);
        this.bump();
    }

    /** The viewers the tab can be shown in, its own first. */
    options(tabId: string | null, current: ViewerId): ViewerId[] {
        const probe = this.probe(tabId);
        return probe ? viewerOptions(probe) : [current];
    }

    forget(tabId: string): void {
        this.probes.delete(tabId);
        this.revisions.delete(tabId);
    }
}

// ── Dropped files with no host path ────────────────────────────────────
// Their bytes are kept for the session (any Editor pane can show one, and a
// tab can move between panes), and gone after a restart, as the Media
// pane's were.

const DROPPED_PREFIX = "agentmux-dropped:/";
const droppedFiles = new Map<string, File>();

export function isDroppedPath(path: string): boolean {
    return path.startsWith(DROPPED_PREFIX);
}

/** The bytes behind a dropped file's tab path. */
export function droppedFileOf(path: string): File | undefined {
    return droppedFiles.get(path);
}

/** Keep a dropped file's bytes; returns the tab path that names them. */
export function rememberDropped(file: File): string {
    const path = `${DROPPED_PREFIX}${crypto.randomUUID()}/${file.name || "file"}`;
    droppedFiles.set(path, file);
    return path;
}

/** `[` / `]`: the previous or next file of the same sort (images, or
 *  audio and video) in `path`'s folder, by name; null when there is none. */
export async function neighbourInFolder(path: string, viewer: ViewerId, delta: -1 | 1): Promise<string | null> {
    const dir = dirnameOf(path);
    if (!dir) return null;
    const exts = viewer === "image" ? IMAGE_EXTENSIONS : [...VIDEO_EXTENSIONS, ...AUDIO_EXTENSIONS];
    const listing = await RpcApi.ListEditorDirCommand(TabRpcClient, { path: dir });
    const names = listing.entries
        .filter((e) => !e.is_dir && exts.includes(extOf(e.name)))
        .map((e) => e.name)
        .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
    if (names.length < 2) return null;
    const at = Math.max(0, names.findIndex((n) => n.toLowerCase() === basenameOf(path).toLowerCase()));
    return `${dir}/${names[(((at + delta) % names.length) + names.length) % names.length]}`;
}

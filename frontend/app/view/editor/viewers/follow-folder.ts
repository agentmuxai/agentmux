// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follow the folder: while on, the file's folder is watched for image, video
 * and audio files, and a newer one replaces the tab's file (a fresh render
 * landing in an agent's output folder), or the same file written again
 * reloads. The retired Media pane's behaviour, now a toggle
 * on the Editor's image and media viewers.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.9.
 */

import { AUDIO_EXTENSIONS, dirnameOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";
import { makeORef } from "@/app/store/mos";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";
import { createEffect, onCleanup, type Accessor } from "solid-js";

/** What the watched folder reports: image, video and audio files. */
export const FOLLOWED_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS, ...AUDIO_EXTENSIONS];

/** Whether two paths name the same file, whatever their separators and the
 *  drive letter's case. */
export function samePath(a: string, b: string): boolean {
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/^([a-z]):/i, (_, d: string) => d.toLowerCase() + ":");
    return norm(a) === norm(b);
}

export function createFollowFolder(opts: {
    blockId: string;
    /** The file the tab shows ("" for none). */
    path: Accessor<string>;
    enabled: Accessor<boolean>;
    /** A newer matching file arrived: the tab shows it now. */
    onNewer: (path: string) => void;
    /** The tab's own file was written again. */
    onRewritten: () => void;
}): void {
    let watchedDir: string | null = null;
    let unsub: () => void = () => {};

    const stop = () => {
        if (watchedDir != null) {
            const dir = watchedDir;
            fireAndForget(() => RpcApi.UnwatchMediaDirCommand(TabRpcClient, { path: dir, block_id: opts.blockId }));
            watchedDir = null;
        }
        unsub();
        unsub = () => {};
    };

    createEffect(() => {
        const path = opts.path();
        const dir = opts.enabled() && path ? dirnameOf(path) : "";
        if (dir === (watchedDir ?? "")) return;
        stop();
        if (!dir) return;
        watchedDir = dir;
        fireAndForget(() =>
            RpcApi.WatchMediaDirCommand(TabRpcClient, { path: dir, block_id: opts.blockId, extensions: FOLLOWED_EXTENSIONS })
        );
        unsub = onMuxEvent(
            WpsEvent.MediaFileChanged,
            (data) => {
                const changed = data.path;
                if (!changed || !opts.enabled()) return;
                if (samePath(changed, opts.path())) opts.onRewritten();
                else opts.onNewer(changed);
            },
            makeORef("block", opts.blockId)
        );
    });

    onCleanup(stop);
}

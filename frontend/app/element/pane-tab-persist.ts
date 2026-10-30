// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Writes a tab's last real name and favicon to its block's meta, so a tab reads the same before and
 * after it is clicked, and after a reload (`describePaneTab` reads them back). One write per tab at
 * most every `PERSIST_DELAY_MS`: a page whose title changes every second does not write every second.
 */

import { setBlockMeta } from "@/app/store/block-meta";
import { fireAndForget } from "@/util/util";
import { TAB_FAVICON_META, TAB_NAME_META, type PaneTabInfo } from "./pane-tab-model";

export const PERSIST_DELAY_MS = 1500;

type Write = (blockId: string, meta: Record<string, string>) => unknown;

const pending = new Map<string, { patch: Record<string, string>; timer: ReturnType<typeof setTimeout> }>();

/** Queue the meta write `info.persist` asks for; none asked cancels one that is queued. */
export function persistTabInfo(info: PaneTabInfo, write: Write = (id, meta) => setBlockMeta(id, meta as MetaType)): void {
    const { blockId, persist } = info;
    const patch: Record<string, string> = {};
    if (persist?.name) patch[TAB_NAME_META] = persist.name;
    if (persist?.favicon) patch[TAB_FAVICON_META] = persist.favicon;
    const queued = pending.get(blockId);
    if (Object.keys(patch).length === 0) {
        if (queued) {
            clearTimeout(queued.timer);
            pending.delete(blockId);
        }
        return;
    }
    if (queued) {
        queued.patch = patch; // the newest value goes out when the timer already running fires
        return;
    }
    const entry = {
        patch,
        timer: setTimeout(() => {
            pending.delete(blockId);
            fireAndForget(async () => void (await write(blockId, entry.patch)));
        }, PERSIST_DELAY_MS),
    };
    pending.set(blockId, entry);
}

/** For tests: drop every queued write. */
export function resetTabPersistForTests(): void {
    for (const { timer } of pending.values()) clearTimeout(timer);
    pending.clear();
}

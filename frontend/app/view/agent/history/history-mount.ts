// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The History reader's start and end: subscribe to the transcript file, poll it, load the newest page,
 * and let go of all of it. Split out of AgentHistoryView.
 */

import { getFileSubject } from "@/app/store/mps";
import { onCleanup, onMount } from "solid-js";
import type { TranscriptFileEvent } from "../transcript-cursor";
import { startHistoryPoll } from "./history-poll";
import type { HistoryReader } from "./history-reader-state";

export function useHistoryMount(s: HistoryReader): void {
    onMount(() => {
        const fileSubject = getFileSubject(s.readBlockId, "output");
        const subscription = fileSubject.subscribe((ev: TranscriptFileEvent) => {
            if (s.hidden() && s.cursor?.isSettled()) {
                s.missedWhileHidden = true;
                if (ev.fileop !== "append") s.resetWhileHidden = true;
                return;
            }
            s.cursor?.push(ev);
        });

        const stopPoll = startHistoryPoll({
            readBlockId: s.readBlockId,
            cursor: () => s.cursor,
            hidden: s.hidden,
            reloadIfSkipped: s.act.reloadIfSkipped,
            recover: s.act.recover,
        });

        onCleanup(() => {
            s.disposed = true;
            stopPoll();
            subscription.unsubscribe();
            fileSubject.release();
            s.cursor?.dispose();
            s.cursor = null;
        });
        void s.act.load();
    });
}

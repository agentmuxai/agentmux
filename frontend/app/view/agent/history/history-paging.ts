// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The History reader paging backwards: older pages from the list, from "Show older" and from a wheel
 * turned up at the top, and filling a pane the page doesn't fill. Split out of AgentHistoryView.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { batch } from "solid-js";
import { FILL_MAX_PAGES, HISTORY_PAGE, type HistoryReader } from "./history-reader-state";

export function useHistoryPaging(s: HistoryReader): void {
    const loadOlder = async (): Promise<void> => {
        const currentOffset = s.historyOffset();
        if (currentOffset === 0 || s.loadingOlder()) return;
        const seq = s.loadSeq;
        s.setLoadingOlder(true);
        try {
            const newOffset = Math.max(0, currentOffset - HISTORY_PAGE);
            const resp = await RpcApi.BlockfileReadRangeCommand(
                TabRpcClient,
                { block_id: s.readBlockId, filename: "output", offset: newOffset, limit: currentOffset - newOffset, ...s.src() },
                { timeout: 15_000 }
            );
            // A reload started meanwhile: this page belongs to the old view.
            if (seq !== s.loadSeq || s.disposed) return;
            s.accLines = [...(resp.lines ?? []), ...s.accLines];
            s.accStamps = [...s.stampsOf(resp), ...s.accStamps];
            // Rebuild the parser over the whole range; records appended from
            // now on continue from this one.
            s.parser = s.newParser(newOffset);
            s.parser.feed(s.accLines, s.accStamps);
            batch(() => {
                s.act.publishNow();
                s.setHistoryOffset(newOffset);
            });
        } catch {
            // Non-fatal — the affordance stays and the user can retry by
            // scrolling again; same soft-fail posture as the live pane.
        } finally {
            s.setLoadingOlder(false);
        }
    };

    const scroller = (): HTMLElement | null => s.rootEl?.querySelector<HTMLElement>(".agent-document") ?? null;
    const nextFrame = (): Promise<void> => new Promise((r) => requestAnimationFrame(() => r()));

    /**
     * Load older pages while what is shown doesn't fill the pane. Without
     * this, a short page has nothing to scroll, so scrolling up never asks
     * for more and the reader is stuck on it.
     */
    const fillPane = async (seq: number): Promise<void> => {
        for (let i = 0; i < FILL_MAX_PAGES; i++) {
            await nextFrame();
            await nextFrame();
            const el = scroller();
            if (seq !== s.loadSeq || s.disposed || !el || s.historyOffset() === 0) return;
            // A pane with no height (a hidden tab) has nothing to fill yet.
            if (el.clientHeight === 0 || el.scrollHeight > el.clientHeight) return;
            await loadOlder();
        }
    };

    /**
     * An older page, keeping what the reader was looking at in place: the
     * page lands above it, and the scroll position moves down by its height.
     * For "Show older" and a wheel turned up at the very top, where the list
     * sees no scroll and asks for nothing.
     */
    const showOlder = async (): Promise<void> => {
        const el = scroller();
        const fromBottom = el ? el.scrollHeight - el.scrollTop : null;
        await loadOlder();
        await nextFrame();
        const after = scroller();
        if (after && fromBottom !== null) after.scrollTop = Math.max(0, after.scrollHeight - fromBottom);
    };
    const onWheel = (e: WheelEvent): void => {
        const el = scroller();
        if (e.deltaY < 0 && el && el.scrollTop <= 0 && s.historyOffset() > 0 && !s.loadingOlder()) void showOlder();
    };
    Object.assign(s.act, { loadOlder, fillPane, showOlder, onWheel });
}

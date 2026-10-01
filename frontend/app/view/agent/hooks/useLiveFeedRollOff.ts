// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 7).

import { snapshot as layoutSnapshot } from "@/app/store/agent-pane-layout-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { getSettingsKeyAtom } from "@/app/store/global";
import { batch, createEffect, createMemo, createSignal, on, onCleanup, untrack, type Accessor } from "solid-js";
import { feedOverLimits, liveFeedSupported, resolveLiveFeedTurns, visibleIdsOf } from "../live-feed";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readToolResult, type ToolResultLoader } from "../tool-result-loader";
import type { AgentAtoms } from "../state";
import { userIsInteracting } from "../stream-scheduler";

export function useLiveFeedRollOff(opts: {
    blockId: string;
    paneModel: AgentPaneModel;
    outputFormat: Accessor<string>;
    block: Accessor<Block | undefined>;
    agentAtoms: () => AgentAtoms;
    hidden: Accessor<boolean>;
    history: { historyOffset: Accessor<number> };
}) {
    const { paneModel, outputFormat, block, agentAtoms, hidden, history } = opts;
    // ---- The live feed (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.9) ----
    // The pane keeps the turn in flight plus the last K finished turns;
    // older ones roll off into History, which follows the transcript. Read
    // once at mount, like agent:turnscopedtail. Providers whose transcript
    // lacks the user's messages keep today's behaviour (`liveFeedSupported`).
    const liveFeedSetting = untrack(() => getSettingsKeyAtom("agent:livefeed")()) !== false;
    const liveFeedTurns = resolveLiveFeedTurns(untrack(() => getSettingsKeyAtom("agent:livefeedturns")()));
    const liveFeedOn = (): boolean =>
        liveFeedSetting && liveFeedSupported(outputFormat(), block()?.meta?.["controller"] as string | undefined);
    // Whether the reader follows the bottom — handed over by the document view.
    // Held in a signal so the backstop below re-runs when it's handed over.
    const [followingBottomSrc, setFollowingBottomSrc] = createSignal<Accessor<boolean>>(() => true);
    const followingBottom = (): boolean => followingBottomSrc()();
    const setFollowingBottom = (f: Accessor<boolean>): void => {
        setFollowingBottomSrc(() => f);
    };
    // Turns rolled off the front since mount, and the gap rows between kept turns.
    const [rolledOffTurns, setRolledOffTurns] = createSignal(0);
    const [gapsBefore, setGapsBefore] = createSignal<ReadonlySet<string>>(new Set());
    let rollOffDisposed = false;
    onCleanup(() => {
        rollOffDisposed = true;
    });

    /** One roll-off pass: a single reducer command, planned on its current nodes. */
    const runRollOff = (): void => {
        if (rollOffDisposed) return;
        const [docState, setDocState] = agentAtoms().documentStateAtom;
        const pinnedIds = untrack(docState).pinnedNodes;
        // Collapsed tool results leave memory in the same idle pass; rows the
        // user pinned or that are held open keep theirs
        // (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.3). This
        // runs with the live feed off too (Codex P2 on #4126).
        paneModel.dispatchDoc({
            type: "UnloadToolResults",
            keepIds: new Set([...pinnedIds, ...untrack(docState).expandedTools]),
        });
        if (!liveFeedOn()) return;
        const events = paneModel.dispatchDoc({
            type: "RollOff",
            keepTurns: liveFeedTurns,
            visibleIds: visibleIdsOf(layoutSnapshot(opts.blockId)),
            keepIds: pinnedIds,
            pinned: untrack(followingBottom),
        });
        const ev = events.find((e) => e.type === "turns-rolled-off");
        if (!ev || ev.type !== "turns-rolled-off") return;
        const present = new Set(untrack(paneModel.document).map((n) => n.id));
        const prune = (set: Set<string>): Set<string> => {
            let changed = false;
            const next = new Set<string>();
            for (const id of set) {
                if (present.has(id)) next.add(id);
                else changed = true;
            }
            return changed ? next : set;
        };
        batch(() => {
            setRolledOffTurns((n) => n + ev.prefixTurns);
            setGapsBefore((prev) => {
                const next = new Set<string>();
                for (const id of prev) if (present.has(id)) next.add(id);
                for (const id of ev.gapsBefore) next.add(id);
                return next;
            });
            // The view's own id sets must not keep ids that are gone.
            setDocState((prev) => {
                const collapsedNodes = prune(prev.collapsedNodes);
                const expandedTools = prune(prev.expandedTools);
                const pinnedNodes = prune(prev.pinnedNodes);
                return collapsedNodes === prev.collapsedNodes &&
                    expandedTools === prev.expandedTools &&
                    pinnedNodes === prev.pinnedNodes
                    ? prev
                    : { ...prev, collapsedNodes, expandedTools, pinnedNodes };
            });
        });
        if (ev.blockedTurns > 0) {
            console.debug(`[live-feed] ${opts.blockId}: ${ev.blockedTurns} older turn(s) kept (not in the transcript)`);
        }
    };

    /**
     * Schedule a pass off the input path: when the browser is idle, stepping
     * aside while the user types, but never later than ROLL_OFF_DEADLINE_MS —
     * a deferred pass is re-queued, not dropped (§6.9).
     */
    const ROLL_OFF_DEADLINE_MS = 1_000;
    let rollOffQueued = false;
    const whenIdle = (cb: () => void, timeoutMs: number): void => {
        const ric = (globalThis as { requestIdleCallback?: (cb: () => void, o: { timeout: number }) => number })
            .requestIdleCallback;
        if (ric) ric(cb, { timeout: Math.max(1, timeoutMs) });
        else setTimeout(cb, Math.min(50, Math.max(0, timeoutMs)));
    };
    function scheduleRollOff(): void {
        if (rollOffQueued) return;
        rollOffQueued = true;
        const deadline = performance.now() + ROLL_OFF_DEADLINE_MS;
        const attempt = (): void => {
            if (rollOffDisposed) return;
            const left = deadline - performance.now();
            if (left > 0 && userIsInteracting()) return whenIdle(attempt, left);
            rollOffQueued = false;
            runRollOff();
        };
        whenIdle(attempt, ROLL_OFF_DEADLINE_MS);
    }

    // Backstop for paths that add many turns at once without a turn end or a
    // send (a restore, a large history load): a pass whenever the feed first
    // holds clearly more turns than it keeps. A memo, so it fires on the
    // transition, not on every flush. A pass made while the reader was up in
    // older rows may keep them all, so it runs again once they're back at the
    // bottom.
    const feedOverBudget = createMemo(() => liveFeedOn() && feedOverLimits(paneModel.document(), liveFeedTurns));
    createEffect(
        on(feedOverBudget, (over) => {
            if (over) scheduleRollOff();
        })
    );
    createEffect(
        on(
            followingBottom,
            (atBottom) => {
                if (atBottom && untrack(feedOverBudget)) scheduleRollOff();
            },
            { defer: true }
        )
    );

    // Roll-off points besides the history load and turn end (below): the next
    // send, and the pane going out of view.
    createEffect(
        on(
            () => {
                const doc = paneModel.document();
                const last = doc[doc.length - 1];
                return last?.type === "user_message" ? last.id : null;
            },
            (id) => {
                if (id) scheduleRollOff();
            },
            { defer: true }
        )
    );
    createEffect(
        on(
            hidden,
            (hidden) => {
                if (hidden) scheduleRollOff();
            },
            { defer: true }
        )
    );

    // Scrolling up pages older lines in while the pane's range is contiguous:
    // once turns have rolled off the front, the lines just before the loaded
    // range no longer join what's on screen, so History takes over.
    const canPageOlder = (): boolean => !liveFeedOn() || rolledOffTurns() === 0;
    // A new session's divider no longer hides what came before it, so only
    // roll-off sends earlier turns to History.
    const earlierHistoryAvailable = createMemo(() => liveFeedOn() && rolledOffTurns() > 0);
    // "N earlier turns" only when N is the whole story: everything before the
    // feed was loaded from line 0 and rolled off here.
    const earlierTurnsKnown = (): number | undefined =>
        liveFeedOn() && rolledOffTurns() > 0 && history.historyOffset() === 0
            ? rolledOffTurns()
            : undefined;

    /** Reads an unloaded tool result back from its transcript line (§3.4). */
    const loadToolResult: ToolResultLoader = async (node) => {
        const result = await readToolResult(node, outputFormat(), (offset) =>
            RpcApi.BlockfileReadRangeCommand(TabRpcClient, { block_id: opts.blockId, filename: "output", offset, limit: 1 }, { timeout: 15_000 }),
        );
        if (!result) return false;
        paneModel.dispatchDoc({ type: "ResultLoaded", nodeId: node.id, result });
        return true;
    };

    return {
        liveFeedOn,
        liveFeedTurns,
        loadToolResult,
        canPageOlder,
        scheduleRollOff,
        gapsBefore,
        earlierHistoryAvailable,
        earlierTurnsKnown,
        setFollowingBottom,
    };
}

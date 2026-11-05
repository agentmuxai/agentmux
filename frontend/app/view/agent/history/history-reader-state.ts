// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the History reader keeps: the signals it shows, the raw lines and the one parser that has
 * consumed them, the cursor and load counters, and the actions its parts call on each other. Split out
 * of AgentHistoryView; the hook groups (history-*.ts) are called in the order their effects were set up.
 */

import {
    registerPane as registerLayoutPane,
    unregisterPane as unregisterLayoutPane,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import { createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import type { DocumentNode, DocumentState, FilterState } from "../../../agent-model/types";
import { HistoryParser } from "../parseHistoryLines";
import type { TranscriptCursor } from "../transcript-cursor";
import { injectDayDividers } from "./day-dividers";
import { shortWhen, type HistoryMode } from "./HistoryNavigator";

/** Lines per fetch. Most transcript lines are streaming fragments, so 600
 *  lines came to about a dozen rows, less than a screen; still bounded well
 *  under the backend's 10k per-request cap. */
export const HISTORY_PAGE = 3_000;

/** Older pages loaded on their own while what is shown doesn't fill the pane. */
export const FILL_MAX_PAGES = 8;

/** Appended records are shown at most this often (spec §6.9). */
export const HISTORY_PUBLISH_MIN_INTERVAL_MS = 1_000;

const HISTORY_FILTER: FilterState = {
    // The reader shows the honest full record — thinking included is a
    // deliberate divergence from the live default (readers came here to
    // peruse everything; the filter bar is not mounted in history mode).
    showThinking: true,
    showSuccessfulTools: true,
    showFailedTools: true,
    showIncoming: true,
    showOutgoing: true,
};

export interface AgentHistoryViewProps {
    /** This reader's OWN block id — used only as the layout-slot identity
     *  (`${blockId}:history`) so concurrently-open history tabs (even for
     *  the same agent) never collide on layout state. NOT used for the
     *  transcript reads — see `sourceBlockId`. */
    blockId: string;
    /**
     * The block id whose meta resolves the agent's GLOBAL transcript zone
     * for the RPC reads (`agent:session:read`'s backend, `global_output_source`
     * in blockfile.rs, keyed off `block.meta.agentId`). Must be a block that
     * has actually run the agent — when the global zone lookup itself comes
     * up empty, the backend falls back to reading that exact block's own
     * LOCAL per-channel output, so a never-launched block (like the history
     * tab's own, per `openOrFocusHistoryTab`) would silently read as empty
     * in that fallback case even though the live conversation has full
     * local data. Defaults to `blockId` for standalone/test callers that
     * don't distinguish the two. codex P1 on PR #2539.
     */
    sourceBlockId?: string;
    /**
     * A request to open at a position (`agent:historyAt` on the reader's block, see history-start-at.ts):
     * applied once per request, then `onStartAtApplied` asks for it to be cleared.
     */
    startAt?: Accessor<unknown>;
    onStartAtApplied?: () => void;
    outputFormat: Accessor<string>;
    agentName?: Accessor<string>;
    /**
     * Live per-pane zoom factor (the same CSS `zoom` applied on the
     * `.agent-view` ancestor). Forwarded to `AgentDocumentView` /
     * `AgentDocumentVirtualList`, which needs it to normalize the ONE
     * zoomed read the measure `ResizeObserver` takes
     * (`getBoundingClientRect().height`) back into unzoomed CSS px — see
     * that component's own doc comment
     * (SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01). Without
     * this, any zoom level other than 1 records zoomed heights as if they
     * were unzoomed, producing overlapping rows (zoom < 1) or oversized
     * gaps and wrong scroll math (zoom > 1). codex P1 on PR #2542.
     */
    zoomFactor?: Accessor<number>;
    /** The agent's definition id: lists its archived conversations and sessions. */
    agentId?: string;
}

/** What the reader's parts call on each other: each is made by one group and read by the others. */
export interface HistoryActions {
    publishNow(): void;
    publishSoon(): void;
    recover(): void;
    reloadIfSkipped(): void;
    appendRecords(text: string): void;
    newCursor(): TranscriptCursor;
    jumpToLatest(): Promise<void>;
    load(): Promise<void>;
    loadStatic(end?: number): Promise<void>;
    loadOlder(): Promise<void>;
    fillPane(seq: number): Promise<void>;
    showOlder(): Promise<void>;
    loadNewer(): Promise<void>;
    backToLatest(): Promise<void>;
    openArchive(zone: string | null): void;
    jumpTo(line: number): void;
    onWheel(e: WheelEvent): void;
}

export type HistoryReader = ReturnType<typeof createHistoryReader>;

/** What the reader shows and how far it has read: the mode, the lines loaded, the loading and error flags. */
function createReadSignals(props: AgentHistoryViewProps) {
    // What the reader shows (SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md phase 2):
    // "live" follows the current conversation; "jumped" shows it up to a chosen
    // line; "archive" shows a conversation archived by "New conversation". Only
    // "live" follows new records. `endLine` is where what is loaded ends.
    const [mode, setMode] = createSignal<HistoryMode>("live");
    const [archiveZone, setArchiveZone] = createSignal<string | null>(null);
    const [endLine, setEndLine] = createSignal(0);
    const src = (): { archive_zone?: string } => {
        const zone = untrack(archiveZone);
        return zone ? { archive_zone: zone } : {};
    };

    // The accumulated RAW LINES (+ parallel stamps) of everything this
    // reader shows, and ONE resumable parser that has consumed exactly them.
    // One parser — not one per page with an id dedup — because
    // ClaudeCodeStreamParser generates counter-based ids (node_0, user_0, …)
    // that restart for every parser instance: two independently-parsed
    // pages collide on ids for UNRELATED messages, and any per-page dedup
    // silently drops real history (codex P1 on PR #2509). Appended records
    // go through the same parser (O(new lines), open text runs continue);
    // an older page rebuilds it over the whole accumulated range — O(loaded
    // lines) per page-up, bounded by the reader's own pagination.
    const [rawNodes, setRawNodes] = createSignal<DocumentNode[]>([]);
    // Ids are built from each node's own transcript line (`idFirstLine`), so
    // a node keeps its id when an older page rebuilds the parser: the scroll
    // anchor finds it again, and its row (with an open hover panel) is kept
    // rather than replaced under the pointer.
    const newParser = (firstLine: number): HistoryParser =>
        new HistoryParser(props.outputFormat(), props.agentName?.(), { includeResumedOutcomes: true, idFirstLine: firstLine });
    const [historyOffset, setHistoryOffset] = createSignal(0);
    const [totalLines, setTotalLines] = createSignal(0);
    const [loading, setLoading] = createSignal(true);
    const [loadingOlder, setLoadingOlder] = createSignal(false);
    const [loadError, setLoadError] = createSignal<string | null>(null);
    return {
        mode, setMode,
        archiveZone, setArchiveZone,
        endLine, setEndLine,
        src,
        rawNodes, setRawNodes,
        newParser,
        historyOffset, setHistoryOffset,
        totalLines, setTotalLines,
        loading, setLoading,
        loadingOlder, setLoadingOlder,
        loadError, setLoadError,
    };
}

/** What the views draw from the raw nodes: the document with day dividers, the span they cover, and the reused view's atoms and layout slot. */
function createShownDocument(rawNodes: Accessor<DocumentNode[]>, blockId: string) {
    // The rendered document: raw nodes + day separators, recomputed as a
    // pure pass (§4.4 — dividers are render-time synthetics).
    const displayNodes = createMemo(() => injectDayDividers(rawNodes()));

    // When the rows on screen were received: what a reader looks for, where
    // the transcript line numbers say little (most lines are streaming pieces,
    // so 3,000 lines come to a few dozen rows).
    const shownSpan = createMemo((): string => {
        const times = rawNodes()
            .map((n) => (n as { timestamp?: number }).timestamp)
            .filter((t): t is number => typeof t === "number" && t > 0);
        if (times.length === 0) return "";
        const first = shortWhen(times.reduce((m, t) => Math.min(m, t)));
        const last = shortWhen(times.reduce((m, t) => Math.max(m, t)));
        return first === last ? first : `${first} – ${last}`;
    });
    const withSpan = (text: string): string => (shownSpan() ? `${shownSpan()} · ${text}` : text);

    // Local document atoms for the reused view components. The document
    // atom is projected from displayNodes; DocumentState is this reader's
    // own expansion/pin state, independent of the live pane's.
    const [docNodes, setDocNodes] = createSignal<DocumentNode[]>([]);
    createEffect(() => setDocNodes(displayNodes()));
    const [docState, setDocState] = createSignal<DocumentState>({
        collapsedNodes: new Set<string>(),
        pinnedNodes: new Set<string>(),
        heldOpenNodes: new Set<string>(),
        scrollPosition: 0,
        selectedNode: null,
        filter: HISTORY_FILTER,
    });

    // Own layout slot under a synthetic key so the live pane's slot (keyed
    // by the real blockId) is untouched. Required for the virtual list to
    // position rows at all.
    const layoutKey = `${blockId}:history`;
    const [layoutView, setLayoutView] = createSignal<LayoutView | null>(null);
    registerLayoutPane(layoutKey, { layout: setLayoutView, zoom: () => {} });
    onCleanup(() => unregisterLayoutPane(layoutKey));
    return { withSpan, docNodes, docState, setDocState, layoutKey, layoutView };
}

export function createHistoryReader(props: AgentHistoryViewProps) {
    // See the doc comment on `sourceBlockId` above — this is the id every
    // RPC read targets. `blockId` itself is reserved for the layout key only.
    const readBlockId = props.sourceBlockId ?? props.blockId;
    const read = createReadSignals(props);
    const shown = createShownDocument(read.rawNodes, props.blockId);

    const stampsOf = (resp: { lines?: string[]; stamps?: number[] }): (number | undefined)[] =>
        resp.stamps ?? new Array((resp.lines ?? []).length).fill(undefined);

    // Whether the list follows the bottom, as it reports; and a reader who left it after a hole
    // has stale lines (spec §6.9).
    const [followingSource, setFollowingSource] = createSignal<Accessor<boolean>>(() => true);
    const followingSignal = (): boolean => followingSource()();
    const following: Accessor<boolean> = followingSignal;
    const [stale, setStale] = createSignal(false);

    return {
        props,
        readBlockId,
        ...read,
        ...shown,
        stampsOf,
        followingSource, setFollowingSource, followingSignal, following,
        stale, setStale,
        // What changes as the reader runs. The accumulated lines, with a parser that has consumed exactly them:
        accLines: [] as string[],
        accStamps: [] as (number | undefined)[],
        parser: read.newParser(0),
        // The publishing timer and what it is waiting to show:
        dirty: false,
        lastPublishAt: 0,
        publishTimer: undefined as ReturnType<typeof setTimeout> | undefined,
        // The cursor following the transcript, and the counters that tell a stale answer from a current one:
        disposed: false,
        cursor: null as TranscriptCursor | null,
        loadSeq: 0,
        skippedSeen: 0,
        genChangesSeen: 0,
        // What arrived while the tab was hidden:
        missedWhileHidden: false,
        resetWhileHidden: false,
        // Set by the list and the root element as they are drawn:
        scrollToBottom: undefined as ((reason?: string) => void) | undefined,
        rootEl: undefined as HTMLDivElement | undefined,
        // Made by the groups: whether the tab is hidden, and the actions the parts call on each other.
        hidden: (() => false) as Accessor<boolean>,
        act: {} as HistoryActions,
    };
}

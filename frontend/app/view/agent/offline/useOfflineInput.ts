// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * {@link createOfflineInput} and {@link createFailedTurnRecovery} for an
 * agent pane: the turn's API retry from srv's `agent-api-retry`, the
 * provider's reachability from the connectivity store, held messages and the
 * failed turn's latest retry in the block's meta, and the
 * `agent:retryonreconnect` setting (default on).
 *
 * Held input lives here, in the pane, rather than in srv: the pane's send
 * path (slash commands, runtime args, the login guards, the busy queue) is
 * the one a held message has to take, and srv has no copy of it. The cost is
 * that a held message goes out only while its pane is open; it stays in the
 * block's meta until then, so closing the window or quitting loses nothing.
 */

import { type Accessor, createEffect, createSignal, onCleanup, type Setter, untrack } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { setBlockMeta } from "@/app/store/block-meta";
import { connectivity, loadConnectivity } from "@/app/store/connectivity";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getBlockMetaKeyAtom, getSettingsKeyAtom } from "@/app/store/global";
import { registerRecycleBlocker } from "@/app/store/recycle-blockers";
import { clockTime } from "@/store/muxbus-delivery";
import { Logger } from "@/util/logger";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { useApiRetryState } from "../hooks/useApiRetryState";
import type { ProviderDefinition } from "../providers";
import {
    createFailedTurnRecovery,
    type FailedTurnRecovery,
    type RetryMark,
    type RetryMarkStorage,
    type TurnStarted,
} from "./failed-turn";
import {
    confirmedSend,
    createOfflineInput,
    type HeldInput,
    isTrackedService,
    keyedLock,
    type OfflineInput,
    type OfflineStorage,
    paneModelApiService,
    type PaneMessageEvent,
    serviceReachability,
} from "./offline-input";

export { lastUserMessage } from "./failed-turn";

export const HELD_INPUT_META_KEY = "agent:held_input";
export const RETRY_MARK_META_KEY = "agent:retried_turn";
/** Where srv says this pane's model API is, when it is not the provider's default address. */
const API_ENDPOINT_META_KEY = "agent:api_endpoint";
export const RETRY_ON_RECONNECT_SETTING = "agent:retryonreconnect";

/**
 * Each block's held messages, read from its meta once and then kept here:
 * the meta atom catches up only after a write's round trip, and a pane
 * mounted in between must not see a held message that already went. One
 * signal per block, so every mount of a pane sees the same list.
 */
const heldByBlock = new Map<string, { agentId: string | undefined; get: Accessor<HeldInput[]>; set: Setter<HeldInput[]> }>();
/** The same for each block's failed-turn retry: a remount mid-retry sees it on its way. */
const markByBlock = new Map<string, { agentId: string | undefined; get: Accessor<RetryMark | null>; set: Setter<RetryMark | null> }>();
const locks = new Map<string, Promise<unknown>>();

function writeMeta(blockId: string, meta: Record<string, unknown>) {
    setBlockMeta(blockId, meta).catch((e) => Logger.warn("offline-input", "failed to save held input", { blockId, error: String(e) }));
}

/**
 * Held messages are stored with the agent they were typed to, so a pane
 * switched to another agent never sends them there.
 */
export function blockMetaStorage(blockId: string, agentId: () => string | undefined): OfflineStorage {
    const cell = () => {
        const id = untrack(agentId);
        const cached = heldByBlock.get(blockId);
        if (cached?.agentId === id) return cached;
        const stored = untrack(() => getBlockMetaKeyAtom(blockId, HELD_INPUT_META_KEY)());
        const own = stored && stored.agentId === id && Array.isArray(stored.messages) ? stored.messages : [];
        const [get, set] = createSignal<HeldInput[]>(own);
        const made = { agentId: id, get, set };
        heldByBlock.set(blockId, made);
        return made;
    };
    return {
        held() {
            return cell().get();
        },
        saveHeld(held) {
            const id = agentId();
            cell().set(held);
            writeMeta(blockId, { [HELD_INPUT_META_KEY]: held.length > 0 && id ? { agentId: id, messages: held } : null });
        },
    };
}

/**
 * The failed turn's latest retry, stored with its agent like held input.
 * Read from the meta, one stored as on its way is from an earlier load,
 * which can't be sending it any more: its answer never came.
 */
export function blockRetryMarkStorage(blockId: string, agentId: () => string | undefined): RetryMarkStorage {
    const cell = () => {
        const id = untrack(agentId);
        const cached = markByBlock.get(blockId);
        if (cached?.agentId === id) return cached;
        const stored = untrack(() => getBlockMetaKeyAtom(blockId, RETRY_MARK_META_KEY)());
        let own: RetryMark | null = null;
        if (stored && stored.agentId === id && typeof stored.text === "string") {
            const { agentId: _agent, ...mark } = stored;
            own = mark.state === "sending" ? { ...mark, state: "unknown" } : mark;
        }
        const [get, set] = createSignal<RetryMark | null>(own);
        const made = { agentId: id, get, set };
        markByBlock.set(blockId, made);
        return made;
    };
    return {
        mark() {
            return cell().get();
        },
        saveMark(mark) {
            const id = agentId();
            cell().set(mark);
            writeMeta(blockId, { [RETRY_MARK_META_KEY]: mark && id ? { agentId: id, ...mark } : null });
        },
    };
}

export function useOfflineInput(opts: {
    blockId: string;
    paneModel: AgentPaneModel;
    provider: () => ProviderDefinition | undefined;
    agentId: () => string | undefined;
    send: (text: string, attachments?: AttachmentRef[]) => Promise<void>;
    stop: () => void;
}): OfflineInput & { apiRetry: ReturnType<typeof useApiRetryState>; recovery: FailedTurnRecovery } {
    const apiRetry = useApiRetryState(opts.blockId);
    if (connectivity() === null) void loadConnectivity();
    const storage = blockMetaStorage(opts.blockId, opts.agentId);
    const state = opts.paneModel.state;
    const endpoint = getBlockMetaKeyAtom(opts.blockId, API_ENDPOINT_META_KEY);
    const service = () => paneModelApiService(opts.provider()?.supportedVendors, endpoint());
    const api = () => serviceReachability(connectivity()?.services, service());
    const working = () => workingFromPhase(state.turnPhase);
    const ready = () => state.initPhase.kind !== "InitPending";
    // The pane's own events, and srv's acceptance, which still comes after
    // the pane has given up on a message (its entry expired).
    const listen = (cb: (e: PaneMessageEvent) => void) => {
        const offPane = addPaneEventListener((blockId, event) => blockId === opts.blockId && cb(event as PaneMessageEvent));
        const offSrv = onMuxEvent(
            WpsEvent.AgentMessageAccepted,
            (d) => cb({ type: "pending-accepted", id: d.message_id }),
            `block:${opts.blockId}`
        );
        return () => {
            offPane();
            offSrv();
        };
    };
    const deliver = confirmedSend(opts.send, listen, working);
    const lock = keyedLock(locks, opts.blockId);
    const offline = createOfflineInput({
        retrying: () => apiRetry() !== null,
        api,
        ready,
        working,
        document: () => opts.paneModel.document(),
        send: opts.send,
        deliver,
        stop: opts.stop,
        storage,
        lock,
    });
    // The last turn this mount started: only the pane's own TurnStart enters
    // Submitting, and the pane's state is new with each mount.
    const [started, setStarted] = createSignal<TurnStarted | null>(null);
    createEffect(() => {
        const phase = state.turnPhase;
        if (phase.kind === "Submitting") setStarted({ at: phase.submittedAt, content: phase.pendingContent, hidden: !!phase.hidden });
    });
    const recovery = createFailedTurnRecovery({
        failure: () => state.failure,
        started,
        tracked: () => isTrackedService(service(), connectivity()?.services),
        api,
        retrying: () => apiRetry() !== null,
        ready,
        working,
        held: () => storage.held().length,
        document: () => opts.paneModel.document(),
        autoRetryOn: () => getSettingsKeyAtom(RETRY_ON_RECONNECT_SETTING)() !== false,
        deliver,
        restoreFailure: (f) =>
            opts.paneModel.dispatchPane({ type: "FailureObserved", failure: f.data, at: f.at, turnAttempted: f.turnAttempted }, "system"),
        clearFailure: () => opts.paneModel.dispatchPane({ type: "FailureCleared" }, "system"),
        storage: blockRetryMarkStorage(opts.blockId, opts.agentId),
        lock,
        clockTime,
    });
    const unsub = addPaneEventListener((blockId, event) => {
        if (blockId === opts.blockId && event.type === "turn-ended" && event.outcome === "completed") recovery.turnCompleted();
    });
    onCleanup(unsub);
    onCleanup(registerRecycleBlocker(() => (recovery.pending() ? "a failed turn is to be retried when the API is back" : null)));
    return {
        ...offline,
        notices: () => [...offline.notices(), ...recovery.notices()],
        dismissNotice: (id) => {
            offline.dismissNotice(id);
            recovery.dismissNotice(id);
        },
        submit: (text, attachments) => {
            recovery.typed();
            return offline.submit(text, attachments);
        },
        apiRetry,
        recovery,
    };
}

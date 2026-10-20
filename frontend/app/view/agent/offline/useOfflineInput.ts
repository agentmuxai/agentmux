// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * {@link createOfflineInput} for an agent pane: the turn's API retry from
 * srv's `agent-api-retry`, the provider's reachability from the connectivity
 * store, held messages and the automatic-retry marker in the block's meta,
 * and the `agent:retryonreconnect` setting (default on).
 *
 * Held input lives here, in the pane, rather than in srv: the pane's send
 * path (slash commands, runtime args, the login guards, the busy queue) is
 * the one a held message has to take, and srv has no copy of it. The cost is
 * that a held message goes out only while its pane is open; it stays in the
 * block's meta until then, so closing the window or quitting loses nothing.
 */

import { onCleanup, untrack } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { setBlockMeta } from "@/app/store/block-meta";
import { connectivity, loadConnectivity } from "@/app/store/connectivity";
import { getBlockMetaKeyAtom, getSettingsKeyAtom } from "@/app/store/global";
import { clockTime } from "@/store/muxbus-delivery";
import { Logger } from "@/util/logger";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { useApiRetryState } from "../hooks/useApiRetryState";
import type { ProviderDefinition } from "../providers";
import {
    type AutoRetryMarker,
    createOfflineInput,
    type HeldInput,
    keyedLock,
    modelApiService,
    type OfflineInput,
    type OfflineStorage,
    serviceReachability,
} from "./offline-input";

export const HELD_INPUT_META_KEY = "agent:held_input";
export const AUTO_RETRY_META_KEY = "agent:auto_retried_turn";
export const AUTO_RETRY_SETTING = "agent:retryonreconnect";

/**
 * What this renderer last wrote, per block: the meta atom catches up only
 * after the write's round trip, and a pane mounted in between must not see
 * a held message that already went.
 */
const heldByBlock = new Map<string, { agentId: string | undefined; messages: HeldInput[] }>();
const locks = new Map<string, Promise<unknown>>();

function writeMeta(blockId: string, meta: Record<string, unknown>) {
    setBlockMeta(blockId, meta).catch((e) => Logger.warn("offline-input", "failed to save held input", { blockId, error: String(e) }));
}

/**
 * Held messages are stored with the agent they were typed to, so a pane
 * switched to another agent never sends them there.
 */
export function blockMetaStorage(blockId: string, agentId: () => string | undefined): OfflineStorage {
    return {
        loadHeld() {
            const id = agentId();
            const cached = heldByBlock.get(blockId);
            if (cached) return cached.agentId === id ? cached.messages : [];
            const stored = untrack(() => getBlockMetaKeyAtom(blockId, HELD_INPUT_META_KEY)());
            return stored && stored.agentId === id && Array.isArray(stored.messages) ? stored.messages : [];
        },
        saveHeld(held) {
            const id = agentId();
            heldByBlock.set(blockId, { agentId: id, messages: held });
            writeMeta(blockId, { [HELD_INPUT_META_KEY]: held.length > 0 && id ? { agentId: id, messages: held } : null });
        },
        loadMarker() {
            return (getBlockMetaKeyAtom(blockId, AUTO_RETRY_META_KEY)() as AutoRetryMarker | undefined) ?? null;
        },
        saveMarker(marker) {
            writeMeta(blockId, { [AUTO_RETRY_META_KEY]: marker });
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
}): OfflineInput & { apiRetry: ReturnType<typeof useApiRetryState> } {
    const apiRetry = useApiRetryState(opts.blockId);
    if (connectivity() === null) void loadConnectivity();
    const storage = blockMetaStorage(opts.blockId, opts.agentId);
    const state = opts.paneModel.state;
    const offline = createOfflineInput({
        retrying: () => apiRetry() !== null,
        api: () => serviceReachability(connectivity()?.services, modelApiService(opts.provider()?.supportedVendors)),
        ready: () => state.initPhase.kind !== "InitPending",
        working: () => workingFromPhase(state.turnPhase),
        failure: () => state.failure,
        document: () => opts.paneModel.document(),
        autoRetryOn: () => getSettingsKeyAtom(AUTO_RETRY_SETTING)() !== false,
        send: opts.send,
        stop: opts.stop,
        storage,
        lock: keyedLock(locks, opts.blockId),
        clockTime,
    });
    // A turn that completed is done with its automatic retry.
    const unsub = addPaneEventListener((blockId, event) => {
        if (blockId !== opts.blockId || event.type !== "turn-ended" || event.outcome !== "completed") return;
        if (storage.loadMarker()) storage.saveMarker(null);
    });
    onCleanup(unsub);
    return { ...offline, apiRetry };
}

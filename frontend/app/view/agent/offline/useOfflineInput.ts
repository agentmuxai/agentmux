// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * {@link createOfflineInput} for an agent pane: the turn's API retry from
 * srv's `agent-api-retry`, the provider's reachability from the connectivity
 * store, and held messages in the block's meta.
 *
 * Held input lives here, in the pane, rather than in srv: the pane's send
 * path (slash commands, runtime args, the login guards, the busy queue) is
 * the one a held message has to take, and srv has no copy of it. The cost is
 * that a held message goes out only while its pane is open; it stays in the
 * block's meta until then, so closing the window or quitting loses nothing.
 */

import { type Accessor, createSignal, type Setter, untrack } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { addEventListener as addPaneEventListener } from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import { setBlockMeta } from "@/app/store/block-meta";
import { connectivity, loadConnectivity } from "@/app/store/connectivity";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { Logger } from "@/util/logger";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { useApiRetryState } from "../hooks/useApiRetryState";
import type { ProviderDefinition } from "../providers";
import {
    confirmedSend,
    createOfflineInput,
    type HeldInput,
    isTrackedService,
    keyedLock,
    modelApiService,
    type OfflineInput,
    type OfflineStorage,
    type PaneMessageEvent,
    serviceReachability,
} from "./offline-input";

export const HELD_INPUT_META_KEY = "agent:held_input";

/**
 * Each block's held messages, read from its meta once and then kept here:
 * the meta atom catches up only after a write's round trip, and a pane
 * mounted in between must not see a held message that already went. One
 * signal per block, so every mount of a pane sees the same list.
 */
const heldByBlock = new Map<string, { agentId: string | undefined; get: Accessor<HeldInput[]>; set: Setter<HeldInput[]> }>();
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
    const service = () => modelApiService(opts.provider()?.supportedVendors);
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
    const offline = createOfflineInput({
        retrying: () => apiRetry() !== null,
        api: () => serviceReachability(connectivity()?.services, service()),
        tracked: () => isTrackedService(service(), connectivity()?.services),
        statusKnown: () => connectivity() !== null,
        ready: () => state.initPhase.kind !== "InitPending",
        working: () => workingFromPhase(state.turnPhase),
        document: () => opts.paneModel.document(),
        send: opts.send,
        deliver: confirmedSend(opts.send, listen, () => workingFromPhase(state.turnPhase)),
        stop: opts.stop,
        storage,
        lock: keyedLock(locks, opts.blockId),
    });
    return { ...offline, apiRetry };
}

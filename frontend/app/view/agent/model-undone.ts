// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A model change srv undid because the new place refused the conversation on its first turn: block meta
 * `agent:model_switch_undone` (`crates/srv/src/backend/model_gateway/trial.rs`). The pane says what was tried, why it was
 * refused and where the agent is again, and offers to compact the conversation and try again.
 */

import * as MOS from "@/app/store/mos";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** Block meta key srv writes the card under; absent when no change was undone. */
export const MODEL_UNDONE_META_KEY = "agent:model_switch_undone";

export interface UndonePlace {
    /** The route's id; empty on the harness's own sign-in. */
    route: string;
    model: string;
    label: string;
}

export interface ModelUndone {
    /** When it was undone, in ms. */
    at: number;
    tried: UndonePlace;
    backOn: UndonePlace;
    /** The HTTP status the place refused with. */
    status: number;
    reason: string;
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

function readPlace(raw: unknown): UndonePlace | null {
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    const model = str(r.model);
    const label = str(r.label) || model;
    return label ? { route: str(r.route), model, label } : null;
}

/** The pane's undone-change card, or null (absent, null, or not the shape srv writes). */
export function readModelUndone(meta: Record<string, unknown> | null | undefined): ModelUndone | null {
    const raw = meta?.[MODEL_UNDONE_META_KEY];
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    const tried = readPlace(r.tried);
    const backOn = readPlace(r.back_on);
    if (!tried || !backOn) return null;
    return {
        at: typeof r.at === "number" ? r.at : 0,
        tried,
        backOn,
        status: typeof r.status === "number" ? r.status : 0,
        reason: str(r.reason),
    };
}

/** `DeepSeek V4 Pro refused this conversation (400); you are back on DeepSeek V4.1 Flash`. */
export function modelUndoneText(card: ModelUndone): string {
    const status = card.status > 0 ? ` (${card.status})` : "";
    return `${card.tried.label} refused this conversation${status}; you are back on ${card.backOn.label}`;
}

/** Clears the card from the pane. Best effort: a card that stays is dismissed again. */
export async function dismissModelUndone(blockId: string): Promise<void> {
    try {
        await RpcApi.SetMetaCommand(TabRpcClient, { oref: MOS.makeORef("block", blockId), meta: { [MODEL_UNDONE_META_KEY]: null } });
    } catch {
        // Best effort.
    }
}

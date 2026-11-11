// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model panel's routes: every model of every route that serves the agent's harness, beside the harness's own
 * models, and the pick that moves the agent there (docs/specs/SPEC_AGENT_MODEL_ROUTES_ANY_MODEL_SPAWN_AND_SWITCH_2026_11_06.md
 * section 7a). srv decides what can be picked and why not (`agent.model.panel`) and does the move (`agent.model.switch`);
 * this builds the rows from its answer.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentModelPanel } from "@/types/rpc/AgentModelPanel";
import type { AgentModelPanelModel } from "@/types/rpc/AgentModelPanelModel";
import type { AgentModelSwitchResult } from "@/types/rpc/AgentModelSwitchResult";
import { createSignal } from "solid-js";
import type { AgentRuntimeDropupProps, OptionRow } from "./runtime-dropup-options";

/** The pane meta key srv writes at every launch with the model that really answers; absent off a route. */
export const ROUTE_MODEL_META_KEY = "agent:route_model";
const SWITCH_TIMEOUT_MS = 120_000;

const ATTACHMENT_NODES: ReadonlySet<string> = new Set(["user_message", "jekt_message", "system_note"]);

/** How many images the conversation holds: those the user attached, a jekt carried, or an "Earlier message" note lists. A text-only model reads none; other files reach it as text. */
export function countImages(nodes: ReadonlyArray<{ type: string; attachments?: ReadonlyArray<{ image?: boolean }> }>): number {
    return nodes.reduce(
        (n, node) => n + (ATTACHMENT_NODES.has(node.type) ? (node.attachments?.filter((a) => a.image).length ?? 0) : 0),
        0
    );
}

/** The row value of "go back to where the agent was". */
export const BACK_VALUE = "back";
const ROUTE_PREFIX = "route:";
const COMPACT_PREFIX = "compact:";

export interface PaneRouteModel {
    route: string;
    route_label: string;
    model: string;
    label: string;
}

/** The route and model the pane's agent runs on, or null on the harness's own sign-in. */
export function routeModelOf(meta: Record<string, unknown> | undefined): PaneRouteModel | null {
    const v = meta?.[ROUTE_MODEL_META_KEY] as Partial<PaneRouteModel> | null | undefined;
    return v && typeof v.route === "string" && typeof v.model === "string" && typeof v.label === "string"
        ? { route: v.route, route_label: v.route_label ?? v.route, model: v.model, label: v.label }
        : null;
}

/** A route model's row value. A route id has no colon, so the model id is whatever follows the second one. */
export const routeValue = (route: string, model: string) => `${ROUTE_PREFIX}${route}:${model}`;

/** The row value of a model picked by compacting the conversation first. */
export const compactValue = (route: string, model: string) => `${COMPACT_PREFIX}${route}:${model}`;

/** The route and model a row value names, or null for a harness model or the Back row. */
export function parseRouteValue(value: string): { route: string; model: string } | null {
    return parsePrefixed(value, ROUTE_PREFIX);
}

/** The route and model a "Compact, then switch" row value names, or null. */
export function parseCompactValue(value: string): { route: string; model: string } | null {
    return parsePrefixed(value, COMPACT_PREFIX);
}

function parsePrefixed(value: string, prefix: string): { route: string; model: string } | null {
    if (!value.startsWith(prefix)) return null;
    const rest = value.slice(prefix.length);
    const i = rest.indexOf(":");
    return i > 0 ? { route: rest.slice(0, i), model: rest.slice(i + 1) } : null;
}

const money = (usd: number) => (usd >= 1 ? `$${usd.toFixed(2)}` : `$${usd.toFixed(usd >= 0.1 ? 2 : 3)}`);
const tokens = (n: number) => (n >= 1_000_000 ? `${+(n / 1_000_000).toFixed(1)}M` : `${Math.round(n / 1000)}K`);

/** What a route model's row says under its name: the reason it cannot be picked, else its size, price and what it does. */
export function routeModelNote(m: AgentModelPanelModel, conversationTokens: number | null | undefined): string {
    if (m.unavailable) return m.unavailable;
    if (m.compact_first) return `${m.compact_first}: compacts it first, then switches`;
    const abilities = [m.tools && "tools", m.thinking && "thinking", m.images && "images"].filter(Boolean).join(", ");
    const line = `${tokens(m.context)} context · ${money(m.price_in)} in / ${money(m.price_out)} out per million${abilities ? ` · ${abilities}` : ""}`;
    return !m.current && m.reread_cost_usd != null && conversationTokens
        ? `${line} · about ${money(m.reread_cost_usd)} to re-read ${tokens(conversationTokens)} on this model`
        : line;
}

/** The option rows of one route, as the Model section shows them. */
export function routeRows(panel: AgentModelPanel, conversationTokens: number | null | undefined) {
    return panel.routes.map((route) => ({
        route,
        rows: route.models.map(
            (m): OptionRow => ({
                section: "model",
                value: m.compact_first ? compactValue(route.id, m.id) : routeValue(route.id, m.id),
                label: m.compact_first ? `${m.label} · Compact, then switch` : m.label,
                description: routeModelNote(m, conversationTokens),
                detail: m.unavailable ?? undefined,
                current: m.current,
                disabled: m.unavailable != null,
            })
        ),
    }));
}

/** The Back row: where the agent was before its last change. */
export function backRow(panel: AgentModelPanel): OptionRow | null {
    const prev = panel.previous;
    if (!prev) return null;
    const name = prev.route === "" ? "the harness's own model" : prev.label;
    // Back is a switch like any other, so it is held to the same checks as that model's own row (a key
    // gone, images the model cannot read): srv's reason for the model it goes back to.
    const reason = panel.routes.find((r) => r.id === prev.route)?.models.find((m) => m.id === prev.model)?.unavailable ?? null;
    return {
        section: "model",
        value: BACK_VALUE,
        label: `Back to ${name}`,
        description: reason ?? (prev.route === "" ? "your plan's model, as before the last change" : `${prev.route} route`),
        detail: reason ?? undefined,
        current: false,
        disabled: reason != null,
    };
}

/** What the Model section says when srv's routes could not be read, so the harness's own models alone are not taken for "no routes". */
export const readFailureNote = (reason: string) => `Other models: couldn't be read: ${reason}`;

function reasonOf(e: unknown): string {
    const text = (e instanceof Error ? e.message : String(e)).replace(/^FORBIDDEN:\s*/, "").trim();
    return text.length > 140 ? `${text.slice(0, 139)}\u2026` : text || "no reason given";
}

/** The panel's answer for the pane's agent, fetched when the panel opens, and the pick that moves the agent. */
export function createRoutePanel(props: AgentRuntimeDropupProps) {
    const [panel, setPanel] = createSignal<AgentModelPanel | null>(null);
    const [failure, setFailure] = createSignal<string | null>(null);

    const refresh = async () => {
        if (props.providerId !== "claude" || !props.blockId) return;
        try {
            setPanel(
                await RpcApi.AgentModelPanelCommand(TabRpcClient, {
                    blockid: props.blockId,
                    conversation_tokens: props.contextTokens ?? null,
                    images: props.conversationImages?.() ?? null,
                })
            );
            setFailure(null);
        } catch (e) {
            // The panel shows the harness's own models only, and says why the others are missing.
            console.warn("model panel: the routes could not be read", e);
            setPanel(null);
            setFailure(reasonOf(e));
        }
    };

    /** Moves the agent to `route`/`model` (an empty route is the harness's own sign-in). The agent restarts after its turn. */
    const moveTo = async (route: string, model: string, restart: boolean): Promise<AgentModelSwitchResult> => {
        // srv counts the conversation's images before it moves, which reads a big transcript whole the first time: more than the default 5 s.
        const result = await RpcApi.AgentModelSwitchCommand(TabRpcClient, { blockid: props.blockId, route, model, restart }, { timeout: SWITCH_TIMEOUT_MS });
        await refresh();
        return result;
    };

    /** Whether srv has the agent on a route: the saved choice, which a running pane only adopts at its next launch. */
    const onSavedRoute = () => panel()?.routes.some((r) => r.models.some((m) => m.current)) ?? false;

    return { panel, failure, refresh, moveTo, onSavedRoute };
}

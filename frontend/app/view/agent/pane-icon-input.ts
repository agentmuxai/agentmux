// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the agent pane's header icon shows, worked out from the pane's meta: the harness as the main icon (fixed for
 * the agent's life) and the model actually answering as the badge. The header reads it through a memo that
 * compares by value, so the icon is built again only when one of these changes (a model switched in place), not on
 * every meta write.
 */

import type { AgentDefinition } from "@/app/store/rpc-api";

import { PROVIDERS, resolveEffectiveVendor, resolveProviderAlias } from "./providers/catalog";
import { modelMakerLogo, readRouteModel, routeModelText } from "./route-model";

export interface PaneIconInput {
    /** The harness: the main icon. */
    harness: string;
    /** The badge's logo id: the maker of the route's model, or off a route today's resolution (`resolveEffectiveVendor`). */
    vendor: string;
    /** Names the model in the tooltip ("DeepSeek V4.1 Flash via OpenRouter"); absent off a route. */
    modelText?: string;
}

/** The harness the pane was launched with: a definition's provider (an alias is resolved), or a quick launch's agent id. */
function paneHarness(meta: Record<string, unknown>): string | null {
    const provider = meta["agentProvider"];
    if (typeof provider === "string" && provider.length > 0) return resolveProviderAlias(provider);
    const agentId = meta["agentId"];
    return typeof agentId === "string" && PROVIDERS[agentId] ? agentId : null;
}

/** The pane's icon input, or null when no agent has launched into it (the header falls back to `agentIcon`). */
export function paneIconInput(
    meta: Record<string, unknown> | null | undefined,
    definitions: readonly AgentDefinition[]
): PaneIconInput | null {
    if (!meta) return null;
    const harness = paneHarness(meta);
    if (!harness) return null;
    const routed = readRouteModel(meta);
    if (routed) return { harness, vendor: modelMakerLogo(routed), modelText: routeModelText(routed) };
    const definition = definitions.find((d) => d.id === meta["agentId"]);
    return { harness, vendor: resolveEffectiveVendor(harness, definition?.model_vendor_base_url) };
}

/** Value equality, for the memo: a meta write that changed none of these builds no new icon. */
export function samePaneIconInput(a: PaneIconInput | null, b: PaneIconInput | null): boolean {
    if (a == null || b == null) return a === b;
    return a.harness === b.harness && a.vendor === b.vendor && a.modelText === b.modelText;
}

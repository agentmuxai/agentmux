// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model an agent really runs, as srv records it on the pane: block meta `agent:route_model`
 * (`crates/srv/src/backend/model_gateway/launch.rs`). Absent or null on the harness's own sign-in, where the harness's
 * model is what answers. The pane header reads it for its model badge; the composer footer reads the same field.
 */

/** Block meta key srv writes the route and model under. */
export const ROUTE_MODEL_META_KEY = "agent:route_model";

export interface RouteModel {
    /** The route's id ("openrouter"). */
    route: string;
    routeLabel: string;
    /** The model's id on the route ("deepseek/deepseek-v4.1-flash-1017"). */
    model: string;
    /** The model's name for a person ("DeepSeek V4.1 Flash"). */
    label: string;
}

const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** The pane's route model, or null off a route (absent, null, or not the shape srv writes). */
export function readRouteModel(meta: Record<string, unknown> | null | undefined): RouteModel | null {
    const raw = meta?.[ROUTE_MODEL_META_KEY];
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    const model = str(r.model);
    if (!model) return null;
    const route = str(r.route);
    return { route, routeLabel: str(r.route_label) || route, model, label: str(r.label) || model };
}

/** Model-id prefixes (OpenRouter's maker names) whose mark ProviderLogo knows under another id. */
const MAKER_LOGO: Record<string, string> = { moonshotai: "moonshot", "z-ai": "zhipu", "x-ai": "xai", "meta-llama": "meta" };

/**
 * The logo id of the maker of a route model: the part of its id before the slash ("deepseek/…" is DeepSeek).
 * A maker with no mark of its own shows its first letter, as any unknown provider does. An id with no maker in it
 * gives the route's id.
 */
export function modelMakerLogo(model: RouteModel): string {
    const slash = model.model.indexOf("/");
    const maker = slash > 0 ? model.model.slice(0, slash).toLowerCase() : model.route;
    return MAKER_LOGO[maker] ?? maker;
}

/** `DeepSeek V4.1 Flash via OpenRouter`: the model and where it is served from, for a tooltip. */
export function routeModelText(model: RouteModel): string {
    return model.routeLabel ? `${model.label} via ${model.routeLabel}` : model.label;
}

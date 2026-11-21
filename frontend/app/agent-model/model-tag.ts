// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm's model tag: what an agent is running now, as a short label with a family colour and a glyph
 * (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §4). Pure: the facts in (the runtime row's `model_label`,
 * `model_source` and `pending_switch`), the words, hue, glyph and tooltip lines out. A model nobody knows gives no tag:
 * nothing is guessed.
 */

import type { ModelHue } from "@/app/element/ui/tag-tokens";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";

/**
 * How sure the runtime is of the model (#48222's `model_source`). `route` is an agent on a model route: its gateway's
 * model, shown as sure as `effective`.
 */
export type ModelSource = "effective" | "observed" | "route" | "requested" | "unknown";

export type ModelFamily = "fable" | "opus" | "sonnet" | "haiku" | "gpt" | "gemini" | "other";

/** The most characters a tag's words take; the full label is in the tooltip. */
export const MODEL_TAG_MAX_CHARS = 16;

export interface ModelFacts {
    /** srv's short display label ("Sonnet 5.5"), when it has one. */
    label?: string | null;
    /** The model id, for a build that sends no label. */
    id?: string | null;
    /** The agent's provider ("claude", "codex", "gemini", a route), for a family the label does not name. */
    provider?: string | null;
    source?: ModelSource | string | null;
    /** The model a switch just asked for, waiting for the next turn boundary. */
    pendingTo?: string | null;
    /** What the agent's definition says, when that differs from what runs. */
    definitionLabel?: string | null;
    effort?: string | null;
}

export interface ModelTag {
    /** The tag's words, cut to `MODEL_TAG_MAX_CHARS`. */
    text: string;
    family: ModelFamily;
    hue: ModelHue;
    /** Asked for and not confirmed, or a switch pending: drawn muted. */
    muted: boolean;
    /** A Font Awesome solid glyph name: a different shape per family group, so colour is never the only cue. */
    glyph: string;
    /** The tooltip, a line each. */
    lines: string[];
}

const FAMILY_HUE: Record<ModelFamily, ModelHue> = {
    fable: "model-fable",
    opus: "model-opus",
    sonnet: "model-sonnet",
    haiku: "model-haiku",
    gpt: "model-gpt",
    gemini: "model-gemini",
    other: "model-other",
};

const CLAUDE_FAMILIES: readonly ModelFamily[] = ["fable", "opus", "sonnet", "haiku"];

/** A spark for the Claude tiers, a hexagon of nodes for OpenAI, a diamond for Gemini, a dot for the rest. */
function glyphOf(family: ModelFamily): string {
    if (CLAUDE_FAMILIES.includes(family)) return "star-of-life";
    if (family === "gpt") return "hexagon-nodes";
    if (family === "gemini") return "diamond";
    return "circle";
}

const clean = (v: string | null | undefined): string => (typeof v === "string" ? v.trim() : "");

/** "claude-sonnet-5-5" is "Sonnet 5.5"; a date or `-latest` on the end and a `[1m]` suffix are dropped. Anything else as written. */
export function labelFromId(id: string): string {
    const bare = id.replace(/\[[^\]]*\]$/, "").trim();
    const m = /^claude-([a-z]+)-(\d+(?:-\d{1,2})*)(?:-\d{8}|-latest)?$/i.exec(bare);
    if (!m) return bare;
    const name = m[1][0].toUpperCase() + m[1].slice(1).toLowerCase();
    return `${name} ${m[2].split("-").join(".")}`;
}

/** The full label: srv's, else made from the id; "" when there is neither. */
function fullLabel(facts: Pick<ModelFacts, "label" | "id">): string {
    const label = clean(facts.label);
    if (label) return label;
    const id = clean(facts.id);
    return id ? labelFromId(id) : "";
}

/** The words on the tag: a leading "Claude " dropped, cut at `MODEL_TAG_MAX_CHARS` with an ellipsis. */
export function shortModelText(full: string): string {
    const text = full.replace(/^claude[\s-]+/i, "").trim();
    return text.length > MODEL_TAG_MAX_CHARS ? `${text.slice(0, MODEL_TAG_MAX_CHARS - 1)}…` : text;
}

/** The family, matched on the label, the id and the provider, case-insensitively. A route or local model is `other`. */
export function modelFamily(facts: Pick<ModelFacts, "label" | "id" | "provider">): ModelFamily {
    const hay = `${clean(facts.label)} ${clean(facts.id)}`.toLowerCase();
    for (const tier of CLAUDE_FAMILIES) if (hay.includes(tier)) return tier;
    const provider = clean(facts.provider).toLowerCase();
    if (/\bgpt\b|gpt-|codex|\bo[34]\b|\bo[34]-/.test(hay) || provider === "codex" || provider === "openai") return "gpt";
    if (hay.includes("gemini") || provider === "gemini") return "gemini";
    return "other";
}

/**
 * The tag for an agent's model, or null when no model is known (nothing is drawn, no placeholder): neither a label nor
 * an id. A label with a source of `unknown` is drawn muted, like a request: srv sends the label only when it has one,
 * and does not call it confirmed.
 */
export function modelTag(facts: ModelFacts): ModelTag | null {
    const source = clean(facts.source);
    const full = fullLabel(facts);
    if (!full) return null;
    const family = modelFamily(facts);
    const pendingTo = fullLabel({ label: facts.pendingTo, id: null });
    const pending = pendingTo !== "";
    const requested = source === "requested" || source === "unknown";
    const effort = clean(facts.effort);

    const lines: string[] = [];
    if (pending) lines.push(`Switching to ${pendingTo} at its next turn. Now: ${full}.`);
    else if (requested) lines.push(`${full} was asked for; not confirmed yet.`);
    else if (source === "observed") lines.push(`${full}, from its last call.`);
    else if (source === "route") lines.push(`${full}, on its model route.${effort ? ` Effort ${effort}.` : ""}`);
    else lines.push(`${full}, running now (confirmed by its CLI).${effort ? ` Effort ${effort}.` : ""}`);
    const definition = clean(facts.definitionLabel);
    if (definition && definition.toLowerCase() !== full.toLowerCase()) lines.push(`Its definition says ${definition}.`);

    return {
        text: shortModelText(full),
        family,
        hue: FAMILY_HUE[family],
        muted: pending || requested,
        glyph: pending ? "arrow-right" : glyphOf(family),
        lines,
    };
}

/**
 * The model facts a runtime row carries (live facts spec #48221, `agentruntime.list`): `model_id`, `model_label`,
 * `model_source`, `effort`, `pending_switch` (the short label of the model it switches to at the end of the turn) and
 * `definition_model_label`, all optional and omitted when unknown. A row with neither a label nor an id gives null, so
 * no tag is drawn. This is the one place the Swarm reads them from.
 */
export function readModelFacts(
    row: Pick<AgentRuntime, "provider" | "model_id" | "model_label" | "model_source" | "effort" | "pending_switch" | "definition_model_label">
): ModelFacts | null {
    const label = text(row.model_label);
    const id = text(row.model_id);
    if (!label && !id) return null;
    return {
        label,
        id,
        provider: text(row.provider),
        source: text(row.model_source),
        pendingTo: text(row.pending_switch),
        definitionLabel: text(row.definition_model_label),
        effort: text(row.effort),
    };
}

const text = (v: string | undefined): string | null => (v && v.trim() ? v.trim() : null);

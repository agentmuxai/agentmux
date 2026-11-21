// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The numbers a Tag's look is built from, written once. tag.scss writes the same percentages
 * (a test, tag-contrast.test.ts, pins the two together), and the contrast test measures with
 * these, so what is measured is what is painted.
 */

/** A tag's tone, as `Pill`'s: neutral is a plain fact, the rest carry a colour. */
export type TagTone = "neutral" | "ok" | "warn" | "error" | "accent";

export const TAG_TONES: readonly TagTone[] = ["neutral", "ok", "warn", "error", "accent"];

/**
 * The theme colour token that is a tone's colour, its edge token (null for neutral, whose edge
 * is the tone colour at TAG_NEUTRAL_EDGE_PCT over the surface), and the fill's share of the tone.
 */
export const TAG_TONE_TOKENS: Record<TagTone, { tone: string; edge: string | null; fillPct: number }> = {
    neutral: { tone: "secondary-text-color", edge: null, fillPct: 10 },
    ok: { tone: "success-color", edge: "success-edge", fillPct: 14 },
    warn: { tone: "warning-color", edge: "warning-edge", fillPct: 14 },
    error: { tone: "error-color", edge: "error-edge", fillPct: 14 },
    accent: { tone: "accent-color", edge: "accent-edge", fillPct: 14 },
};

/** A neutral tag's outline is the secondary text colour at this share over the surface it sits on. */
export const TAG_NEUTRAL_EDGE_PCT = 80;

/**
 * The fixed order a card's tags read in, and the order `+N` cuts from the end: the action
 * (the ribbon) has its own corner and is never cut, then what needs attention (errors), the
 * plan (Free), where it runs (Local, Container, Native), and any other fact (Upgrade, a count).
 * A call site names one of these; it never invents a rank.
 */
export const TAG_GROUPS = { action: 0, attention: 1, plan: 2, where: 3, fact: 4 } as const;
export type TagGroup = keyof typeof TAG_GROUPS;

/**
 * The Swarm's coloured tags: a seat's role and a model's family each have a hue (a colour token in theme.scss and
 * its `-edge`), which a Tag takes in place of a tone. The names are the tokens' (`--seat-leader`, `--model-opus`),
 * so a call site says which, never a colour. docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §4.2 and §5.
 */
export const TAG_HUES = {
    seat: ["seat-leader", "seat-super", "seat-specialist", "seat-platform-tester", "seat-analyst", "seat-worker"],
    model: ["model-fable", "model-opus", "model-sonnet", "model-haiku", "model-gpt", "model-gemini", "model-other"],
} as const;

export type SeatHue = (typeof TAG_HUES.seat)[number];
export type ModelHue = (typeof TAG_HUES.model)[number];
export type TagHue = SeatHue | ModelHue;

/** A hue's fill is its colour at this share over the surface, the coloured tones' share. */
export const TAG_HUE_FILL_PCT = 14;

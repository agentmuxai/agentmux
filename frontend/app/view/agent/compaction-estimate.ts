// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Estimated progress for a context compaction (Tier 4 of
 * `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`, built by
 * `docs/specs/SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md`).
 *
 * Claude Code reports no real progress for a compaction: it is one summarizing
 * model call, announced at the start and at the end. So the only honest bar is
 * an ESTIMATE from how long earlier compactions took, and the UI says so.
 *
 * Pure functions, plus a small `localStorage`-backed sample store. This file
 * deliberately shares no code with `compact-boundary.ts`: it only feeds this
 * estimate, and changing how the live boundary is consumed is a separate
 * decision (spec §3).
 */

import { getRuntimeConfig } from "./buildRuntimeArgs";
import { PROVIDER_FLAGS_META_KEY } from "./launch-args";
import { effectiveModel } from "./runtime-capabilities";

/** One finished compaction. */
export interface CompactionSample {
    /** The boundary frame's uuid — a frame seen twice counts once. */
    uuid: string;
    /** Tokens in context when compaction started. */
    preTokens: number;
    durationMs: number;
    /** The model the compaction ran on (see `compactionModelKey`), when known. */
    model?: string;
}

const STORAGE_KEY = "agentmux:compaction-samples:v1";
const MAX_SAMPLES = 30;
const MIN_ESTIMATE_MS = 5_000;
const MAX_ESTIMATE_MS = 600_000;
const SCALE_MIN = 0.5;
const SCALE_MAX = 2;
/** The fill never reaches 100%: an estimate must not read as "done". */
const MAX_FILL = 0.95;

type SampleStorage = Pick<Storage, "getItem" | "setItem">;

function pos(n: unknown): n is number {
    return typeof n === "number" && Number.isFinite(n) && n > 0;
}

/**
 * A sample from a stdout `compact_boundary` frame, or null. The CLI writes
 * snake_case (`compact_metadata.pre_tokens`) to stdout and camelCase
 * (`compactMetadata.preTokens`) to its transcript; both are accepted.
 */
export function parseCompactionSample(rawEvent: unknown, model?: string | null): CompactionSample | null {
    if (!rawEvent || typeof rawEvent !== "object") return null;
    const e = rawEvent as Record<string, unknown>;
    if (e.type !== "system" || e.subtype !== "compact_boundary") return null;
    const meta = (e.compact_metadata ?? e.compactMetadata) as Record<string, unknown> | undefined;
    if (!meta || typeof meta !== "object") return null;
    const preTokens = meta.pre_tokens ?? meta.preTokens;
    const durationMs = meta.duration_ms ?? meta.durationMs;
    if (!pos(preTokens) || !pos(durationMs)) return null;
    const uuid = typeof e.uuid === "string" && e.uuid ? e.uuid : `${preTokens}:${durationMs}`;
    return model ? { uuid, preTokens, durationMs, model } : { uuid, preTokens, durationMs };
}

function defaultStorage(): SampleStorage | null {
    try {
        return typeof localStorage === "undefined" ? null : localStorage;
    } catch {
        return null;
    }
}

/** The stored samples, oldest first. Anything unreadable or malformed is dropped. */
export function readCompactionSamples(storage: SampleStorage | null = defaultStorage()): CompactionSample[] {
    if (!storage) return [];
    try {
        const raw = storage.getItem(STORAGE_KEY);
        if (!raw) return [];
        const parsed: unknown = JSON.parse(raw);
        if (!Array.isArray(parsed)) return [];
        const out: CompactionSample[] = [];
        for (const item of parsed) {
            if (!item || typeof item !== "object") continue;
            const s = item as Record<string, unknown>;
            if (typeof s.uuid === "string" && s.uuid && pos(s.preTokens) && pos(s.durationMs)) {
                const sample: CompactionSample = { uuid: s.uuid, preTokens: s.preTokens, durationMs: s.durationMs };
                if (typeof s.model === "string" && s.model) sample.model = s.model;
                out.push(sample);
            }
        }
        return out.slice(-MAX_SAMPLES);
    } catch {
        return [];
    }
}

/** Remember one finished compaction. Never throws; a repeated uuid is ignored. */
export function recordCompactionSample(
    sample: CompactionSample | null,
    storage: SampleStorage | null = defaultStorage()
): void {
    if (!sample || !storage) return;
    try {
        const all = readCompactionSamples(storage);
        if (all.some((s) => s.uuid === sample.uuid)) return;
        all.push(sample);
        storage.setItem(STORAGE_KEY, JSON.stringify(all.slice(-MAX_SAMPLES)));
    } catch {
        /* storage full or denied: the estimate just has fewer samples */
    }
}

/**
 * The model a pane's compaction samples are kept under: the model the process is
 * configured to run (runtime selection, or a `--model` in the agent's own flags),
 * so a `/model` switch counts before the next reply arrives; else the resolved
 * model id from the replies.
 */
export function compactionModelKey(
    blockMeta: Record<string, any> | undefined,
    resolvedModel: string | null | undefined
): string | undefined {
    const configured = effectiveModel(getRuntimeConfig(blockMeta).model ?? "", blockMeta?.[PROVIDER_FLAGS_META_KEY]);
    return configured || resolvedModel || undefined;
}

/**
 * The samples to estimate from: the ones recorded for `model` when there are
 * any, otherwise all of them. Duration depends mostly on which model
 * summarizes; samples from before models were recorded only serve the fallback.
 */
export function samplesForModel(samples: readonly CompactionSample[], model: string | null | undefined): CompactionSample[] {
    if (model) {
        const own = samples.filter((s) => s.model === model);
        if (own.length > 0) return own;
    }
    return [...samples];
}

function median(nums: number[]): number {
    const s = [...nums].sort((a, b) => a - b);
    const mid = Math.floor(s.length / 2);
    return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/**
 * How long this compaction will probably take, in ms, or null with no
 * history. Median duration (one slow run doesn't move it), scaled by the
 * current context size relative to the samples' but clamped to 0.5x..2x, then
 * kept within 5s..600s.
 */
export function estimateCompactionMs(samples: readonly CompactionSample[], contextTokens: number | null | undefined): number | null {
    if (samples.length === 0) return null;
    const base = median(samples.map((s) => s.durationMs));
    let scale = 1;
    if (pos(contextTokens)) {
        const typical = median(samples.map((s) => s.preTokens));
        scale = Math.min(SCALE_MAX, Math.max(SCALE_MIN, contextTokens / typical));
    }
    return Math.round(Math.min(MAX_ESTIMATE_MS, Math.max(MIN_ESTIMATE_MS, base * scale)));
}

/**
 * Where the bar is, given elapsed time and the estimate. The fill stops at
 * 95%; `over` is true once the estimate is exceeded, so the UI can switch to
 * "longer than usual" instead of a bar stuck near the end.
 */
export function compactionProgress(elapsedMs: number, estimateMs: number): { fraction: number; over: boolean } {
    const elapsed = Math.max(0, elapsedMs);
    return { fraction: Math.min(MAX_FILL, elapsed / estimateMs), over: elapsed > estimateMs };
}

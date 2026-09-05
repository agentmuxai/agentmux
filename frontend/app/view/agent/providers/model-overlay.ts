// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

import { PROVIDERS, resolveProviderAlias } from "./catalog";
import type { ProviderDefinition, ProviderModel } from "./types";

// ── API-sourced model catalog overlay ───────────────────────────────────────
//
// The static `models` above are hand-curated and their version labels drift
// ("Sonnet 4.6" → "Sonnet 5" …). `providers.models` (backend) reads the
// authoritative list from the Anthropic Models API; `setProviderModels` folds
// it in. Signal-backed so a strip that rendered before the fetch resolved
// re-renders when the catalog lands (see app-init.ts).
const [modelOverlay, setModelOverlay] = createSignal<Record<string, ProviderModel[]>>({});

/** Numeric version tokens of a model id, e.g. "claude-sonnet-5" → [5],
 *  "claude-opus-4-8" → [4,8]. Order-independent "which is newest" key. */
function versionTuple(id: string): number[] {
    return (id.match(/\d+/g) ?? []).map((n) => parseInt(n, 10));
}

/** Element-wise version compare; missing positions count as 0 so [5] > [3,5]. */
function cmpVersion(a: number[], b: number[]): number {
    const n = Math.max(a.length, b.length);
    for (let i = 0; i < n; i++) {
        const d = (a[i] ?? 0) - (b[i] ?? 0);
        if (d !== 0) return d;
    }
    return 0;
}

/** The highest-versioned model in a group — robust to API ordering, so a newly
 *  shipped Sonnet 5.x / 6 wins over the prior version automatically. */
function pickNewest(models: ProviderModel[]): ProviderModel {
    return models.reduce((best, m) =>
        cmpVersion(versionTuple(m.value), versionTuple(best.value)) > 0 ? m : best,
    );
}

/** Family key for an id the curated list doesn't cover — its alphabetic tokens
 *  minus the "claude" prefix, e.g. "claude-fable-5" → "fable". Groups versions
 *  of a genuinely new family so we surface one (newest) entry for it. */
function familyKey(id: string): string {
    const alpha = id.split("-").filter((t) => /^[a-z]+$/i.test(t) && t.toLowerCase() !== "claude");
    return alpha.join("-").toLowerCase() || id.toLowerCase();
}

/** Drop the redundant "Claude " prefix so API labels match the curated style
 *  ("Claude Sonnet 5" → "Sonnet 5"). */
function cleanLabel(label: string): string {
    return label.replace(/^claude\s+/i, "").trim();
}

/** Whether a curated `value` is a family ALIAS rather than a concrete model id.
 *
 *  An alias carries no version digits ("opus", "sonnet", "haiku") and is
 *  resolved by the CLI at call time, so it must keep its curated value — that
 *  self-resolution is the entire point. A concrete id ("claude-fable-5-1") does
 *  NOT self-resolve, so its value has to be refreshed alongside its label or the
 *  row ends up advertising a model it doesn't actually select. */
function isAliasValue(value: string): boolean {
    return !/\d/.test(value);
}

/**
 * Fold the authoritative catalog into a provider's model list. Behavior-
 * preserving by design:
 *  - Curated family entries (opus/sonnet/haiku/fable) keep their curated
 *    `value` (for opus/sonnet/haiku, a short alias `--model` resolves to the
 *    current version; fable has no such alias yet, so its `value` is the
 *    concrete pinned id instead), `default` marker, `aliases`, and
 *    `description`; only their **label** is refreshed to the newest matching
 *    API model. This is what turns "Sonnet 4.6" into "Sonnet 5" without changing
 *    what `--model` receives or breaking `models.find(m => m.default)`.
 *  - Families the curated list doesn't cover (any future family) are appended
 *    automatically, one newest entry each, using the concrete API id as
 *    `value` (always a valid `--model` target). No family is hardcoded.
 *  - Matching uses `familyKey()` (alphabetic tokens, "claude" stripped, digits
 *    dropped) rather than a raw substring test, so a *version-pinned* curated
 *    `value` (fable's `claude-fable-5`) still matches a future API version
 *    (`claude-fable-6`) the same way a generic alias (`sonnet`) already does —
 *    a plain `.includes()` test would only match the exact pinned string,
 *    leaving the curated row stale AND appending a duplicate "extra" entry
 *    once a newer version showed up in the API.
 * Empty input (no token / macOS Keychain / offline) is a no-op → static list.
 */
export function setProviderModels(id: string, apiModels: ProviderModel[]): void {
    if (apiModels.length === 0) return;
    const canonical = resolveProviderAlias(id);
    const base = PROVIDERS[id] ?? PROVIDERS[canonical];
    if (!base) return;

    const consumed = new Set<string>();

    // 1. Refresh curated family entries from the newest API model in that family.
    const curated = base.models.map((m) => {
        const family = familyKey(m.value);
        const matches = apiModels.filter((a) => familyKey(a.value) === family);
        if (matches.length === 0) return m;
        matches.forEach((a) => consumed.add(a.value)); // don't re-surface as an "extra"
        const newest = pickNewest(matches);
        const label = cleanLabel(newest.label);
        // Alias rows keep their curated value (the CLI resolves it); concrete
        // version-pinned rows must have their value refreshed too. Refreshing
        // only the label there produced a row that displayed "Fable 5.1" while
        // still passing `claude-fable-5` to `--model` — advertising one model
        // and selecting an older one.
        return isAliasValue(m.value) ? { ...m, label } : { ...m, value: newest.value, label };
    });

    // 2. Surface families the curated list misses (grouped, newest per family).
    const byFamily = new Map<string, ProviderModel[]>();
    for (const a of apiModels) {
        if (consumed.has(a.value)) continue;
        const key = familyKey(a.value);
        const group = byFamily.get(key) ?? [];
        group.push(a);
        byFamily.set(key, group);
    }
    const extras = [...byFamily.values()].map((group) => {
        const newest = pickNewest(group);
        return { value: newest.value, label: cleanLabel(newest.label) } satisfies ProviderModel;
    });

    setModelOverlay((prev) => ({ ...prev, [canonical]: [...curated, ...extras] }));
}

export function getProvider(id: string): ProviderDefinition | undefined {
    const base = PROVIDERS[id] ?? PROVIDERS[resolveProviderAlias(id)];
    if (!base) return undefined;
    const overlay = modelOverlay()[resolveProviderAlias(base.id)];
    return overlay ? { ...base, models: overlay } : base;
}

export function getProviderList(): ProviderDefinition[] {
    // Route through getProvider so list consumers that read `.models` see the
    // same API-sourced overlay (refreshed labels / new families) as callers of
    // getProvider(), not the raw static list.
    return Object.keys(PROVIDERS).map((id) => getProvider(id)!);
}

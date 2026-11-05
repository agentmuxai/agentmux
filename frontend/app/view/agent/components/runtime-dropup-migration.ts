// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Moving a persisted model id the live catalog has replaced onto its successor. */

import { createEffect } from "solid-js";
import { familyKey, isAccountListed } from "../providers";
import type { DropupState } from "./runtime-dropup-state";

/** Creates the one effect that migrates a superseded stored model. */
export function useModelMigration(s: DropupState) {
    // Migrate a persisted model id the live-catalog overlay has superseded.
    //
    // `setProviderModels` refreshes CONCRETE (version-pinned) option values
    // when the authoritative catalog resolves, so an agent configured before
    // such a bump holds an id that is no longer present in `modelOptions()`.
    // Left alone, `modelLabel()` falls back to rendering the raw id and
    // `build()` marks no row `current` — the dropdown silently loses its
    // selection display for that agent.
    //
    // This migrates the PERSISTED value rather than making the lookups
    // family-tolerant. A display-only fix would show the new row as selected
    // while block meta still held the old id — reintroducing exactly the
    // advertise-one/select-another mismatch this migration exists to remove, one
    // layer further down. Alias values ("opus"/"sonnet") are never superseded,
    // so they never enter this path (#2990).
    createEffect(() => {
        const opts = s.modelOptions();
        const current = s.runtime().model;
        if (!current || opts.length === 0) return;
        if (opts.some((o) => o.value === current)) return;
        // From the account's own list (Codex), a model not on it is one the
        // account can't run any more: move to the list's default.
        const replacement =
            opts.find((o) => familyKey(o.value) === familyKey(current)) ??
            (isAccountListed(s.props.providerId) ? (opts.find((o) => o.default) ?? opts[0]) : undefined);
        if (replacement) void s.migrateModel(replacement.value);
    });
}

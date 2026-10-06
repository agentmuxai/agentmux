// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's context meter, as the view shows it: the reducer's
 * `context` reading put through `plausibleReading`, the provenance note for its
 * tooltip, and the block-meta mirror the Swarm reads.
 *
 * Everything that displays the meter reads it from here, so a reading the
 * meter refuses (more tokens than its window — the "17m / 200k" a freshly
 * opened pane once showed) reaches no view at all.
 * store/agent-pane-state/context-reading.ts;
 * docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md.
 */

import { createEffect, createMemo, on, untrack, type Accessor } from "solid-js";

import {
    contextReadingFromMeta,
    contextReadingNote,
    plausibleReading,
    sameContextReading,
    type ContextReading,
} from "@/app/store/agent-pane-state/context-reading";
import { setBlockMeta } from "@/app/store/block-meta";

import { getRuntimeConfig } from "../buildRuntimeArgs";
import { PROVIDER_FLAGS_META_KEY } from "../launch-args";
import { effectiveRuntime } from "../runtime-capabilities";

export interface ContextMeter {
    /** The reading to display, or null when there is none fit to show. */
    reading: Accessor<ContextReading | null>;
    /** Where its numbers come from, for the tooltip and popover. */
    note: Accessor<string | undefined>;
}

export interface ContextMeterOptions {
    /**
     * True once the pane's history restore has completed (`InitReady`). Until
     * then the reading is null only because the history seed hasn't landed,
     * and mirroring that null would wipe the persisted reading the Swarm is
     * showing. After a failed restore (`InitFailed`) it stays false: the
     * persisted reading is the best there is until a live one replaces it.
     */
    ready: Accessor<boolean>;
    /** The block's meta: the persisted reading (to skip a write that would
     *  change nothing) and the model setting. */
    meta: Accessor<MetaType | null | undefined>;
    /** The pane's model setting changed (`/model`, the model menu): dispatch
     *  `ContextModelSwitched`. Not called for the setting at mount. */
    onModelSwitched: (model: string) => void;
}

export function useContextReading(
    blockId: string,
    context: Accessor<ContextReading | null>,
    opts: ContextMeterOptions,
): ContextMeter {
    // Compared by value so a reducer step that rebuilds an identical reading
    // doesn't re-render or re-write meta.
    const reading = createMemo(() => plausibleReading(context()), null, { equals: sameContextReading });
    const note = createMemo(() => {
        const r = reading();
        return r ? contextReadingNote(r) : undefined;
    });

    // Writes go out one at a time, latest wins: srv handles each RPC on its
    // own task, so two writes in flight together could land in either order
    // and leave an older reading persisted. A reading that changes while a
    // write is out replaces any not yet sent.
    let sending = false;
    let queued: { value: ContextReading | null } | null = null;
    const send = async () => {
        sending = true;
        while (queued) {
            const { value } = queued;
            queued = null;
            try {
                await setBlockMeta(blockId, { "agent:context": value, "term:ctx-tokens": null });
            } catch (err) {
                console.warn("[agent-context]", `pane=${blockId.slice(0, 7)}`, "meta mirror write failed:", err);
            }
        }
        sending = false;
    };
    const write = (value: ContextReading | null) => {
        queued = { value };
        if (!sending) void send();
    };
    // Whether this pane has mirrored a reading yet: until then, a null means
    // only that nothing has been read, not that the persisted one is wrong.
    let mirrored = false;

    // Mirror the reading to block meta so the Swarm view can read it without
    // access to per-pane in-memory signals — the displayable reading only.
    // Fires when the reading changes (once per API call while a turn runs).
    // A reading is written whenever there is one; a null only once the
    // history restore has settled or after this pane wrote a reading — a
    // mount-time null would wipe the persisted reading the Swarm shows (and
    // a failed restore would never write it back). The legacy bare
    // `term:ctx-tokens` is cleared: it held whatever the pane once computed,
    // including history seeds that were turn totals, and the Swarm no longer
    // reads it. Meta is read untracked: this effect writes it, and must not
    // re-run on its own write.
    createEffect(
        on([reading, opts.ready], ([r, ready]) => {
            if (r == null && !ready && !mirrored) return;
            const meta = untrack(opts.meta);
            const persisted = contextReadingFromMeta(meta?.["agent:context"]);
            const unchanged =
                sameContextReading(persisted, r) &&
                // An implausible persisted reading parses to null; it still
                // needs clearing when the pane has none to show.
                (r != null || meta?.["agent:context"] == null) &&
                meta?.["term:ctx-tokens"] == null;
            if (r != null) mirrored = true;
            if (unchanged) return;
            write(r);
        }),
    );

    // A model change keeps the conversation but not its window: the reading
    // says so until the new model's first reply.
    createEffect(
        on(
            () => {
                const meta = opts.meta() ?? undefined;
                return effectiveRuntime(getRuntimeConfig(meta), meta?.[PROVIDER_FLAGS_META_KEY]).model;
            },
            (modelSetting, prev) => {
                if (prev !== undefined && modelSetting && modelSetting !== prev) opts.onModelSwitched(modelSetting);
            },
            { defer: true },
        ),
    );

    return { reading, note };
}

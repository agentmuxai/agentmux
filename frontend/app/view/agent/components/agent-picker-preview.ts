// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The "Preview agent CLIs" section of the New Agent picker: which harnesses it holds, and whether the person left it open,
// kept on this machine like the sort (agent-picker-sort.ts).

import type { AgentDefinition } from "@/app/store/rpc-api";
import { createEffect, createMemo, createSignal, type Accessor } from "solid-js";
import type { CardFilter } from "../defaults/provider-access-model";
import { getProvider } from "../providers";

/** The harness is installable and working but not supported to the first-class depth: it lives in the Preview section. */
export const isPreviewAgent = (agent: AgentDefinition): boolean => getProvider(agent.provider)?.tier === "preview";

const PREVIEW_OPEN_STORAGE_KEY = "agentPicker:previewOpen";

/** Closed unless the person left it open; a failed read is closed too. */
export function loadPreviewOpen(): boolean {
    try {
        return localStorage.getItem(PREVIEW_OPEN_STORAGE_KEY) === "1";
    } catch {
        return false;
    }
}

export function storePreviewOpen(open: boolean): void {
    try {
        localStorage.setItem(PREVIEW_OPEN_STORAGE_KEY, open ? "1" : "0");
    } catch {
        // best-effort: the section is just closed again next time
    }
}

/**
 * The Preview agent CLIs sit in their own collapsed section under the first-class cards: never in the first row, never
 * the default choice. A filter that matches one opens the section (not remembered), so it never hides a match.
 */
export function createPreviewSection(d: {
    shownTemplates: Accessor<AgentDefinition[]>;
    cardFilters: Accessor<CardFilter[]>;
    openTemplate: Accessor<string | null>;
    /** Closes the open panel; false when it is busy creating an agent and can't close. */
    closePanel: () => boolean;
}) {
    const firstClassTemplates = createMemo(() => d.shownTemplates().filter((a) => !isPreviewAgent(a)));
    const previewTemplates = createMemo(() => d.shownTemplates().filter(isPreviewAgent));
    const [previewOpen, setPreviewOpenState] = createSignal(loadPreviewOpen());
    createEffect(() => {
        if (d.cardFilters().length > 0 && previewTemplates().length > 0) setPreviewOpenState(true);
    });
    /** Opens or closes the section; closing it closes the open panel of a card inside it, unless that panel is busy. */
    const togglePreview = (): void => {
        const next = !previewOpen();
        if (!next) {
            const open = d.openTemplate();
            if (open && previewTemplates().some((a) => a.id === open) && !d.closePanel()) return;
        }
        setPreviewOpenState(next);
        storePreviewOpen(next);
    };
    return { firstClassTemplates, previewTemplates, previewOpen, togglePreview };
}

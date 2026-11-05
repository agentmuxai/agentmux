// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * McpManager's state: the two models, the signals the page owns, and the memos derived from them. Created once,
 * first in the component, so the computations keep the order they were written in.
 */

import { createMemo, createSignal, onCleanup } from "solid-js";

import { matchesEveryWord } from "@/app/util/fuzzysearch";
import type { McpServerCatalogItem } from "@/types/rpc/McpServerCatalogItem";
import { connectionSummary, McpCatalogModel } from "./mcp-model";
import { McpObservedModel, notLoadedBy, notLoadedText, sourceLabel, type ObservedServerRow } from "./mcp-observed";

/** The card of a server being added, first in "Your servers" while its form is open. */
export const NEW_KEY = "\u0000new";
export const CUSTOM_KEY = "\u0000custom";

export const observedKey = (row: ObservedServerRow) => `${row.source}\u0000${row.name}`;

export function createMcpState() {
    const model = new McpCatalogModel();
    onCleanup(() => model.dispose());
    const observed = new McpObservedModel();
    onCleanup(() => observed.dispose());

    const [filter, setFilter] = createSignal("");
    // One card is open in the section: a catalog server's (in the model) or
    // an observed server's (here).
    const [observedOpen, setObservedOpen] = createSignal<string | null>(null);
    const [deleting, setDeleting] = createSignal<McpServerCatalogItem | null>(null);
    const [deletingBound, setDeletingBound] = createSignal<string[] | null>(null);

    // Cards are keyed by id, so a refreshed list keeps them (and an open form).
    const serverById = createMemo(() => new Map(model.serversAtom().map((s) => [s.id, s])));
    const catalogNames = createMemo(() => new Set(model.serversAtom().map((s) => s.name)));
    const usesFor = (name: string) =>
        observed
            .rowsAtom()
            .filter((r) => r.name === name)
            .flatMap((r) => r.uses);
    /** Why a catalog server isn't on every agent it should be: they started first. */
    const notLoaded = (name: string) => notLoadedText(notLoadedBy(observed.agentsAtom(), name));
    const otherRows = createMemo(() => observed.rowsAtom().filter((r) => !catalogNames().has(r.name)));
    const rowByKey = createMemo(() => new Map(otherRows().map((r) => [observedKey(r), r])));

    const newDraft = () => {
        const d = model.draftAtom();
        return d && !d.id ? d : null;
    };
    const serverKeys = createMemo(() => {
        const keys = model
            .serversAtom()
            .filter((s) => matchesEveryWord(filter(), s.name, connectionSummary(s.transport, s.config)))
            .map((s) => s.id);
        return newDraft() ? [NEW_KEY, ...keys] : keys;
    });
    const otherKeys = createMemo(() =>
        otherRows()
            .filter((r) => matchesEveryWord(filter(), r.name, sourceLabel(r.source)))
            .map(observedKey),
    );
    const catalogOpen = () => (newDraft() ? NEW_KEY : model.selectedIdAtom());
    const count = () => model.serversAtom().length + otherRows().length;
    const hasAny = () => count() > 0 || newDraft() !== null;

    return {
        model,
        observed,
        filter,
        setFilter,
        observedOpen,
        setObservedOpen,
        deleting,
        setDeleting,
        deletingBound,
        setDeletingBound,
        serverById,
        catalogNames,
        usesFor,
        notLoaded,
        otherRows,
        rowByKey,
        newDraft,
        serverKeys,
        otherKeys,
        catalogOpen,
        count,
        hasAny,
    };
}

export type McpState = ReturnType<typeof createMcpState>;

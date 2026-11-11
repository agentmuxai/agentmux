// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-provider instruction overrides, the state: the rows being edited, kept in step with the draft's raw JSON string
// (see the header of BundleProviderInstructionsSection.tsx for why they are local state), and the edits to them.

import { createEffect, createMemo, createSignal } from "solid-js";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import {
    duplicateProviderKeys,
    parseInstructionsByProvider,
    serializeInstructionsByProvider,
    type ProviderInstruction,
} from "./bundle-model";

export interface BundleProviderInstructionsSectionProps {
    /** The draft's raw `instructions_by_provider` JSON string. */
    value: string;
    onChange: (nextRaw: string) => void;
}

export type ProviderInstructionsState = ReturnType<typeof createProviderInstructions>;

export function createProviderInstructions(props: BundleProviderInstructionsSectionProps) {
    const parsed = createMemo(() => parseInstructionsByProvider(props.value));

    const [rows, setRows] = createSignal<ProviderInstruction[]>(parsed().entries);
    // What we last wrote out, so the re-seed effect can tell an external change
    // (bundle switch) apart from the echo of our own edit.
    let lastEmitted = props.value;

    createEffect(() => {
        const incoming = props.value;
        if (incoming === lastEmitted) return;
        lastEmitted = incoming;
        setRows(parseInstructionsByProvider(incoming).entries);
    });

    const emit = (next: ProviderInstruction[]) => {
        setRows(next);
        const raw = serializeInstructionsByProvider(next);
        lastEmitted = raw;
        props.onChange(raw);
    };

    const updateRow = (index: number, patch: Partial<ProviderInstruction>) => {
        emit(rows().map((r, i) => (i === index ? { ...r, ...patch } : r)));
    };

    const removeRow = (index: number) => {
        emit(rows().filter((_, i) => i !== index));
    };

    const addRow = () => {
        // Blank key by design: the user names it. It is flagged immediately and
        // simply does not serialize until named, so an unnamed row can never
        // reach storage or export.
        emit([...rows(), { provider: "", content: "" }]);
    };

    const dupes = createMemo(() => new Set(duplicateProviderKeys(rows())));
    const knownProviders = createMemo(() => Object.keys(PROVIDERS).sort());

    return { parsed, rows, updateRow, removeRow, addRow, dupes, knownProviders };
}

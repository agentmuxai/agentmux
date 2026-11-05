// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// reactive-coverage: one-shot the skill names are fetched once when the panel opens, as an advisory hint; commit is authoritative

/**
 * The import preview's state: what the person has chosen so far, and the actions that change or hand it on.
 * All of it is built here and handed to Step 3 through `onNext`; no RPC is made except the one advisory fetch
 * of the existing skill names.
 */

import { createSignal, onMount } from "solid-js";

import type { BundleImportSelectionState, BundleImportSkillSelectionState } from "@/app/element/modal-layer";
import type { BundleImportPreviewResponse } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export interface BundleImportPreviewProps {
    preview: BundleImportPreviewResponse;
    onNext: (selection: BundleImportSelectionState) => void;
    onCancel: () => void;
}

export function createPreviewState(props: BundleImportPreviewProps) {
    const { preview } = props;

    const [bundleName, setBundleName] = createSignal(preview.name);
    const [includeInstructions, setIncludeInstructions] = createSignal(true);
    const [instructionsExpanded, setInstructionsExpanded] = createSignal(false);
    const [contextChecked, setContextChecked] = createSignal<Record<number, boolean>>(
        Object.fromEntries(preview.context_files.map((cf) => [cf.id, true])),
    );
    const [skills, setSkills] = createSignal<BundleImportSkillSelectionState[]>(
        // A skill whose text the preview could not show in full starts unchecked:
        // it is imported only once the person has chosen to take what they could
        // not read.
        preview.skills.map((s) => ({ sourceDir: s.source_dir, checked: !s.content_truncated, renameValue: "" })),
    );
    // A skill's text is shown by default (it is what an agent will be told to
    // follow); this holds the ones the person folded away.
    const [skillTextHidden, setSkillTextHidden] = createSignal<Record<string, boolean>>({});
    const [warningsDismissed, setWarningsDismissed] = createSignal(false);
    // Widgets the bundle carries (SPEC_WIDGET_SHARING_2026_10_10.md §3.3): on
    // by default, except one that can't be imported or is installed already.
    const offeredWidgets = () => (preview.widgets ?? []).filter((w) => !w.error && !w.same_as_installed);
    const [widgetChecked, setWidgetChecked] = createSignal<Record<string, boolean>>(
        Object.fromEntries((preview.widgets ?? []).filter((w) => !w.error && !w.same_as_installed).map((w) => [w.id, true])),
    );

    // §4.1 point 2: the modal fetches the full existing global skill-name
    // list ONCE, up front, via skill.catalog.list -- not just the subset
    // preview.skills already flagged "name_conflict" against itself (a
    // rename to some OTHER, unrelated existing global skill name would
    // otherwise show no client-side warning and silently be skipped at
    // commit, since skill_upsert_unique_global still rejects it there).
    const [globalSlugs, setGlobalSlugs] = createSignal<Set<string>>(new Set());
    onMount(() => {
        void RpcApi.SkillCatalogListCommand(TabRpcClient, {})
            .then((items) => setGlobalSlugs(new Set(items.map((i) => i.name))))
            .catch(() => {}); // advisory only -- a failed fetch just means no client-side hint; commit is still authoritative.
    });

    return {
        props,
        preview,
        bundleName,
        setBundleName,
        includeInstructions,
        setIncludeInstructions,
        instructionsExpanded,
        setInstructionsExpanded,
        contextChecked,
        setContextChecked,
        skills,
        setSkills,
        skillTextHidden,
        setSkillTextHidden,
        warningsDismissed,
        setWarningsDismissed,
        offeredWidgets,
        widgetChecked,
        setWidgetChecked,
        globalSlugs,
    };
}

export type BundleImportPreviewState = ReturnType<typeof createPreviewState>;

export function createPreviewActions(s: BundleImportPreviewState) {
    const { preview } = s;

    // Live union of the global catalog + every OTHER selected skill's
    // current slug/rename (§4.1 point 2) -- advisory client-side check
    // only; skill_upsert_unique_global is the real authority at commit.
    const takenSlugsFor = (sourceDir: string): Set<string> => {
        const taken = new Set<string>(s.globalSlugs());
        for (const row of s.skills()) {
            if (row.sourceDir === sourceDir || !row.checked) continue;
            const other = preview.skills.find((sk) => sk.source_dir === row.sourceDir);
            const effective = row.renameValue.trim() || other?.slug;
            if (effective) taken.add(effective);
        }
        return taken;
    };

    const updateSkill = (sourceDir: string, patch: Partial<BundleImportSkillSelectionState>) => {
        s.setSkills((prev) => prev.map((sk) => (sk.sourceDir === sourceDir ? { ...sk, ...patch } : sk)));
    };

    const toggleContext = (id: number) => {
        s.setContextChecked((prev) => ({ ...prev, [id]: !prev[id] }));
    };

    const suggestedAltName = () => {
        const m = s.bundleName().match(/^(.*) \((\d+)\)$/);
        if (m) return `${m[1]} (${Number(m[2]) + 1})`;
        return `${s.bundleName()} (2)`;
    };

    const next = () => {
        s.props.onNext({
            bundleName: s.bundleName().trim() || preview.name,
            includeInstructions: s.includeInstructions(),
            includeContextFileIds: preview.context_files
                .filter((cf) => s.contextChecked()[cf.id])
                .map((cf) => cf.id),
            skills: s.skills(),
            includeWidgets: s
                .offeredWidgets()
                .filter((w) => s.widgetChecked()[w.id])
                .map((w) => w.id),
        });
    };

    return { takenSlugsFor, updateSkill, toggleContext, suggestedAltName, next };
}

export type BundleImportPreviewActions = ReturnType<typeof createPreviewActions>;

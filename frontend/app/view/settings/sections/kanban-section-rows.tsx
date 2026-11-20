// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Kanban's two behaviour rows (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §8.2): whether a merged PR closes its
// task in Review, and Review's soft limit. Each was a setting only settings.json held; here it is a row with a switch or a
// number, found by the settings search like the rest.

import type { JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { settingsAtom } from "@/app/store/global";
import { reviewLimitOf } from "@/app/view/kanban/kanban-columns";

import { NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { settingEntry } from "../settings-model";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──
export const KANBAN_ROW_SETTINGS = {
    closeOnMerge: settingEntry({
        id: "kanban.close_on_merge",
        labelKey: "settings.kanban.closeOnMerge.label",
        descriptionKey: "settings.kanban.closeOnMerge.description",
        section: "kanban",
        groupKey: "settings.kanban.group.behaviour",
        keywords: ["pull request", "merge", "review", "accept", "close a task", "kanban:closeonmerge"],
    }),
    reviewLimit: settingEntry({
        id: "kanban.review_limit",
        labelKey: "settings.kanban.reviewLimit.label",
        descriptionKey: "settings.kanban.reviewLimit.description",
        section: "kanban",
        groupKey: "settings.kanban.group.behaviour",
        keywords: ["review", "limit", "amber", "waiting", "too many", "kanban:reviewlimit"],
    }),
};

const settings = () => (settingsAtom() ?? {}) as Record<string, unknown>;

/** A merged PR closes its task in Review unless the switch is off (srv's default is on). */
export const closeOnMergeOn = (s: Record<string, unknown> | undefined) => s?.["kanban:closeonmerge"] !== false;

export function KanbanBehaviourRows(): JSX.Element {
    return (
        <>
            <SectionHeader label={t("settings.kanban.group.behaviour")} />
            <SettingRow
                id={KANBAN_ROW_SETTINGS.closeOnMerge.id}
                settingKey="kanban:closeonmerge"
                label={KANBAN_ROW_SETTINGS.closeOnMerge.label}
                description={KANBAN_ROW_SETTINGS.closeOnMerge.description}
                control={
                    <ToggleControl checked={closeOnMergeOn(settings())} onChange={(v) => set("kanban:closeonmerge", v)} />
                }
            />
            <SettingRow
                id={KANBAN_ROW_SETTINGS.reviewLimit.id}
                settingKey="kanban:reviewlimit"
                label={KANBAN_ROW_SETTINGS.reviewLimit.label}
                description={KANBAN_ROW_SETTINGS.reviewLimit.description}
                control={
                    <NumberControl
                        class="setting-number"
                        min={0}
                        step={1}
                        value={reviewLimitOf(settings())}
                        onChange={(v) => set("kanban:reviewlimit", Math.max(0, Math.floor(v)))}
                    />
                }
            />
        </>
    );
}

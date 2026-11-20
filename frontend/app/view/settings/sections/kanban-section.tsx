// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Kanban: the person sets the stages of a board here (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §8.2): how
// many, what each is called, its colour, and which of the board's sources it shows. A board keeps its own set. The
// cog at the top right of a Kanban pane opens this section on the board it was showing.

import { For, type JSX } from "solid-js";

import { Select } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { tierOf } from "@/app/view/kanban/kanban-boards";

import { SectionHeader } from "../settings-controls";
import { settingEntry } from "../settings-model";
import type { SettingsViewModel } from "../settings-model";
import { createKanbanStagesState } from "./kanban-section-state";
import { Footer, History, Notices, Preview, Problems, StageActions, StageList } from "./kanban-section-parts";

import "./kanban-section.scss";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──
export const KANBAN_SETTINGS = {
    stages: settingEntry({
        id: "kanban.stages",
        labelKey: "settings.kanban.stages.label",
        descriptionKey: "settings.kanban.stages.description",
        section: "kanban",
        groupKey: "settings.kanban.group.stages",
        keywords: ["columns", "board", "stage names", "stage colours", "backlog", "needs you", "merged", "released", "lanes", "workflow"],
    }),
};

export function KanbanSection(props: { model?: SettingsViewModel }): JSX.Element {
    const s = createKanbanStagesState(
        () => props.model?.kanbanBoard() ?? null,
        () => props.model?.kanbanAsk?.() ?? 0
    );
    return (
        <div class="settings-section-body kanban-stages" id={`setting-${KANBAN_SETTINGS.stages.id}`}>
            <SectionHeader label={t("settings.kanban.group.stages")} />
            <p class="kanban-stages__tier">{t("settings.kanban.stages.description")}</p>
            <div class="kanban-stages__board">
                <label>
                    {t("settings.kanban.board")}{" "}
                    <Select
                        density="compact"
                        ariaLabel={t("settings.kanban.board")}
                        value={s.selected()}
                        onChange={s.select}
                    >
                        <For each={s.boards()}>
                            {(b) => (
                                <option value={b.slug} disabled={b.archived}>
                                    {b.archived ? t("settings.kanban.boardArchived", { name: b.name }) : b.name}
                                </option>
                            )}
                        </For>
                    </Select>
                </label>
                <span class="kanban-stages__tier">{s.info() ? tierOf(s.info()!.tier).label : ""}</span>
            </div>
            <Notices s={s} />
            <StageList s={s} />
            <StageActions s={s} />
            <Preview s={s} />
            <Problems s={s} />
            <Footer s={s} />
            <History s={s} />
        </div>
    );
}

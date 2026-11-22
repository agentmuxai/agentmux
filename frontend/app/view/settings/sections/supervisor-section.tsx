// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Supervisor: the master switch first, with what it allows in a line, then every limit and notice the Swarm
// Supervisor has (docs/specs/SPEC_SWARM_SUPERVISOR_2026_10_16.md). They were the Swarm's Supervise panel; they live
// here now, and the Swarm's mode menu opens this section. The keys, defaults and bounds are srv's
// (`supervisor/settings.rs`; settings-defaults.test.ts holds them to it).

import { For, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { settingsAtom } from "@/app/store/global";
import { supervisorEnabled } from "@/app/store/supervisor";

import { NumberControl, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";

export interface LimitSetting {
    /** The setting's key, and the row's id with its colons undone. */
    key: string;
    textKey: string;
    min: number;
    max: number;
    fallback: number;
    keywords: string[];
}

/** The limits, with srv's defaults and bounds. */
export const SUPERVISOR_LIMITS: LimitSetting[] = [
    { key: "supervisor:steady:continuations", textKey: "steadyContinuations", min: 1, max: 100, fallback: 5, keywords: ["steady", "continuations in a row"] },
    { key: "supervisor:steady:minutes", textKey: "steadyMinutes", min: 0, max: 1440, fallback: 0, keywords: ["steady", "time cap", "minutes"] },
    { key: "supervisor:drive:continuations", textKey: "driveContinuations", min: 1, max: 100, fallback: 15, keywords: ["drive", "continuations in a row"] },
    { key: "supervisor:drive:minutes", textKey: "driveMinutes", min: 0, max: 1440, fallback: 120, keywords: ["drive", "time cap", "minutes"] },
    { key: "supervisor:noprogress", textKey: "noprogress", min: 1, max: 10, fallback: 2, keywords: ["no progress", "no tool", "stuck"] },
    { key: "supervisor:cooldown", textKey: "cooldown", min: 0, max: 600, fallback: 30, keywords: ["cooldown", "wait", "delay", "seconds"] },
    { key: "supervisor:hourly", textKey: "hourly", min: 0, max: 1000, fallback: 30, keywords: ["fleet budget", "continuations per hour", "budget"] },
];

export interface NoticeSetting {
    key: string;
    textKey: string;
}

/** Which stops send an OS notice; the rows and the history show every stop whatever these say. */
export const SUPERVISOR_NOTICES: NoticeSetting[] = [
    { key: "supervisor:notify:needsyou", textKey: "notifyNeedsyou" },
    { key: "supervisor:notify:limit", textKey: "notifyLimit" },
    { key: "supervisor:notify:failed", textKey: "notifyFailed" },
    { key: "supervisor:notify:done", textKey: "notifyDone" },
];

/** A notice setting's value; every notice is on until turned off. */
export function noticeOn(settings: Record<string, unknown> | null | undefined, item: NoticeSetting): boolean {
    return settings?.[item.key] !== false;
}

/** A setting's value, or its default when unset. */
export function limitValue(settings: Record<string, unknown> | null | undefined, item: LimitSetting): number {
    const v = settings?.[item.key];
    return typeof v === "number" ? v : item.fallback;
}

const rowId = (key: string) => `supervisor.${key.slice("supervisor:".length).replaceAll(":", "_")}`;

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

const ENABLED = settingEntry({
    id: "supervisor.enabled",
    labelKey: "settings.supervisor.enabled.label",
    descriptionKey: "settings.supervisor.enabled.description",
    section: "supervisor",
    keywords: ["supervisor", "master switch", "autopilot", "keep going", "continue", "continuation", "steady", "drive", "swarm", "supervisor:enabled"],
});

const entryOf = (item: LimitSetting | NoticeSetting, group: "limits" | "notices", more: string[]): SettingsIndexEntry =>
    settingEntry({
        id: rowId(item.key),
        labelKey: `settings.supervisor.${item.textKey}.label`,
        descriptionKey: `settings.supervisor.${item.textKey}.description`,
        groupKey: `settings.supervisor.group.${group}`,
        section: "supervisor",
        keywords: ["supervisor", "swarm", item.key, ...more],
    });

const LIMIT_ENTRIES = SUPERVISOR_LIMITS.map((item) => entryOf(item, "limits", item.keywords));
const NOTICE_ENTRIES = SUPERVISOR_NOTICES.map((item) => entryOf(item, "notices", ["notify", "notification", "tray"]));

export const SUPERVISOR_SETTINGS: Record<string, SettingsIndexEntry> = Object.fromEntries(
    [ENABLED, ...LIMIT_ENTRIES, ...NOTICE_ENTRIES].map((entry) => [entry.id, entry])
);

export function SupervisorSection(): JSX.Element {
    const settings = () => (settingsAtom() ?? {}) as Record<string, unknown>;
    const entry = (item: LimitSetting | NoticeSetting) => SUPERVISOR_SETTINGS[rowId(item.key)];
    return (
        <div class="settings-section-body">
            {/* The master switch first, its explanation always visible: what it allows, and that each agent needs its own mode. */}
            <SettingRow
                id={ENABLED.id}
                settingKey="supervisor:enabled"
                label={ENABLED.label}
                description={ENABLED.description}
                control={<ToggleControl checked={supervisorEnabled(settings())} onChange={(v) => set("supervisor:enabled", v)} />}
            />
            <SectionHeader label={t("settings.supervisor.group.limits")} />
            <For each={SUPERVISOR_LIMITS}>
                {(item) => (
                    <SettingRow
                        id={rowId(item.key)}
                        settingKey={item.key}
                        label={entry(item).label}
                        description={entry(item).description}
                        control={
                            <NumberControl
                                class="setting-number"
                                min={item.min}
                                max={item.max}
                                step={1}
                                parse="int"
                                value={limitValue(settings(), item)}
                                onChange={(v) => set(item.key, v)}
                            />
                        }
                    />
                )}
            </For>
            <SectionHeader label={t("settings.supervisor.group.notices")} />
            <For each={SUPERVISOR_NOTICES}>
                {(item) => (
                    <SettingRow
                        id={rowId(item.key)}
                        settingKey={item.key}
                        label={entry(item).label}
                        description={entry(item).description}
                        control={<ToggleControl checked={noticeOn(settings(), item)} onChange={(v) => set(item.key, v)} />}
                    />
                )}
            </For>
            <p class="settings-section-note">{t("settings.supervisor.note")}</p>
        </div>
    );
}

// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Advanced → Unrecognized settings: the keys of settings.json that are not settings, each with the key it
// was probably meant to be when it is a few characters away from one, and a Remove button. srv lists them
// (`FullConfigType.unrecognized`); a key stays in the file, with no effect, until the person removes it here or in the
// editor: AgentMux never deletes one on its own (docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md §5.6).

import { For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { fullConfigAtom, settingsAtom } from "@/app/store/config-signals";
import { SectionHeader, set, SettingRow } from "../settings-controls";

const SHOWN_VALUE_CHARS = 40;

/** What the key holds, as one short line: a long value is cut. */
export function valueShown(value: unknown): string {
    const text = typeof value === "string" ? JSON.stringify(value) : (JSON.stringify(value) ?? String(value));
    return text.length > SHOWN_VALUE_CHARS ? `${text.slice(0, SHOWN_VALUE_CHARS)}…` : text;
}

/** The row's sentences: what the key is, what it holds, where it is, and what it may have been meant to be. */
export function unrecognizedDescription(u: UnrecognizedSetting, value: unknown): string {
    const parts = [t("settings.unrecognized.what"), t("settings.unrecognized.value", { value: valueShown(value) })];
    if (u.line != null) parts.push(t("settings.errors.line", { line: u.line }));
    if (u.suggestion) parts.push(t("settings.unrecognized.suggest", { key: u.suggestion }));
    return parts.join(" · ");
}

export function UnrecognizedGroup(): JSX.Element {
    const list = () => fullConfigAtom()?.unrecognized ?? [];
    return (
        <Show when={list().length > 0}>
            <SectionHeader label={t("settings.advanced.group.unrecognized")} />
            <For each={list()}>
                {(u) => (
                    <SettingRow
                        settingKey={u.key}
                        label={u.key}
                        description={unrecognizedDescription(u, (settingsAtom() as Record<string, unknown> | undefined)?.[u.key])}
                        control={
                            <Button onClick={() => set(u.key, null)} aria-label={t("settings.unrecognized.removeKey", { key: u.key })}>
                                {t("settings.unrecognized.remove")}
                            </Button>
                        }
                    />
                )}
            </For>
        </Show>
    );
}

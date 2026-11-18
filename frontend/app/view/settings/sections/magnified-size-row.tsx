// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The magnified pane's size, a share of the layout area it opens over (layout/lib/magnified-size.ts): the field takes
 * any number, writes it held to that range, and says so when it moved it, putting the corrected number back in the
 * box, as the Animations numbers do. A value from an old config shows as the layout reads it.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { clampMagnifiedSize, MAGNIFIED_SIZE_MAX, MAGNIFIED_SIZE_MIN } from "@/layout/lib/magnified-size";
import { NumberControl, set, SettingRow } from "../settings-controls";

const KEY = "window:magnifiedblocksize";

/** How far from 0 an entry can be and still reach the handler, which holds it to the range and says so. */
const MAX_ENTRY = 1_000_000;

export function MagnifiedSizeRow(props: {
    s: () => any;
    row: { id: string; label: string; description?: string };
}): JSX.Element {
    const [snapped, setSnapped] = createSignal<string | null>(null);
    // Bumped when an entry was corrected back to the stored number: the box then shows it again, in place.
    const [corrected, setCorrected] = createSignal(0);
    const edit = (raw: number) => {
        const next = clampMagnifiedSize(raw);
        const moved = next !== raw;
        setSnapped(
            moved
                ? t("settings.appearance.magnifiedSize.snapped", {
                      raw: String(raw),
                      next: String(next),
                      min: String(MAGNIFIED_SIZE_MIN),
                      max: String(MAGNIFIED_SIZE_MAX),
                  })
                : null
        );
        if (moved) setCorrected((n) => n + 1);
        set(KEY, next);
    };
    return (
        <>
            <SettingRow
                settingKey={KEY}
                id={props.row.id}
                label={props.row.label}
                description={props.row.description}
                control={
                    <NumberControl
                        class="setting-number setting-number--wide"
                        min={-MAX_ENTRY}
                        max={MAX_ENTRY}
                        step={0.05}
                        value={clampMagnifiedSize((props.s()["window:magnifiedblocksize"] as number) ?? 0.9)}
                        resyncKey={corrected()}
                        onChange={edit}
                    />
                }
            />
            <Show when={snapped()}>
                <p class="setting-snap" role="status" data-testid="magnified-size-snap">
                    {snapped()}
                </p>
            </Show>
        </>
    );
}

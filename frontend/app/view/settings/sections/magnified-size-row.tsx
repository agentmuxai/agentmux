// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The magnified pane's size, a share of the layout area it opens over (layout/lib/magnified-size.ts): the field takes
 * any number, writes it held to that range, and says so when it moved it, putting the corrected number back in the
 * box, as the Animations numbers do (when the field is left, if it is being typed in). A value from an old config shows as the layout reads it.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { t } from "@/app/i18n/i18n";
import { clampMagnifiedSize, MAGNIFIED_SIZE_MAX, MAGNIFIED_SIZE_MIN } from "@/layout/lib/magnified-size";
import { NumberControl, set, SettingRow } from "../settings-controls";

const KEY = "window:magnifiedblocksize";

/** How far from 0 an entry can be and still reach the handler, which holds it to the range and says so. */
const MAX_ENTRY = 1_000_000;

/** How long after an entry the row is brought into view once more: the Settings pane has taken its new size by then, ms. */
const REVEAL_AGAIN_MS = 250;

export function MagnifiedSizeRow(props: {
    s: () => any;
    row: { id: string; label: string; description?: string };
}): JSX.Element {
    const [snapped, setSnapped] = createSignal<string | null>(null);
    // Bumped when an entry was corrected back to the stored number: the box then shows it again, in place.
    const [corrected, setCorrected] = createSignal(0);
    // What the person typed, kept in the box while they are still in it (see showStored).
    const [held, setHeld] = createSignal<number | null>(null);
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
        if (moved) showStored(raw);
        else setHeld(null);
        set(KEY, next);
        reveal();
    };
    // A Settings pane that is itself magnified changes size with the entry (at 0.5 its list is a few rows high), which can
    // leave the row, and the message under it, below what is shown. So after an entry the row (or its message, which comes
    // under it) is scrolled into view, and again once the pane has taken its new size.
    let message: HTMLParagraphElement | undefined;
    let later: ReturnType<typeof setTimeout> | undefined;
    const scrollToRow = () => {
        const target = message?.isConnected ? message : document.getElementById(`setting-${props.row.id}`);
        target?.scrollIntoView?.({ block: "nearest" });
    };
    const reveal = () => {
        scrollToRow();
        clearTimeout(later);
        later = setTimeout(scrollToRow, REVEAL_AGAIN_MS);
    };
    onCleanup(() => clearTimeout(later));
    // The corrected number goes back in the box. Not while the person is still typing in it: a pause after
    // "0" would turn it into 0.5 under the caret, and the ".75" that follows would land after it (0.575).
    // The message already says what was used; the box shows it when the field is left.
    const showStored = (raw: number) => {
        const box = document.getElementById(`setting-${props.row.id}`)?.querySelector("input");
        if (box && box === document.activeElement) {
            setHeld(raw);
            box.addEventListener(
                "focusout",
                () => {
                    setHeld(null);
                    setCorrected((n) => n + 1);
                },
                { once: true }
            );
        } else {
            setCorrected((n) => n + 1);
        }
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
                        value={held() ?? clampMagnifiedSize((props.s()["window:magnifiedblocksize"] as number) ?? 0.9)}
                        resyncKey={corrected()}
                        onChange={edit}
                    />
                }
            />
            <Show when={snapped()}>
                <p class="setting-snap" role="status" data-testid="magnified-size-snap" ref={message}>
                    {snapped()}
                </p>
            </Show>
        </>
    );
}

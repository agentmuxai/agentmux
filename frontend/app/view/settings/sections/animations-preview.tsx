// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Animations page's preview (docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §4.3): three
// samples that play with the chosen settings, not the OS: a card that slides and fades in, a switch
// and a button press, and a panel that collapses. Replay runs them again.

import { Button, Switch } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { DURATION } from "@/app/motion/primitives";
import { enter } from "@/app/motion/primitives";
import { motionScale, scaledMs } from "@/app/motion/motion";
import { createSignal, onCleanup, Show, type JSX } from "solid-js";

export function AnimationsPreview(): JSX.Element {
    const [on, setOn] = createSignal(true);
    const [open, setOpen] = createSignal(true);
    let card: HTMLDivElement | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(timer));

    const replay = () => {
        if (card) enter(card, "slideY");
        setOn((v) => !v);
        setOpen(false);
        clearTimeout(timer);
        // Back open once the collapse has played, in the length it plays at.
        timer = setTimeout(() => setOpen(true), scaledMs(DURATION.slow) + 250);
    };

    return (
        <div class="animations-preview" data-testid="animations-preview">
            <Show when={motionScale() === 0}>
                <div class="animations-preview-off">{t("settings.animations.preview.off")}</div>
            </Show>
            <div class="animations-preview-samples">
                <div ref={card} class="animations-preview-card">
                    {t("settings.animations.preview.card")}
                </div>
                <Switch checked={on()} onChange={setOn} ariaLabel={t("settings.animations.preview.switch")} />
                <Button onClick={replay} icon="rotate-right">
                    {t("settings.animations.preview.replay")}
                </Button>
            </div>
            <div class="animations-preview-panel" classList={{ "animations-preview-panel--open": open() }}>
                <div class="animations-preview-panel-inner">{t("settings.animations.preview.panel")}</div>
            </div>
        </div>
    );
}

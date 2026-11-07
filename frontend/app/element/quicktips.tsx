// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { JSX, Show } from "solid-js";
import { AlphaNotice, EmptyNotice, GesturesCard, HeaderIconsCard, LinksCard, ShortcutsCard, TipsCard } from "./quicktips-parts";
import { createQuickTips } from "./quicktips-state";

/**
 * The Help pane's content. `filter`: what the user typed in the pane's filter
 * box; every word must appear in an item's title, text, keys or section name
 * (SPEC_HELP_PANE_FILTER_2026_10_08.md). Cards with nothing left are hidden,
 * and so is the alpha notice while filtering.
 */
const QuickTips = (props: { filter?: string }): JSX.Element => {
    const s = createQuickTips(props);

    return (
        <div class="flex flex-col w-full gap-6 @container">
            <Show when={s.filtering() && s.nothing()}>
                <EmptyNotice filter={props.filter} />
            </Show>

            <Show when={s.headerIcons().length > 0}>
                <HeaderIconsCard s={s} />
            </Show>

            <Show when={s.shortcutSections().length > 0}>
                <ShortcutsCard s={s} />
            </Show>

            <Show when={s.gestureSections().length > 0}>
                <GesturesCard s={s} />
            </Show>

            <Show when={s.tips().length > 0}>
                <TipsCard s={s} />
            </Show>

            <Show when={s.links().length > 0}>
                <LinksCard s={s} />
            </Show>

            <Show when={!s.filtering()}>
                <AlphaNotice />
            </Show>
        </div>
    );
};

export { QuickTips };

// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { shortcutFor, shortcutHelp, keyPlatform } from "@/app/keybindings";
import { createMemo } from "solid-js";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { keyLabelWords } from "@/app/keybindings/help";
import { gestureLabels, tipLabel, tipsFor, type TipArea } from "@/app/keybindings/tips";
import { getPlatform } from "@/util/platformutil";

/** The pane-header icons, each with the command its shortcut comes from. */
const HEADER_ICONS: { icon: string; label: string; command?: string }[] = [
    { icon: "fa-regular fa-window-maximize", label: "Maximize a pane", command: "pane:magnify" },
    { icon: "fa-laptop", label: "Change connection", command: "pane:changeConnection" },
    { icon: "fa-cog", label: "Pane Settings" },
    { icon: "fa-xmark", label: "Close pane", command: "pane:close" },
];

const MORE_TIPS: { icon: string; lead: string; text: string }[] = [
    { icon: "fa-cog", lead: "Web View", text: "Click the gear in the web view to set your homepage" },
    { icon: "fa-cog", lead: "Terminal", text: "Click the gear in the terminal to set your terminal theme and font size" },
];

const HELP_LINKS: { icon: string; label: string; href: string }[] = [
    { icon: "fa-brands fa-discord", label: "Join Our Discord", href: "https://discord.com/invite/96erama9Ar" },
    { icon: "fa-solid fa-sliders", label: "Configuration Options", href: "https://docs.agentmux.ai/config" },
    { icon: "fa-solid fa-keyboard", label: "All Keybindings", href: "https://docs.agentmux.ai/keybindings" },
    { icon: "fa-solid fa-book", label: "Full Documentation", href: "https://docs.agentmux.ai" },
    { icon: "fa-solid fa-bug", label: "Report Bugs & Issues", href: "https://github.com/agentmuxai/agentmux/issues/new" },
];

/** What the Help pane shows for the words typed in its filter: each card's items, and whether nothing is left. */
export function createQuickTips(props: { filter?: string }) {
    const query = () => props.filter ?? "";
    const filtering = () => query().trim() !== "";
    const matches = (...texts: string[]) => matchesEveryWord(query(), ...texts);

    const headerIcons = createMemo(() =>
        HEADER_ICONS.filter((it) => matches("Header Icons", it.label, keyLabelWords(it.command ? shortcutFor(it.command) : "")))
    );
    const shortcutSections = createMemo(() =>
        shortcutHelp()
            .map((section) => ({
                category: section.category,
                entries: section.entries.filter((e) => matches(section.category, e.label, ...e.keys.map(keyLabelWords))),
            }))
            .filter((section) => section.entries.length > 0)
    );
    const tips = createMemo(() => MORE_TIPS.filter((t) => matches("More Tips", t.lead, t.text)));
    // The hidden tips (keybindings/tips.ts), grouped by area in table order.
    const gestureSections = createMemo(() => {
        const platform = keyPlatform();
        const byArea = new Map<TipArea, { label: string; where?: string; keys: string[][] }[]>();
        for (const tip of tipsFor(getPlatform())) {
            const keys = gestureLabels(tip, platform);
            const label = tipLabel(tip, platform);
            if (!matches("Mouse and gestures", tip.area, label, tip.where ?? "", ...keys.flat().map(keyLabelWords))) continue;
            byArea.set(tip.area, [...(byArea.get(tip.area) ?? []), { label, where: tip.where, keys }]);
        }
        return [...byArea.entries()].map(([area, entries]) => ({ area, entries }));
    });
    const links = createMemo(() => HELP_LINKS.filter((l) => matches("Need More Help", l.label)));
    const nothing = () =>
        headerIcons().length === 0 &&
        shortcutSections().length === 0 &&
        gestureSections().length === 0 &&
        tips().length === 0 &&
        links().length === 0;

    return { filtering, headerIcons, shortcutSections, tips, gestureSections, links, nothing };
}

export type QuickTipsState = ReturnType<typeof createQuickTips>;

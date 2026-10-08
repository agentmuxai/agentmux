// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { shortcutFor, shortcutHelp } from "@/app/keybindings";
import { cn } from "@/util/util";
import { createMemo, For, JSX, Show } from "solid-js";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { keyLabelWords } from "@/app/keybindings/help";

const KeyCap = (props: { children?: JSX.Element }): JSX.Element => {
    return (
        <div class="inline-block px-2 py-1 mx-[1px] font-mono text-[0.85em] text-foreground bg-highlightbg rounded-[3px] border border-border whitespace-nowrap">
            {props.children}
        </div>
    );
};

const IconBox = (props: { children?: JSX.Element; variant?: "accent" | "secondary" }): JSX.Element => {
    const variant = props.variant ?? "accent";
    const colorClasses =
        variant === "secondary"
            ? "text-secondary bg-hover border-border [&_svg]:fill-secondary [&_svg_#arrow1]:fill-primary [&_svg_#arrow2]:fill-primary"
            : "text-accent-400 bg-accent-400/10 border-accent-400/20 [&_svg]:fill-accent-400 [&_svg_#arrow1]:fill-accent-400 [&_svg_#arrow2]:fill-accent-400";

    return (
        <div
            class={cn(
                "text-[20px] min-w-[32px] h-[32px] flex items-center justify-center rounded-md border [&_svg]:h-[16px]",
                colorClasses
            )}
        >
            {props.children}
        </div>
    );
};

/** One shortcut label ("Ctrl+Shift+T", "⇧⌘W") as a key cap. */
const Shortcut = (props: { label: string }): JSX.Element => (
    <Show when={props.label}>
        <div class="flex flex-row items-center">
            <KeyCap>{props.label}</KeyCap>
        </div>
    </Show>
);

const CARD =
    "flex flex-col gap-4 p-5 bg-gradient-to-br from-highlightbg/30 to-transparent hover:from-accent-400/5 rounded-lg border border-border hover:border-accent-400/20 transition-colors duration-300";

const CardTitle = (props: { children: JSX.Element }): JSX.Element => (
    <div class="flex items-center gap-2 text-xl font-bold">
        <div class="w-1 h-6 bg-accent-400 rounded-full" />
        <span class="text-foreground">{props.children}</span>
    </div>
);

/** The pane-header icons, each with the command its shortcut comes from. */
const HEADER_ICONS: { icon: string; label: string; command?: string }[] = [
    { icon: "fa-window-maximize", label: "Maximize a pane", command: "pane:magnify" },
    { icon: "fa-laptop", label: "Change connection", command: "pane:changeConnection" },
    { icon: "fa-cog", label: "Pane Settings" },
    { icon: "fa-xmark-large", label: "Close pane", command: "pane:close" },
];

const MOUSE_ENTRIES: { label: string; keys: string[] }[] = [{ label: "Resize a single border", keys: ["Shift + drag"] }];

const MORE_TIPS: { icon: string; lead: string; text: string }[] = [
    { icon: "fa-computer-mouse", lead: "Tabs", text: "Right click any tab to change backgrounds or rename." },
    { icon: "fa-cog", lead: "Web View", text: "Click the gear in the web view to set your homepage" },
    { icon: "fa-cog", lead: "Terminal", text: "Click the gear in the terminal to set your terminal theme and font size" },
];

const HELP_LINKS: { icon: string; label: string; href: string }[] = [
    { icon: "fa-brands fa-discord", label: "Join Our Discord", href: "https://discord.com/invite/96erama9Ar" },
    { icon: "fa-solid fa-sharp fa-sliders", label: "Configuration Options", href: "https://docs.agentmux.ai/config" },
    { icon: "fa-solid fa-sharp fa-keyboard", label: "All Keybindings", href: "https://docs.agentmux.ai/keybindings" },
    { icon: "fa-solid fa-sharp fa-book", label: "Full Documentation", href: "https://docs.agentmux.ai" },
    { icon: "fa-solid fa-sharp fa-bug", label: "Report Bugs & Issues", href: "https://github.com/agentmuxai/agentmux/issues/new" },
];

/**
 * The Help pane's content. `filter`: what the user typed in the pane's filter
 * box; every word must appear in an item's title, text, keys or section name
 * (SPEC_HELP_PANE_FILTER_2026_10_08.md). Cards with nothing left are hidden,
 * and so is the alpha notice while filtering.
 */
const QuickTips = (props: { filter?: string }): JSX.Element => {
    const query = () => props.filter ?? "";
    const filtering = () => query().trim() !== "";
    const matches = (...texts: string[]) => matchesEveryWord(query(), ...texts);

    const headerIcons = createMemo(() =>
        HEADER_ICONS.filter((it) => matches("Header Icons", it.label, keyLabelWords(it.command ? shortcutFor(it.command) : "")))
    );
    const shortcutSections = createMemo(() =>
        [...shortcutHelp(), { category: "Mouse", entries: MOUSE_ENTRIES }]
            .map((section) => ({
                category: section.category,
                entries: section.entries.filter((e) => matches(section.category, e.label, ...e.keys.map(keyLabelWords))),
            }))
            .filter((section) => section.entries.length > 0)
    );
    const tips = createMemo(() => MORE_TIPS.filter((t) => matches("More Tips", t.lead, t.text)));
    const links = createMemo(() => HELP_LINKS.filter((l) => matches("Need More Help", l.label)));
    const nothing = () =>
        headerIcons().length === 0 && shortcutSections().length === 0 && tips().length === 0 && links().length === 0;

    return (
        <div class="flex flex-col w-full gap-6 @container">
            <Show when={filtering() && nothing()}>
                <div class="quicktips-empty p-5 text-secondary">Nothing in Help matches “{props.filter?.trim()}”.</div>
            </Show>

            <Show when={headerIcons().length > 0}>
                <div class={CARD}>
                    <CardTitle>Header Icons</CardTitle>
                    <div class="grid grid-cols-[repeat(auto-fit,minmax(min(15rem,100%),1fr))] gap-3">
                        <For each={headerIcons()}>
                            {(it) => (
                                <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                                    <IconBox variant="secondary">
                                        <i class={`fa-solid fa-sharp ${it.icon} fa-fw`} />
                                    </IconBox>
                                    <Show when={it.command} fallback={<span class="text-[15px]">{it.label}</span>}>
                                        <div class="flex flex-col gap-0.5 flex-1">
                                            <span class="text-[15px]">{it.label}</span>
                                            <Shortcut label={shortcutFor(it.command!)} />
                                        </div>
                                    </Show>
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            </Show>

            <Show when={shortcutSections().length > 0}>
                <div class={CARD}>
                    <CardTitle>Keyboard Shortcuts</CardTitle>

                    {/* Generated from the shortcut table (frontend/app/keybindings/defaults.ts),
                        so this list can't disagree with what the keys do. Columns, not a grid:
                        the browser fits as many 17rem columns as the pane is wide and balances
                        the sections between them, so a short section is followed straight away
                        by the next one instead of leaving a gap beside a long one. */}
                    <div class="columns-[17rem] gap-x-5">
                        <For each={shortcutSections()}>
                            {(section) => (
                                <div class="flex flex-col gap-1.5 mb-6 break-inside-avoid">
                                    <div class="text-sm text-accent-400 font-semibold uppercase tracking-wide mb-1">
                                        {section.category}
                                    </div>
                                    <For each={section.entries}>
                                        {(entry) => (
                                            <div class="flex flex-col gap-0.5 p-2 rounded-md hover:bg-hover transition-colors">
                                                <span class="text-[15px]">{entry.label}</span>
                                                <div class="flex flex-row flex-wrap items-center gap-1">
                                                    <For each={entry.keys}>{(k) => <KeyCap>{k}</KeyCap>}</For>
                                                </div>
                                            </div>
                                        )}
                                    </For>
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            </Show>

            <Show when={tips().length > 0}>
                <div class={CARD}>
                    <CardTitle>More Tips</CardTitle>
                    <div class="flex flex-col gap-2">
                        <For each={tips()}>
                            {(tip) => (
                                <div class="flex items-center gap-3 p-2 rounded-md hover:bg-hover transition-colors">
                                    <IconBox variant="secondary">
                                        <i class={`fa-solid fa-sharp ${tip.icon} fa-fw`} />
                                    </IconBox>
                                    <span>
                                        <b>{tip.lead}</b> - {tip.text}
                                    </span>
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            </Show>

            <Show when={links().length > 0}>
                <div class={CARD}>
                    <CardTitle>Need More Help?</CardTitle>
                    <div class="grid grid-cols-[repeat(auto-fit,minmax(min(15rem,100%),1fr))] gap-2">
                        <For each={links()}>
                            {(link) => (
                                <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors">
                                    <IconBox variant="secondary">
                                        <i class={`${link.icon} fa-fw`} />
                                    </IconBox>
                                    <a
                                        target="_blank"
                                        href={link.href}
                                        rel="noopener"
                                        class="hover:text-accent-400 hover:underline transition-colors font-medium"
                                    >
                                        {link.label}
                                    </a>
                                </div>
                            )}
                        </For>
                    </div>
                </div>
            </Show>

            <Show when={!filtering()}>
                <div class="flex flex-col gap-4 p-5 bg-gradient-to-br from-amber-900/20 to-transparent rounded-lg border border-amber-500/20">
                    <div class="flex items-center gap-2 text-xl font-bold">
                        <div class="w-1 h-6 bg-amber-400 rounded-full" />
                        <span class="text-foreground">Alpha Software &amp; AI Content</span>
                    </div>
                    <div class="flex flex-col gap-2 text-secondary text-sm">
                        <p>
                            AgentMux is <b class="text-amber-300">alpha software</b>. Features may be incomplete, unstable, or change between releases.
                            AI agents generate code, text, and other outputs that may be <b>inaccurate, incomplete, or inappropriate</b>.
                            Always review agent outputs before using them.
                        </p>
                        <p>
                            To report bugs, request features, or flag issues with AI-generated content:
                        </p>
                        <div class="flex items-center gap-3 p-3 rounded-md bg-panel hover:bg-highlightbg transition-colors mt-1">
                            <IconBox variant="secondary">
                                <i class="fa-brands fa-github fa-fw" />
                            </IconBox>
                            <a
                                target="_blank"
                                href="https://github.com/agentmuxai/agentmux/issues/new"
                                rel="noopener"
                                class="hover:text-accent-400 hover:underline transition-colors font-medium"
                            >
                                Open an Issue on GitHub
                            </a>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export { QuickTips };
